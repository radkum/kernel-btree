//! Panics from user code (`Ord`, `Clone`, `Drop`, closures) and inconsistent `Ord`: the map
//! stays usable, nothing is freed or dropped twice, and there is no UB (run under Miri too).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use kernel_btree::{BTreeMap, RawAlloc};

const N: u32 = if cfg!(miri) { 40 } else { 400 };

thread_local! {
    // comparisons left before `Key::cmp` panics; usize::MAX = never
    static CMP_BUDGET: Cell<usize> = const { Cell::new(usize::MAX) };
    static CLONE_BUDGET: Cell<usize> = const { Cell::new(usize::MAX) };
    static CREATED: Cell<usize> = const { Cell::new(0) };
    static DROPPED: Cell<usize> = const { Cell::new(0) };
    static PANIC_ON_DROP: Cell<Option<u32>> = const { Cell::new(None) };
    static CHAOS: Cell<u64> = const { Cell::new(0x9E37_79B9_7F4A_7C15) };
}

/// Key whose comparisons can be made to panic.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key(u32);

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        let budget = CMP_BUDGET.get();
        if budget == 0 {
            panic!("cmp bomb");
        }
        if budget != usize::MAX {
            CMP_BUDGET.set(budget - 1);
        }
        self.0.cmp(&other.0)
    }
}

/// Value that counts creations and drops, and can panic on drop or clone.
#[derive(Debug)]
struct Tracked(u32);

impl Tracked {
    fn new(id: u32) -> Self {
        CREATED.set(CREATED.get() + 1);
        Tracked(id)
    }
}

impl Clone for Tracked {
    fn clone(&self) -> Self {
        let budget = CLONE_BUDGET.get();
        if budget == 0 {
            panic!("clone bomb");
        }
        if budget != usize::MAX {
            CLONE_BUDGET.set(budget - 1);
        }
        Tracked::new(self.0)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        DROPPED.set(DROPPED.get() + 1);
        if PANIC_ON_DROP.get() == Some(self.0) {
            PANIC_ON_DROP.set(None);
            panic!("drop bomb");
        }
    }
}

fn reset_counters() {
    CMP_BUDGET.set(usize::MAX);
    CLONE_BUDGET.set(usize::MAX);
    CREATED.set(0);
    DROPPED.set(0);
    PANIC_ON_DROP.set(None);
}

fn alive() -> usize {
    CREATED.get() - DROPPED.get()
}

/// Node allocator that records live blocks, so a double free or a foreign pointer is caught.
#[derive(Default)]
struct Tracker {
    live: RefCell<HashSet<usize>>,
    bad_free: Cell<bool>,
}

unsafe impl RawAlloc for Tracker {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        self.live.borrow_mut().insert(ptr as usize);
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !self.live.borrow_mut().remove(&(ptr as usize)) {
            // never free it twice for real; just report
            self.bad_free.set(true);
            return;
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

impl Tracker {
    fn live(&self) -> usize {
        self.live.borrow().len()
    }
}

fn filled(tracker: &Tracker, n: u32) -> BTreeMap<Key, Tracked, &Tracker> {
    let mut map = BTreeMap::new_in(tracker);
    for i in 0..n {
        map.try_insert(Key(i * 2), Tracked::new(i * 2)).unwrap();
    }
    map
}

fn assert_contents(map: &BTreeMap<Key, Tracked, &Tracker>, keys: impl Iterator<Item = u32>) {
    CMP_BUDGET.set(usize::MAX);
    map.check_invariants();
    assert!(
        map.iter()
            .map(|(k, v)| {
                assert_eq!(k.0, v.0);
                k.0
            })
            .eq(keys)
    );
}

fn quiet<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    result
}

#[test]
fn ord_panic_in_every_operation_leaves_map_unchanged() {
    reset_counters();
    let tracker = Tracker::default();
    let mut map = filled(&tracker, N);
    let all = || (0..N).map(|i| i * 2);
    for budget in 0..12 {
        for probe in [0, 1, N - 1, N, 2 * N + 1] {
            CMP_BUDGET.set(budget);
            let r = quiet(|| map.try_insert(Key(probe), Tracked::new(probe)));
            let existing = probe % 2 == 0 && probe < 2 * N;
            if r.is_ok() && !existing {
                CMP_BUDGET.set(usize::MAX);
                map.remove(&Key(probe));
            }
            assert_contents(&map, all());

            CMP_BUDGET.set(budget);
            if let Ok(Some(v)) = quiet(|| map.remove(&Key(probe))) {
                CMP_BUDGET.set(usize::MAX);
                map.try_insert(Key(probe), v).unwrap();
            }
            CMP_BUDGET.set(budget);
            let _ = quiet(|| map.get(&Key(probe)).is_some());
            CMP_BUDGET.set(budget);
            let _ = quiet(|| map.range(Key(probe)..Key(probe + 10)).count());
            CMP_BUDGET.set(budget);
            let _ = quiet(|| {
                let _ = map.entry(Key(probe + 1));
            });
            assert_contents(&map, all());
        }
    }
    drop(map);
    assert_eq!(alive(), 0);
    assert_eq!(tracker.live(), 0);
    assert!(!tracker.bad_free.get());
}

#[test]
fn clone_panic_in_try_clone() {
    reset_counters();
    let tracker = Tracker::default();
    let map = filled(&tracker, N);
    for budget in [0, 1, 5, 11, 12, 13, N as usize / 2, N as usize - 1] {
        let before_alive = alive();
        let before_nodes = tracker.live();
        CLONE_BUDGET.set(budget);
        assert!(quiet(|| map.try_clone()).is_err());
        CLONE_BUDGET.set(usize::MAX);
        // the partial copy was dropped: its values and nodes are gone
        assert_eq!(alive(), before_alive, "budget {budget}");
        assert_eq!(tracker.live(), before_nodes);
        assert_contents(&map, (0..N).map(|i| i * 2));
    }
    drop(map);
    assert_eq!(alive(), 0);
    assert_eq!(tracker.live(), 0);
    assert!(!tracker.bad_free.get());
}

#[test]
fn retain_closure_panic_keeps_map_consistent() {
    for stop in [0, 1, 5, N / 2, N - 1] {
        reset_counters();
        let tracker = Tracker::default();
        let mut map = filled(&tracker, N);
        let mut visited = 0;
        let r = quiet(|| {
            map.retain(|k, _| {
                visited += 1;
                if k.0 == stop * 2 {
                    panic!("retain bomb");
                }
                k.0 % 4 == 0
            })
        });
        assert!(r.is_err());
        assert_eq!(visited, stop + 1);
        // before the panic: filtered; from the panicking key on: untouched
        assert_contents(&map, (0..N).map(|i| i * 2).filter(|&k| k >= stop * 2 || k % 4 == 0));
        assert_eq!(alive(), map.len());
        drop(map);
        assert_eq!(alive(), 0);
        assert_eq!(tracker.live(), 0);
        assert!(!tracker.bad_free.get());
    }
}

#[test]
fn drop_panic_in_remove_and_pop() {
    reset_counters();
    let tracker = Tracker::default();
    let mut map = filled(&tracker, N);
    for k in [0, 2 * (N / 2), 2 * (N - 1)] {
        PANIC_ON_DROP.set(Some(k));
        // the value is handed out and dropped by the caller, after the map is updated
        assert!(quiet(|| drop(map.remove(&Key(k)))).is_err());
        CMP_BUDGET.set(usize::MAX);
        map.check_invariants();
        assert!(!map.contains_key(&Key(k)));
    }
    PANIC_ON_DROP.set(Some(2));
    assert!(quiet(|| drop(map.pop_first())).is_err());
    assert_eq!(map.len(), N as usize - 4);
    map.check_invariants();
    drop(map);
    assert_eq!(alive(), 0);
    assert_eq!(tracker.live(), 0);
    assert!(!tracker.bad_free.get());
}

// a panicking `Drop` while clearing leaks the rest by design; Miri reports the leak
#[test]
#[cfg_attr(miri, ignore)]
fn drop_panic_in_clear_and_drop_never_double_frees() {
    for bomb in [0, 1, N / 2, N - 1] {
        reset_counters();
        let tracker = Tracker::default();
        let mut map = filled(&tracker, N);
        PANIC_ON_DROP.set(Some(bomb * 2));
        assert!(quiet(|| map.clear()).is_err());
        // detached before dropping: empty and reusable
        assert!(map.is_empty());
        assert!(map.iter().next().is_none());
        map.try_insert(Key(1), Tracked::new(1)).unwrap();
        map.check_invariants();
        drop(map);
        assert!(!tracker.bad_free.get(), "bomb {bomb}");

        let tracker = Tracker::default();
        let map = filled(&tracker, N);
        PANIC_ON_DROP.set(Some(bomb * 2));
        assert!(quiet(|| drop(map)).is_err());
        assert!(!tracker.bad_free.get(), "bomb {bomb}");
    }
}

#[test]
fn every_value_dropped_exactly_once() {
    reset_counters();
    let tracker = Tracker::default();
    let mut map = BTreeMap::new_in(&tracker);
    for i in 0..N {
        assert!(map.try_insert(Key(i), Tracked::new(i)).unwrap().is_none());
    }
    // replace hands the old value back
    let old = map.try_insert(Key(3), Tracked::new(3)).unwrap();
    assert_eq!(old.map(|v| v.0), Some(3));
    let _ = map.entry(Key(4)).or_try_insert(Tracked::new(999));
    map.remove(&Key(5));
    map.pop_first();
    map.pop_last();
    map.retain(|k, _| k.0 % 3 != 0);
    let copy = map.try_clone().unwrap();
    let mut it = copy.into_iter();
    it.next();
    it.next_back();
    drop(it);
    let mut into = map.try_clone().unwrap().into_iter();
    while into.next().is_some() {}
    map.clear();
    for i in 0..N {
        map.try_insert(Key(i), Tracked::new(i)).unwrap();
    }
    drop(map);
    assert_eq!(alive(), 0, "created {} dropped {}", CREATED.get(), DROPPED.get());
    assert_eq!(tracker.live(), 0);
    assert!(!tracker.bad_free.get());
}

/// Key with a deliberately inconsistent `Ord`.
#[derive(Debug, PartialEq, Eq)]
struct Chaos(u32);

impl PartialOrd for Chaos {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Chaos {
    fn cmp(&self, _: &Self) -> Ordering {
        let mut x = CHAOS.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        CHAOS.set(x);
        [Ordering::Less, Ordering::Equal, Ordering::Greater][(x % 3) as usize]
    }
}

#[test]
fn inconsistent_ord_is_memory_safe() {
    reset_counters();
    let tracker = Tracker::default();
    let mut map = BTreeMap::new_in(&tracker);
    let ops = if cfg!(miri) { 400 } else { 20_000 };
    for i in 0..ops {
        match i % 9 {
            0..=3 => drop(map.try_insert(Chaos(i), Tracked::new(i))),
            4 => drop(map.remove(&Chaos(i))),
            5 => drop(map.get(&Chaos(i))),
            6 => assert!(map.range(Chaos(i)..Chaos(i + 1)).count() <= map.len()),
            7 => map.retain(|k, _| k.0 % 5 != 0),
            _ => drop(map.entry(Chaos(i)).or_try_insert(Tracked::new(i))),
        }
        // structure stays intact even though the order is garbage
        assert_eq!(map.iter().count(), map.len());
        assert_eq!(map.iter().rev().count(), map.len());
        assert_eq!(alive(), map.len());
    }
    let n = map.len();
    assert_eq!(map.into_iter().count(), n);
    assert_eq!(alive(), 0);
    assert_eq!(tracker.live(), 0);
    assert!(!tracker.bad_free.get());
}

#[test]
fn degenerate_orders() {
    #[derive(Debug, PartialEq, Eq)]
    struct Always(Ordering, u32);
    impl PartialOrd for Always {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Always {
        fn cmp(&self, _: &Self) -> Ordering {
            self.0
        }
    }
    for ord in [Ordering::Less, Ordering::Equal, Ordering::Greater] {
        let mut map = BTreeMap::new();
        for i in 0..300 {
            map.try_insert(Always(ord, i), i).unwrap();
        }
        let expected = if ord == Ordering::Equal { 1 } else { 300 };
        assert_eq!(map.len(), expected);
        assert_eq!(map.iter().count(), expected);
        assert!(map.range(Always(ord, 0)..Always(ord, 1)).count() <= expected);
        for i in 0..300 {
            map.remove(&Always(ord, i));
        }
        assert_eq!(map.iter().count(), map.len());
        map.retain(|_, v| *v % 2 == 0);
        assert_eq!(map.iter().rev().count(), map.len());
    }
}
