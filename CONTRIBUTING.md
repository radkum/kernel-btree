# Contributing

Run the full check before sending a change:

```powershell
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
scripts\check-no-oom.ps1
```

The same commands run in CI; `scripts\ci.ps1` adds Miri.

Guidelines:

- Every allocating library operation must be fallible and return `AllocError`. No `panic`,
  `unwrap`, `expect` or indexing in library code.
- `try_insert` must stay atomic: add a case to `tests/fault_injection.rs` for any new
  allocating operation.
- Any `unsafe` block or `unsafe impl` needs a `// SAFETY:` comment.
- Keep algorithms iterative; kernel stacks are small.
- Commits should be focused, and user-visible changes belong in `CHANGELOG.md`.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this crate by you shall be dual licensed under MIT OR Apache-2.0, without any additional
terms or conditions.
