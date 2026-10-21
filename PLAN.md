# Plan

Stan na 2026-10-20. Pełne `scripts\ci.ps1` zielone (Miri ~2 h, z czego fault injection ~85 min).

## Gotowe

- Szkielet crate'a: API `BTreeMap` (`new`, `len`, `is_empty`, `get`, `get_mut`,
  `contains_key`, `try_insert`, `remove`, `clear`, `first_key_value`, `last_key_value`, `iter`),
  `AllocError`.
- Tymczasowy backend: posortowany `Vec` (jak Ariadne w Shem), żeby API i testy działały.
- Testy: unit, differential proptest vs std, fault injection (atomowość `try_insert`,
  0 alokacji w `remove` i podmianie). Wszystkie przechodzą.
- `scripts/check-no-oom.ps1`: build pod `no_global_oom_handling` przechodzi; kanarek z `push`
  zgodnie z oczekiwaniem się nie kompiluje.

## MVP — zrobione (2026-10-19)

1. Węzły `LeafNode`/`InternalNode` (`B = 6`), `parent` + `parent_idx`, `alloc` z obsługą `null`
   (`src/node.rs`).
2. Walidator `check_invariants` (feature `debug-invariants`), wołany w differential po każdej
   operacji i w fault injection po każdej próbie.
3. `get`/`get_mut`/`contains_key` — zejście iteracyjne.
4. `try_insert`: rezerwacja węzłów przed mutacją; zarezerwowane węzły wewnętrzne trzymane w
   łańcuchu przez pole `parent` (bez tablicy `MAX_HEIGHT` na stosie).
5. `remove` z kradzieżą/scalaniem, bez alokacji; pusty korzeń jest zwalniany.
6. `Iter` (przód/tył), `Drop` i `clear` iteracyjnie.
7. Backend `Vec` usunięty; testy przechodzą bez zmian (dodane tylko wołania walidatora).
8. Miri: unit + differential czyste (`PROPTEST_CASES=3`); `try_insert_is_atomic` pod Miri
   trwa bardzo długo (2000 insertów × walidator O(n)).

Znane różnice vs std: `Drop` bez `#[may_dangle]` (ostrzejszy dropck); panic w `Drop` klucza/
wartości w trakcie `drop` mapy przecieka resztę węzłów (bez double free).

## Później — zrobione (2026-10-19)

- `range`/`range_mut` (odwrócony zakres = pusty, std panikuje), `entry`
  (`VacantEntry::try_insert`, `or_try_insert[_with]`, `and_modify`), `retain`,
  `pop_first`/`pop_last`, `iter_mut`, `keys`/`values`/`values_mut`, `into_iter`, `try_clone`,
  `BTreeSet` (`src/set.rs`). Iteratory na wspólnym kursorze par (`src/iter.rs`).
- `Borrow<Q>` w `get`/`get_mut`/`contains_key`/`remove`/`range`.
- `try_insert_returning` → `InsertError { key, value }`.
- `examples/bench.rs`: 1 M losowych `u64` — insert/get/iter/remove na poziomie std.
- `README.md` z przykładem kernelowym i opisem błędu w `fallible_collections`.
- `scripts/ci.ps1`: test, clippy `-D warnings`, no-oom, Miri (bez dwóch długich testów fault
  injection). Miri na targecie `x86_64-unknown-linux-gnu`: na Windows `System` pod
  `#[global_allocator]` z testu czyta nagłówek spoza `Box` (nadwyrównane alokacje libtestu) i
  Stacked Borrows to zgłasza — to nie jest błąd biblioteki. Workflow pod konkretny hosting — po założeniu repo.
- `IntoIter` = `pop_first`/`pop_last` (O(log n) na element, bez alokacji); `try_clone` =
  kolejne `try_insert` (O(n log n)). Do przyspieszenia, jeśli kiedyś będzie potrzeba.

Testy brzegowe (2026-10-20): `tests/edge_cases.rs` (29), `tests/panic_safety.rs` (8), +2 w
fault injection (klucze sekwencyjne = najgorszy przypadek rezerwacji; drop pary dokładnie raz).
Wyłapały i naprawione: `BTreeMap`/`Iter` były niezmiennicze po `K`/`V` (std: kowariantne) —
`parent`/`root`/`Pos::node` są teraz `*const`; `Default` generyczny po `A` psuł inferencję
`BTreeMap::default()` — teraz tylko dla `Global`, jak std.

## Do zrobienia

- Ewentualnie feature `windows-kernel`: gotowy `RawAlloc` na `ExAllocatePool2` (pool tag, typ
  puli). Szkic jest w README; na hoście nie da się go zlinkować, więc test tylko na VM.

Parametr allocatora — zrobione (2026-10-20): własny trait `RawAlloc` (stable, bez zależności),
`BTreeMap<K, V, A: RawAlloc = Global>`, `new_in`, to samo w `BTreeSet`. Odrzucone:
`core::alloc::Allocator` (tylko nightly) i `allocator-api2` 0.2.21 (wariant stable nie
kompiluje się pod `no_global_oom_handling` — `boxed.rs` używa `Vec`; wariant `nightly` to
re-eksport `core`). Test `custom_allocator_is_atomic_and_leak_free`: węzły tylko z puli,
atomowość przy psuciu alokacji z puli, 0 żywych bloków na końcu (pierwszy test na wycieki).

## Integracja z Shem

- Ariadne (`shem/ariadne`) używa `BTreeMap<u32, Node>` zamiast posortowanego `Vec` (2026-10-20).
- Zależność ścieżką (`path = "../../kernel-btree"`); docelowo git.

## Otwarte decyzje

- **Licencja ustalona (2026-10-21):** `MIT OR Apache-2.0`. Repo publiczne: https://github.com/radkum/kernel-btree.
  Docelowo crates.io; do tego czasu `publish = false`.
