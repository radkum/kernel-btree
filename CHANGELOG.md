# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-10-28

### Added

- `BTreeMap` and `BTreeSet` for `no_std`, with no dependencies.
- Edition 2024; MSRV 1.85.
- Fallible, atomic `try_insert` / `try_insert_returning`; on `Err` the map is unchanged.
- Allocation-free `remove`, `pop_first`, `pop_last`, `retain`, `clear`, iteration and `Drop`.
- `BTreeMap::try_clone`, `Entry` API with fallible insertion, ranges and iterators.
- Per-map node allocator via the `RawAlloc` trait and `new_in`.
- `AllocError` and `InsertError`, both implementing `core::error::Error`.
- Builds with `alloc` compiled under `--cfg no_global_oom_handling`.
- Tests: differential against `std::collections::BTreeMap`, fault injection, panic safety,
  edge cases and a benchmark example.

[0.1.0]: https://github.com/radkum/kernel-btree/releases/tag/v0.1.0
