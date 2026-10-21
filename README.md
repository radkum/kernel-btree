# kernel-btree

Ordered map and set for `no_std` that **never panic or abort on allocation failure**.
Written for a Windows kernel driver, where a panic is a bugcheck (BSOD).

- `try_insert` is **atomic**: on `Err(AllocError)` the map is exactly as before the call.
- `remove`, `pop_first`/`pop_last`, `retain`, `get`, iteration, `clear` and `Drop`
  **never allocate**, so cleanup paths cannot fail.
- No panics in library code (`#![deny(clippy::panic, unwrap_used, expect_used, indexing_slicing)]`);
  a panic can only come from your `Ord`/`Drop`. A broken `Ord` gives wrong results, never UB.
- Builds with `alloc` compiled under `--cfg no_global_oom_handling`, so infallible allocation
  cannot sneak in.
- No recursion (small kernel stacks).

## Example (kernel driver)

```rust
#![no_std]
extern crate alloc;

use kernel_btree::{AllocError, BTreeMap};

// The global allocator is the driver's pool allocator (ExAllocatePool2 + pool tag).
// Allocation failure is an ordinary error here, not a bugcheck.
struct Sessions {
    by_pid: BTreeMap<u32, u64>,
}

impl Sessions {
    fn on_process_create(&mut self, pid: u32, tag: u64) -> Result<(), AllocError> {
        self.by_pid.try_insert(pid, tag)?; // on Err the table is unchanged
        Ok(())
    }

    fn on_process_exit(&mut self, pid: u32) {
        self.by_pid.remove(&pid); // never allocates, always succeeds
    }
}
```

## Custom allocator

Each map can take its own node allocator, e.g. a separate pool for code running at
`DISPATCH_LEVEL`. `RawAlloc` works on stable and has no dependencies:

```rust
use core::alloc::Layout;
use kernel_btree::{BTreeMap, RawAlloc};

struct NonPaged;

unsafe impl RawAlloc for NonPaged {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        // pool blocks are 16-byte aligned on x64
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        unsafe { ExAllocatePool2(POOL_FLAG_NON_PAGED, layout.size() as u64, TAG) }.cast()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { ExFreePoolWithTag(ptr.cast(), TAG) }
    }
}

let mut flows: BTreeMap<u64, u32, NonPaged> = BTreeMap::new_in(NonPaged);
```

`BTreeMap::new()` uses `Global` (the `#[global_allocator]`); `&A` is a `RawAlloc` too, so
several maps can share one allocator.

## API

`BTreeMap`: `new`, `new_in`, `len`, `is_empty`, `get`, `get_mut`, `contains_key` (all with `Borrow<Q>`),
`try_insert`, `try_insert_returning` (hands the pair back on failure), `remove`, `clear`,
`first_key_value`, `last_key_value`, `pop_first`, `pop_last`, `retain`, `range`, `range_mut`,
`entry` (`VacantEntry::try_insert`, `Entry::or_try_insert[_with]`), `iter`, `iter_mut`,
`keys`, `values`, `values_mut`, `into_iter`, `try_clone`.

`BTreeSet`: `new`, `new_in`, `len`, `is_empty`, `contains`, `try_insert`, `remove`, `first`, `last`,
`pop_first`, `pop_last`, `retain`, `range`, `iter`, `into_iter`, `try_clone`, `clear`.

Deliberately missing: `Index`, `Clone`, `Extend`, `FromIterator`, `insert` — none of them can
report allocation failure.

Differences from std: `range` with `start > end` yields nothing instead of panicking.

## Why not `fallible_collections`?

`fallible_collections` 0.5.2 has a `btree` module, but (as of 2026-10):

- it sits behind the `unstable` feature, which needs long-gone nightly features and does not
  compile on current nightly;
- its `VacantEntry::try_insert` increments `len` first, then splits the leaf, then allocates
  the parent. If that last allocation fails, the leaf is already split, the separator and the
  right half are dropped on the floor — **existing entries are lost**, `len` is wrong, and the
  caller gets `Err`;
- `Clone`, `Extend`, `split_off` and `append` call `.expect("Out of Mem")`.

kernel-btree allocates every node an insert will need **before** touching the tree
(counting the full nodes on the path), so the mutation phase cannot fail. This is checked by a
fault-injection test that fails every allocation inside each insert in turn and compares the
map with its previous state.

## Testing

```powershell
cargo test                 # unit + differential (proptest vs std) + fault injection
scripts\ci.ps1             # the above + clippy + no_global_oom_handling build + Miri
cargo run --release --example bench
```

On 1 M random `u64` keys the timings are on par with `std::collections::BTreeMap`.

## Status

Not on crates.io yet.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
