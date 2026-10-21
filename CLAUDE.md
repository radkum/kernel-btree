# kernel-btree — kontekst dla Claude

Ordered map (`BTreeMap`) dla `no_std`, która **nigdy** nie robi panic/abort przy braku pamięci.
Powstała dla projektu Shem (`C:\VSExclude\shem`): sterownik kernelowy Windows `ramon-km`, gdzie
panic = BSOD (`KeBugCheckEx`). Użytkownik pisze po polsku.

Przeczytaj najpierw: `docs/DESIGN.md` (dlaczego i jak), `PLAN.md` (co dalej, status).

## Twarde wymagania (nie łamać)

1. **Żadnego panic na OOM.** Każda alokacja jest fallible i zwraca `Err(AllocError)`.
2. **`try_insert` jest atomowe:** po `Err` mapa jest dokładnie taka jak przed wywołaniem.
3. **`remove`, `get`, iteracja, `clear`, `Drop` nie alokują.**
4. **Biblioteka kompiluje się z `alloc` zbudowanym pod `--cfg no_global_oom_handling`**
   (`scripts/check-no-oom.ps1`). Infallible API (`push`, `Vec::insert`, `clone`, `vec!`,
   `format!`, `Box::new`, `extend`) się wtedy nie kompilują — i dobrze.
5. **Bez panic w ogóle w ścieżkach biblioteki:** `#![deny(clippy::panic, unwrap_used,
   expect_used, indexing_slicing)]`. Nie implementujemy `Index`, `Clone`, `Extend`,
   `FromIterator` (te nie mają jak zgłosić błędu). Zamiast tego `try_clone`, `try_extend` itp.
6. **Kernel ma mały stos** (~24 KB na x64): żadnej rekurencji, iteracyjne algorytmy,
   małe ramki stosu.
7. **Nieprawidłowe `Ord` (niespójne porównania) nie może dawać UB** — co najwyżej błędne
   wyniki, jak w std.

## Komendy

```powershell
$env:HTTPS_PROXY=$null; $env:HTTP_PROXY=$null   # cargo nie przejdzie przez mitmproxy sesji
cargo test                                       # unit + differential (proptest) + fault injection
cargo clippy --lib
scripts\check-no-oom.ps1                         # build pod no_global_oom_handling (nightly, -Zbuild-std)
cargo +nightly miri test                         # gdy pojawi się unsafe w B-drzewie (miri do doinstalowania)
```

## Testy

- `tests/differential.rs` — proptest: losowe sekwencje operacji vs `std::collections::BTreeMap`.
- `tests/fault_injection.rs` + `tests/common/mod.rs` — globalny allocator, który per wątek
  odmawia po N alokacjach. Każda alokacja w `try_insert` jest po kolei psuta; po `Err` mapa musi
  być niezmieniona. Sprawdza też, że `remove` i podmiana wartości alokują 0 razy.
- `tests/edge_cases.rs` — wyczerpująco: usunięcie każdego klucza i wstawienie w każdą lukę dla
  rozmiarów 0..160 w 4 kształtach drzewa (każda gałąź split/steal/merge), wszystkie kombinacje
  granic `range`, spotkanie iteratorów w każdym punkcie, puste/1-elementowe, ZST, `Borrow`,
  `Send`/`Sync`, kowariancja (kompilacja).
- `tests/panic_safety.rs` — panic w `Ord`/`Clone`/`Drop`/`retain`, licznik dropów, niespójne
  `Ord`; allocator `Tracker` wykrywa double free bez Miri.
- Nowe operacje alokujące muszą dostać test w `fault_injection.rs`.
- Pokrycie: `cargo llvm-cov --all-features --summary-only` (2026-10-20: 99,5% linii; niepokryte
  tylko `Pos::clone`, wymagane przez `Copy`).

## Stan

MVP gotowe: B-drzewo (`src/node.rs` — układ węzłów i operacje lokalne, `src/lib.rs` — mapa).
Walidator `BTreeMap::check_invariants` za feature `debug-invariants` (włączany w testach przez
self dev-dependency w `Cargo.toml`). Iteratory: `src/iter.rs`, entry: `src/entry.rs`,
`BTreeSet`: `src/set.rs`. Pełne sprawdzenie: `scripts\ci.ps1`. Allocator per mapa: trait `RawAlloc` (`new_in`),
domyślnie `Global`.

## Zasady repo

- Licencja `MIT OR Apache-2.0`. Repo publiczne: https://github.com/radkum/kernel-btree (gałąź `main`).
- Docelowo crates.io; do tego czasu `publish = false` w `Cargo.toml`.
