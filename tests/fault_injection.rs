//! Every allocation inside `try_insert` is made to fail in turn; after each `Err` the map
//! must be exactly as before (atomicity), after `Ok` it must contain the new entry.

mod common;

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::collections::BTreeMap as StdMap;

use common::FailingAlloc;
use kernel_btree::{AllocError, BTreeMap, RawAlloc};

#[global_allocator]
static ALLOC: FailingAlloc = FailingAlloc;

fn assert_same<A: RawAlloc>(map: &BTreeMap<u32, u32, A>, expected: &StdMap<u32, u32>) {
    map.check_invariants();
    assert_eq!(map.len(), expected.len());
    assert!(map.iter().eq(expected.iter()));
}

fn insert_under_fault_injection(map: &mut BTreeMap<u32, u32>, expected: &mut StdMap<u32, u32>, key: u32) {
    for budget in 0.. {
        common::fail_after(budget);
        let result = map.try_insert(key, key * 10);
        common::disarm();
        match result {
            Err(AllocError) => assert_same(map, expected),
            Ok(old) => {
                assert_eq!(old, expected.insert(key, key * 10));
                assert_same(map, expected);
                return;
            }
        }
        assert!(budget < 64, "try_insert keeps failing with budget {budget}");
    }
}

#[test]
fn try_insert_is_atomic() {
    let mut map = BTreeMap::new();
    let mut expected = StdMap::new();
    // enough keys for several node splits once the B-tree backend lands
    for i in 0..2000u32 {
        let key = i.wrapping_mul(2_654_435_761) % 5000;
        insert_under_fault_injection(&mut map, &mut expected, key);
    }
}

#[test]
fn remove_does_not_allocate() {
    let mut map = BTreeMap::new();
    for i in 0..2000u32 {
        map.try_insert(i, i).unwrap();
    }
    common::fail_after(0);
    for i in (0..2000u32).step_by(3) {
        assert_eq!(map.remove(&i), Some(i));
    }
    let allocations = common::allocations();
    common::disarm();
    assert_eq!(allocations, 0);
}

#[test]
fn replace_does_not_allocate() {
    let mut map = BTreeMap::new();
    for i in 0..100u32 {
        map.try_insert(i, i).unwrap();
    }
    common::fail_after(0);
    let result = map.try_insert(50, 0);
    let allocations = common::allocations();
    common::disarm();
    assert_eq!(result, Ok(Some(50)));
    assert_eq!(allocations, 0);
}

#[test]
fn vacant_entry_try_insert_is_atomic() {
    let mut map = BTreeMap::new();
    let mut expected = StdMap::new();
    for i in 0..1000u32 {
        let key = i.wrapping_mul(2_654_435_761) % 3000;
        for budget in 0.. {
            common::fail_after(budget);
            let result = map.entry(key).or_try_insert(key * 10).map(|v| *v);
            common::disarm();
            match result {
                Err(AllocError) => assert_same(&map, &expected),
                Ok(v) => {
                    assert_eq!(v, *expected.entry(key).or_insert(key * 10));
                    assert_same(&map, &expected);
                    break;
                }
            }
            assert!(budget < 64);
        }
    }
}

#[test]
fn try_insert_returning_hands_pair_back() {
    let mut map = BTreeMap::new();
    for i in 0..1000u32 {
        map.try_insert(i * 2, i).unwrap();
    }
    for i in 0..1000u32 {
        common::fail_after(0);
        let result = map.try_insert_returning(i * 2 + 1, i);
        common::disarm();
        if let Err(e) = result {
            assert_eq!((e.key, e.value), (i * 2 + 1, i));
        }
    }
    map.check_invariants();
}

#[test]
fn try_clone_fails_cleanly() {
    let mut map = BTreeMap::new();
    let mut expected = StdMap::new();
    for i in 0..500u32 {
        map.try_insert(i, i).unwrap();
        expected.insert(i, i);
    }
    for budget in 0.. {
        common::fail_after(budget);
        let result = map.try_clone();
        common::disarm();
        assert_same(&map, &expected);
        if let Ok(copy) = result {
            assert_same(&copy, &expected);
            break;
        }
    }
}

#[test]
fn removals_and_iteration_do_not_allocate() {
    let mut map = BTreeMap::new();
    for i in 0..2000u32 {
        map.try_insert(i, i).unwrap();
    }
    common::fail_after(0);
    map.retain(|k, _| k % 5 != 0);
    map.pop_first();
    map.pop_last();
    map.iter_mut().for_each(|(_, v)| *v += 1);
    let in_range = map.range(100..900).count();
    if let kernel_btree::Entry::Occupied(e) = map.entry(7) {
        e.remove();
    }
    let rest = map.into_iter().count();
    let allocations = common::allocations();
    common::disarm();
    assert_eq!(allocations, 0);
    assert_eq!(in_range, 640);
    assert_eq!(rest, 2000 - 400 - 2 - 1);
}

/// Per-map allocator: counts live blocks and fails once `budget` runs out.
struct Pool {
    live: Cell<isize>,
    budget: Cell<usize>,
}

unsafe impl RawAlloc for Pool {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        if self.budget.get() == 0 {
            return std::ptr::null_mut();
        }
        self.budget.set(self.budget.get() - 1);
        self.live.set(self.live.get() + 1);
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.live.set(self.live.get() - 1);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[test]
fn custom_allocator_is_atomic_and_leak_free() {
    let pool = Pool { live: Cell::new(0), budget: Cell::new(usize::MAX) };
    let mut map = BTreeMap::new_in(&pool);
    let mut expected = StdMap::new();
    for i in 0..1500u32 {
        let key = i.wrapping_mul(2_654_435_761) % 4000;
        for budget in 0.. {
            pool.budget.set(budget);
            // nodes must come from the pool only
            common::fail_after(0);
            let result = map.try_insert(key, key);
            let global = common::allocations();
            common::disarm();
            assert_eq!(global, 0);
            match result {
                Err(AllocError) => assert_same(&map, &expected),
                Ok(old) => {
                    assert_eq!(old, expected.insert(key, key));
                    assert_same(&map, &expected);
                    break;
                }
            }
        }
    }
    for budget in [0, 1, 7, 40] {
        pool.budget.set(budget);
        assert!(map.try_clone().is_err());
    }
    pool.budget.set(usize::MAX);
    let copy = map.try_clone().unwrap();
    assert_same(&copy, &expected);
    map.retain(|k, _| k % 3 != 0);
    map.pop_first();
    map.into_iter().next_back();
    let mut set = kernel_btree::BTreeSet::new_in(&pool);
    for i in 0..500u32 {
        set.try_insert(i).unwrap();
    }
    drop(set);
    drop(copy);
    assert_eq!(pool.live.get(), 0);
}

#[test]
fn try_insert_is_atomic_on_sequential_keys() {
    // ascending/descending keys keep the right/left spine full, so every height change hits
    // the worst case: a split on every level plus a new root
    let n = if cfg!(miri) { 150 } else { 2000 };
    let orders: [Box<dyn Iterator<Item = u32>>; 2] = [Box::new(0..n), Box::new((0..n).rev())];
    for keys in orders {
        let mut map = BTreeMap::new();
        let mut expected = StdMap::new();
        for key in keys {
            insert_under_fault_injection(&mut map, &mut expected, key);
        }
    }
}

#[test]
fn failed_insert_drops_the_pair_once() {
    use std::rc::Rc;
    let v = Rc::new(());
    let mut map = BTreeMap::new();
    // a full root leaf: the next insert needs a leaf and a new root
    for i in 0..11u32 {
        map.try_insert(i * 2, Rc::clone(&v)).unwrap();
    }
    common::fail_after(0);
    let plain = map.try_insert(1, Rc::clone(&v)).is_err();
    let returned = map.try_insert_returning(3, Rc::clone(&v));
    let entry = map.entry(5).or_try_insert(Rc::clone(&v)).is_err();
    let replaced = map.entry(4).or_try_insert(Rc::clone(&v)).is_ok();
    let allocations = common::allocations();
    common::disarm();
    assert!(plain && entry && replaced);
    // replacing through an occupied entry keeps the stored Rc and drops the offered one
    assert_eq!(Rc::strong_count(&v), 1 + 11 + 1);
    let e = returned.unwrap_err();
    assert_eq!(e.key, 3);
    assert!(Rc::ptr_eq(&e.value, &v));
    drop(e);
    assert_eq!(Rc::strong_count(&v), 1 + 11);
    // one failed attempt per failing insert (reservation stops at the first null), none for the replace
    assert_eq!(allocations, 3);
    map.check_invariants();
    drop(map);
    assert_eq!(Rc::strong_count(&v), 1);
}
