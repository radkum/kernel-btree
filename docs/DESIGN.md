# kernel-btree — projekt

Stan na 2026-10-19.

## Problem

W sterowniku Windows (Shem `ramon-km`) każdy panic kończy się `KeBugCheckEx`, czyli BSOD.
Standardowe kolekcje `alloc` przy braku pamięci wołają `handle_alloc_error`, co też jest
abortem. W Shem `alloc` jest budowany z `--cfg no_global_oom_handling`, więc infallible API
się nie kompilują, ale zostają tylko `Vec` z `try_reserve` i `Box::try_new`. Brak
uporządkowanej mapy.

Potrzebna mapa po kluczu (Ariadne: tablica procesów `pid → tag sesji`; później MOTA), w której:
- brak pamięci zwraca błąd i nie psuje struktury,
- usuwanie nigdy nie alokuje (sprzątanie po wyjściu procesu musi się zawsze udać),
- da się to przetestować na hoście.

## Przegląd istniejących rozwiązań (2026-10-19)

| Kandydat | Dlaczego nie |
|---|---|
| `std::collections::BTreeMap` | brak `try_insert`; pod `no_global_oom_handling` `insert` w ogóle nie istnieje |
| `hashbrown` 0.15.5 | nie kompiluje się pod `no_global_oom_handling` (importuje `handle_alloc_error`); to mapa haszująca, bez porządku |
| `fallible_collections` 0.5.2 (`btree`) | patrz niżej — nie kompiluje się i `try_insert` nie jest atomowe |
| `scapegoat` 2.3.0 | stała pojemność (bez heapa); „fallible” = brak miejsca w buforze; nieutrzymywany od 2022 |
| `allocated-btree` 0.1.3 | jawny allocator, ale niszowy (122 pobrania), bez gwarancji OOM-atomowości |
| `intrusive-collections` RBTree | węzły alokuje użytkownik — działa, ale zbędna komplikacja i `unsafe` po stronie wywołującego |
| `RtlInitializeGenericTableAvl` | tylko Windows kernel, callbacki C, nie do testowania na hoście |
| posortowany `Vec` (dziś w Ariadne) | O(n) insert/remove; wystarcza dla setek procesów, nie dla MOTA |

### `fallible_collections` — szczegóły

- Moduł `btree` jest za feature `unstable`, który włącza m.in. `ptr_internals`,
  `core_intrinsics`, `maybe_uninit_uninit_array` — na nightly 1.97 nie kompiluje się.
  To kopia starego `std::BTreeMap`.
- `VacantEntry::try_insert` (`src/btree/map.rs`): najpierw `*self.length += 1`, potem
  `handle.insert(...)?` dzieli liść, a potem w pętli `parent.insert(...)?` / `push_level()?`
  alokuje rodzica. OOM na tym etapie: liść jest już rozcięty, separator i prawa połówka są
  porzucone → **giną istniejące wpisy**, `len` jest zawyżony, a wywołujący dostaje `Err`.
- `Clone`, `Extend`, `split_off`, `append` mają `.expect("Out of Mem")`.

To jest główne uzasadnienie istnienia tego crate'a (dobre do README).

## Gwarancje

1. `try_insert(k, v) -> Result<Option<V>, AllocError>`: atomowe. Podmiana istniejącego klucza
   nie alokuje. Przy `Err` `k` i `v` są dropowane (wariant zwracający je z błędem do rozważenia).
2. `remove`, `get`, `get_mut`, iteracja, `clear`, `Drop`: zero alokacji.
3. Zero panic w kodzie biblioteki (lint `deny`). Panic może pochodzić tylko z `Ord`/`Drop`
   użytkownika.
4. Kompilacja pod `no_global_oom_handling` sprawdzana skryptem.

## Algorytm — B-drzewo

Jak w std: węzeł ma do `CAPACITY = 2*B - 1` kluczy (`B = 6` → 11), liście i węzły wewnętrzne
to osobne typy (wewnętrzny = liść + `CAPACITY + 1` krawędzi).

### Atomowy insert: najpierw rezerwacja, potem mutacja

1. Zejście od korzenia do liścia, zapamiętanie ścieżki (tablica na stosie o rozmiarze
   `MAX_HEIGHT`; wysokość ≤ ~30 dla `usize` kluczy, więc stała wystarcza).
2. Jeśli klucz istnieje → podmiana wartości, koniec (bez alokacji).
3. Policz, ile węzłów się podzieli: liczba kolejnych **pełnych** węzłów od liścia w górę
   (`s`). Jeśli wszystkie na ścieżce są pełne, dochodzi nowy korzeń.
   Potrzebne: `s` nowych węzłów (1 liść + `s-1` wewnętrznych) + ewentualnie 1 korzeń.
   Pusta mapa: 1 liść.
4. Zaalokuj wszystkie z góry do lokalnej tablicy. Jeśli którakolwiek alokacja się nie uda:
   zwolnij już zaalokowane, zwróć `Err`. **Drzewo nie zostało dotknięte.**
5. Wstaw i podziel węzły, biorąc nowe węzły z rezerwy. Ta faza nie może się nie udać.

Pesymistycznie wystarczy zarezerwować `wysokość + 1`, ale liczenie pełnych węzłów daje zwykle
0 alokacji (liść ma miejsce), więc insert jest tak tani jak w std.

### Remove bez alokacji

Usunięcie z liścia; przy niedopełnieniu kradzież od rodzeństwa albo scalanie (scalanie
**zwalnia** węzeł). Usunięcie z węzła wewnętrznego: zamiana z poprzednikiem z liścia.
Gdy korzeń jest pusty i ma jedno dziecko, wysokość maleje. Żadnej alokacji.

### Inne

- Bez rekurencji: zejście/wynurzenie po zapamiętanej ścieżce albo wskaźnikach do rodzica
  (std trzyma `parent` + `parent_idx` w węźle — upraszcza iteratory).
- `Drop`: iteracyjne zwalnianie (bez rekurencji po wysokości).
- Iterator: dwa „uchwyty” (przód/tył) jak w std, `DoubleEndedIterator`, `ExactSizeIterator`.

## Allocator

- Etap 1: globalny allocator (`alloc::alloc::alloc` → `null` = `AllocError`). W Shem globalny
  allocator to `kernel-alloc` (`ExAllocatePool2` z pool tagiem), więc działa od razu.
- Etap 2 (zrobione 2026-10-20): `BTreeMap<K, V, A: RawAlloc = Global>`, `new_in(alloc)`.
  - `RawAlloc` to własny trait: `alloc(Layout) -> *mut u8` (null = błąd) i
    `unsafe fn dealloc`. Działa na stable, bez zależności; `&A` też jest `RawAlloc`.
  - Odrzucone: `core::alloc::Allocator` (nightly, `allocator_api`) i `allocator-api2` 0.2.21
    (wariant stable nie kompiluje się pod `no_global_oom_handling`, wariant nightly to
    re-eksport `core`).
  - Cel: osobne mapy w NonPagedPool (kod na DISPATCH_LEVEL, np. WFP) i PagedPool.
- Ewentualny feature `windows-kernel` z gotowym allocatorem (pool tag, typ puli) — to
  uzasadnia nazwę „kernel-btree”.

## Testowanie

- Differential proptest vs `std::BTreeMap` (insert/remove/get/clear/iter/rev/first/last;
  później range, entry, retain).
- Fault injection: każda N-ta alokacja w operacji zawodzi; po `Err` porównanie ze stanem
  sprzed operacji. Ten test złapałby błąd z `fallible_collections`.
- Liczenie alokacji: `remove`/podmiana/iteracja = 0.
- Miri na testach (UB w `unsafe` węzłów).
- Walidator inwariantów (debug/test): posortowanie, zapełnienie węzłów ≥ B-1 poza korzeniem,
  równa głębokość liści, poprawne `parent_idx`, `len` zgodny z liczbą kluczy.
- Build pod `no_global_oom_handling` (`scripts/check-no-oom.ps1`).
- Opcjonalnie `cargo fuzz`.
- Integracja: Ariadne w Shem + `lab/test-baseline.ps1` na VM `Shem-Lab`.

## Nazwa

`kernel-btree` (wolna na crates.io 2026-10-19). Odrzucone `fallible-btree`: myli się z
`fallible_collections`. Słowa „fallible”, „no-std”, „allocation” są w `keywords`.
