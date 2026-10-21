//! Global allocator that fails on demand, per thread.
//!
//! `fail_after(n)`: the next `n` allocations on this thread succeed, then every one fails
//! until `disarm()`. `allocations()` counts allocations since the last arm/disarm.
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub struct FailingAlloc;

thread_local! {
    // usize::MAX = disarmed
    static BUDGET: Cell<usize> = const { Cell::new(usize::MAX) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for FailingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !take_one() {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !take_one() {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !take_one() {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn take_one() -> bool {
    // try_with: TLS may already be gone while the thread is shutting down
    let budget = BUDGET.try_with(|b| b.get()).unwrap_or(usize::MAX);
    let _ = COUNT.try_with(|c| c.set(c.get() + 1));
    if budget == usize::MAX {
        return true;
    }
    if budget == 0 {
        return false;
    }
    let _ = BUDGET.try_with(|b| b.set(budget - 1));
    true
}

pub fn fail_after(n: usize) {
    COUNT.with(|c| c.set(0));
    BUDGET.with(|b| b.set(n));
}

pub fn disarm() {
    BUDGET.with(|b| b.set(usize::MAX));
}

pub fn allocations() -> usize {
    COUNT.with(|c| c.get())
}
