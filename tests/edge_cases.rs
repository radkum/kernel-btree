//! Border cases: every split/steal/merge path, every range bound, iterator meeting points,
//! empty and single-entry maps, zero-sized types, `Borrow`, extreme keys, auto traits.

use std::collections::{BTreeMap as StdMap, BTreeSet as StdSet};
use std::ops::Bound;

use kernel_btree::{BTreeMap, BTreeSet, Entry, Iter, RawAlloc};

// sizes cross every height change: a leaf holds 11, height 1 up to 143, height 2 up to 1727
const MAX_N: u32 = if cfg!(miri) { 30 } else { 160 };

/// Tree shapes: dense right spine, dense left spine, scattered, and nodes near minimum fill.
fn shapes(n: u32) -> Vec<(&'static str, BTreeMap<u32, u32>)> {
    let mut asc = BTreeMap::new();
    let mut desc = BTreeMap::new();
    let mut scattered = BTreeMap::new();
    let mut thinned = BTreeMap::new();
    for i in 0..n {
        asc.try_insert(i, i).unwrap();
        desc.try_insert(n - 1 - i, n - 1 - i).unwrap();
        let k = (i * 7919) % n.max(1);
        scattered.try_insert(k, k).unwrap();
    }
    for i in 0..3 * n {
        thinned.try_insert(i, i).unwrap();
    }
    thinned.retain(|k, _| k % 3 == 0);
    let mut thinned_again = BTreeMap::new();
    for (k, v) in thinned {
        thinned_again.try_insert(k / 3, v / 3).unwrap();
    }
    vec![("asc", asc), ("desc", desc), ("scattered", scattered), ("thinned", thinned_again)]
}

fn assert_keys(map: &BTreeMap<u32, u32>, keys: impl Iterator<Item = u32> + Clone) {
    map.check_invariants();
    assert!(map.keys().copied().eq(keys.clone()));
    assert!(map.keys().rev().copied().eq(keys.collect::<Vec<_>>().into_iter().rev()));
}

#[test]
fn shapes_are_valid() {
    for n in 0..=MAX_N {
        for (name, map) in shapes(n) {
            assert_eq!(map.len(), n as usize, "{name} {n}");
            assert_keys(&map, 0..n);
        }
    }
}

#[test]
fn remove_every_key_from_every_shape() {
    for n in 0..=MAX_N {
        for (name, base) in shapes(n) {
            for k in 0..n {
                let mut map = base.try_clone().unwrap();
                assert_eq!(map.remove(&k), Some(k), "{name} {n} {k}");
                assert_eq!(map.remove(&k), None);
                assert_eq!(map.len(), n as usize - 1);
                assert_keys(&map, (0..n).filter(move |&x| x != k));
            }
            let mut map = base.try_clone().unwrap();
            assert_eq!(map.remove(&n), None);
            assert_keys(&map, 0..n);
        }
    }
}

#[test]
fn insert_into_every_gap_of_every_shape() {
    for n in 0..=MAX_N {
        for (name, base) in shapes(n) {
            // keys become 1, 3, 5, ...; insert every even key, i.e. before, between and after
            let mut odd = BTreeMap::new();
            for (k, v) in base.iter() {
                odd.try_insert(2 * k + 1, *v).unwrap();
            }
            for g in 0..=n {
                let mut map = odd.try_clone().unwrap();
                assert_eq!(map.try_insert(2 * g, 0), Ok(None), "{name} {n} {g}");
                map.check_invariants();
                assert!(map.keys().copied().eq((0..n).map(|i| 2 * i + 1).chain([2 * g]).collect::<StdSet<_>>()));
            }
        }
    }
}

#[test]
fn entry_slot_survives_splits() {
    for n in 0..=MAX_N {
        for (name, base) in shapes(n) {
            for g in 0..=n {
                let mut map = BTreeMap::new();
                for (k, v) in base.iter() {
                    map.try_insert(2 * k + 1, *v).unwrap();
                }
                let slot = map.entry(2 * g).or_try_insert(0).unwrap();
                *slot = 4242;
                map.check_invariants();
                assert_eq!(map.get(&(2 * g)), Some(&4242), "{name} {n} {g}");
            }
        }
    }
}

#[test]
fn drain_from_both_ends_and_middle() {
    for n in 0..=MAX_N {
        for (name, base) in shapes(n) {
            let mut map = base.try_clone().unwrap();
            for i in 0..n {
                assert_eq!(map.pop_first(), Some((i, i)), "{name} {n}");
                map.check_invariants();
            }
            assert_eq!(map.pop_first(), None);

            let mut map = base.try_clone().unwrap();
            for i in (0..n).rev() {
                assert_eq!(map.pop_last(), Some((i, i)), "{name} {n}");
                map.check_invariants();
            }
            assert_eq!(map.pop_last(), None);

            // middle out
            let mut map = base;
            let mut order: Vec<u32> = (0..n).collect();
            order.sort_by_key(|&k| (k as i64 - n as i64 / 2).abs());
            for k in order {
                assert_eq!(map.remove(&k), Some(k));
                map.check_invariants();
            }
            assert!(map.is_empty());
        }
    }
}

#[test]
fn map_is_reusable_after_emptying() {
    let mut map = BTreeMap::new();
    for round in 0..3u32 {
        for i in 0..200 {
            map.try_insert(i, round).unwrap();
        }
        match round {
            0 => map.clear(),
            1 => map.retain(|_, _| false),
            _ => (0..200).for_each(|i| assert_eq!(map.remove(&i), Some(round))),
        }
        assert!(map.is_empty());
        assert_eq!(map.iter().next(), None);
        map.check_invariants();
    }
    map.clear();
    map.clear();
    assert_eq!(map.try_insert(1, 1), Ok(None));
    assert_eq!(map.len(), 1);
}

#[test]
fn empty_map() {
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert_eq!(map.get(&0), None);
    assert_eq!(map.get_mut(&0), None);
    assert!(!map.contains_key(&0));
    assert_eq!(map.remove(&0), None);
    assert_eq!(map.first_key_value(), None);
    assert_eq!(map.last_key_value(), None);
    assert_eq!(map.pop_first(), None);
    assert_eq!(map.pop_last(), None);
    assert_eq!(map.iter().len(), 0);
    assert_eq!(map.iter().next_back(), None);
    assert_eq!(map.iter_mut().next(), None);
    assert_eq!(map.keys().next(), None);
    assert_eq!(map.values().next_back(), None);
    assert_eq!(map.values_mut().next(), None);
    assert_eq!(map.range(..).next(), None);
    assert_eq!(map.range(3..=3).next_back(), None);
    assert_eq!(map.range_mut(..).next(), None);
    map.retain(|_, _| unreachable!());
    assert!(matches!(map.entry(5), Entry::Vacant(_)));
    assert!(map.try_clone().unwrap().is_empty());
    assert_eq!(format!("{map:?}"), "{}");
    assert_eq!(map.into_iter().next(), None);
}

#[test]
fn single_entry() {
    let mut map = BTreeMap::new();
    map.try_insert(7u32, 70u32).unwrap();
    assert_eq!(map.first_key_value(), map.last_key_value());
    let mut it = map.iter();
    assert_eq!(it.len(), 1);
    assert_eq!(it.next_back(), Some((&7, &70)));
    assert_eq!(it.len(), 0);
    assert_eq!(it.next(), None);
    assert_eq!(map.range(7..=7).collect::<Vec<_>>(), [(&7, &70)]);
    assert_eq!(map.range(7..7).next(), None);
    assert_eq!(map.range(..7).next(), None);
    assert_eq!(map.range((Bound::Excluded(7), Bound::Unbounded)).next(), None);
    assert_eq!(map.range(..=7).next_back(), Some((&7, &70)));
    assert_eq!(map.pop_last(), Some((7, 70)));
    map.check_invariants();
    assert!(map.is_empty());
}

#[test]
fn iterators_meet_at_every_point() {
    for n in 0..=MAX_N.min(60) {
        for (name, map) in shapes(n) {
            for split in 0..=n {
                let mut it = map.iter();
                for i in 0..split {
                    assert_eq!(it.next(), Some((&i, &i)), "{name} {n} {split}");
                    assert_eq!(it.len(), (n - i - 1) as usize);
                }
                for i in (split..n).rev() {
                    assert_eq!(it.next_back(), Some((&i, &i)), "{name} {n} {split}");
                }
                assert_eq!(it.len(), 0);
                assert_eq!(it.next(), None);
                assert_eq!(it.next_back(), None);
                assert_eq!(it.next(), None);

                // the same through a range, which has no length to stop on
                let mut range = map.range(..);
                assert!((0..split).all(|i| range.next() == Some((&i, &i))));
                assert!((split..n).rev().all(|i| range.next_back() == Some((&i, &i))));
                assert_eq!(range.next(), None);
                assert_eq!(range.next_back(), None);
            }
        }
    }
}

#[test]
fn alternating_ends() {
    for n in 0..=MAX_N {
        for (name, map) in shapes(n) {
            let mut it = map.keys();
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                assert_eq!(it.next(), Some(&lo), "{name} {n}");
                lo += 1;
                if lo < hi {
                    hi -= 1;
                    assert_eq!(it.next_back(), Some(&hi));
                }
            }
            assert_eq!(it.next(), None);
            assert_eq!(it.next_back(), None);
        }
    }
}

fn all_bounds(lo: i32, hi: i32) -> Vec<Bound<i32>> {
    let mut v = vec![Bound::Unbounded];
    for x in lo..=hi {
        v.push(Bound::Included(x));
        v.push(Bound::Excluded(x));
    }
    v
}

// std panics on these; kernel-btree yields nothing
fn std_range_panics(lo: Bound<i32>, hi: Bound<i32>) -> bool {
    match (lo, hi) {
        (Bound::Excluded(a), Bound::Excluded(b)) => a >= b,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => a > b,
        _ => false,
    }
}

#[test]
fn every_range_bound_combination() {
    let sizes: &[i32] = if cfg!(miri) { &[0, 1, 3, 12] } else { &[0, 1, 2, 11, 12, 13, 40, 150] };
    for &n in sizes {
        // even keys, so every bound hits a key, a gap, or lies outside
        let mut map = BTreeMap::new();
        let mut expected = StdMap::new();
        for i in 0..n {
            map.try_insert(2 * i, i).unwrap();
            expected.insert(2 * i, i);
        }
        let bounds = all_bounds(-2, 2 * n + 1);
        for &lo in &bounds {
            for &hi in &bounds {
                if std_range_panics(lo, hi) {
                    assert_eq!(map.range((lo, hi)).count(), 0, "{lo:?} {hi:?}");
                    assert_eq!(map.range((lo, hi)).next_back(), None);
                    continue;
                }
                assert!(map.range((lo, hi)).eq(expected.range((lo, hi))), "{n} {lo:?} {hi:?}");
                assert!(map.range((lo, hi)).rev().eq(expected.range((lo, hi)).rev()));
                let mut ours = map.range((lo, hi));
                let mut theirs = expected.range((lo, hi));
                loop {
                    let a = ours.next();
                    assert_eq!(a, theirs.next());
                    let b = ours.next_back();
                    assert_eq!(b, theirs.next_back());
                    if a.is_none() || b.is_none() {
                        break;
                    }
                }
            }
        }
    }
}

#[test]
fn range_mut_writes_only_inside() {
    let mut map = BTreeMap::new();
    for i in 0..300u32 {
        map.try_insert(i, 0u32).unwrap();
    }
    map.range_mut(100..200).for_each(|(_, v)| *v += 1);
    map.range_mut(150..=250).rev().for_each(|(_, v)| *v += 10);
    for (k, v) in &map {
        let expected = u32::from((100..200).contains(k)) + 10 * u32::from((150..=250).contains(k));
        assert_eq!(*v, expected, "{k}");
    }
}

#[test]
fn range_syntax_forms() {
    let mut map = BTreeMap::new();
    for i in 0..20u32 {
        map.try_insert(i, ()).unwrap();
    }
    let keys = |r: kernel_btree::Range<'_, u32, ()>| r.map(|(k, _)| *k).collect::<Vec<_>>();
    assert_eq!(keys(map.range(..)), (0..20).collect::<Vec<_>>());
    assert_eq!(keys(map.range(5..)), (5..20).collect::<Vec<_>>());
    assert_eq!(keys(map.range(..5)), (0..5).collect::<Vec<_>>());
    assert_eq!(keys(map.range(..=5)), (0..=5).collect::<Vec<_>>());
    assert_eq!(keys(map.range(5..8)), [5, 6, 7]);
    assert_eq!(keys(map.range(5..=8)), [5, 6, 7, 8]);
    #[allow(clippy::reversed_empty_ranges)]
    let inverted = map.range(8..5);
    assert_eq!(keys(inverted), [] as [u32; 0]);
    assert_eq!(keys(map.range(19..=19)), [19]);
    assert_eq!(keys(map.range(20..)), [] as [u32; 0]);
    assert_eq!(keys(map.range(..0)), [] as [u32; 0]);
}

#[test]
fn extreme_keys() {
    let mut map = BTreeMap::new();
    for k in [u64::MIN, 1, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
        map.try_insert(k, k).unwrap();
    }
    assert_eq!(map.first_key_value(), Some((&u64::MIN, &u64::MIN)));
    assert_eq!(map.last_key_value(), Some((&u64::MAX, &u64::MAX)));
    assert_eq!(map.range(u64::MAX..).count(), 1);
    assert_eq!(map.range((Bound::Excluded(u64::MAX), Bound::Unbounded)).count(), 0);
    assert_eq!(map.range(..=u64::MIN).count(), 1);
    assert_eq!(map.range(..u64::MIN).count(), 0);
    assert_eq!(map.range(u64::MIN..=u64::MAX).count(), 5);

    let mut signed = BTreeMap::new();
    for k in [i64::MIN, -1, 0, 1, i64::MAX] {
        signed.try_insert(k, ()).unwrap();
    }
    assert!(signed.keys().copied().eq([i64::MIN, -1, 0, 1, i64::MAX]));
    assert_eq!(signed.range(-1..=1).count(), 3);
}

#[test]
fn borrowed_lookups() {
    let mut map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for word in ["delta", "alpha", "charlie", "bravo", ""] {
        map.try_insert(word.to_string(), word.as_bytes().to_vec()).unwrap();
    }
    assert_eq!(map.get("bravo").map(Vec::as_slice), Some(&b"bravo"[..]));
    assert!(map.contains_key(""));
    assert!(!map.contains_key("echo"));
    map.get_mut("alpha").unwrap().push(b'!');
    assert_eq!(map.get("alpha").unwrap(), b"alpha!");
    let mid: Vec<&str> =
        map.range::<str, _>((Bound::Included("b"), Bound::Excluded("d"))).map(|(k, _)| k.as_str()).collect();
    assert_eq!(mid, ["bravo", "charlie"]);
    assert_eq!(map.remove("delta"), Some(b"delta".to_vec()));
    assert_eq!(map.remove("delta"), None);
    assert!(map.keys().map(String::as_str).eq(["", "alpha", "bravo", "charlie"]));

    let mut set: BTreeSet<Vec<u8>> = BTreeSet::new();
    set.try_insert(vec![1, 2]).unwrap();
    assert!(set.contains(&[1u8, 2][..]));
    assert!(set.remove(&[1u8, 2][..]));
}

#[test]
fn zero_sized_types() {
    let mut map: BTreeMap<(), ()> = BTreeMap::new();
    assert_eq!(map.try_insert((), ()), Ok(None));
    assert_eq!(map.try_insert((), ()), Ok(Some(())));
    assert_eq!(map.len(), 1);
    assert_eq!(map.iter().next(), Some((&(), &())));
    assert_eq!(map.remove(&()), Some(()));
    assert!(map.is_empty());

    let mut units = BTreeMap::new();
    for i in 0..500u32 {
        units.try_insert(i, ()).unwrap();
    }
    units.check_invariants();
    assert_eq!(units.values().count(), 500);

    let mut set = BTreeSet::new();
    assert_eq!(set.try_insert(()), Ok(true));
    assert_eq!(set.try_insert(()), Ok(false));
    assert_eq!(set.pop_first(), Some(()));
}

#[test]
fn large_values() {
    let mut map = BTreeMap::new();
    for i in 0..100u32 {
        map.try_insert(i, [i as u8; 4096]).unwrap();
    }
    map.check_invariants();
    assert!(map.iter().all(|(k, v)| v.iter().all(|&b| b == *k as u8)));
    for i in (0..100).step_by(2) {
        assert_eq!(map.remove(&i).unwrap()[4095], i as u8);
    }
    map.check_invariants();
}

#[test]
fn entry_api() {
    let mut map = BTreeMap::new();
    match map.entry(1u32) {
        Entry::Vacant(e) => {
            assert_eq!(*e.key(), 1);
            assert_eq!(e.into_key(), 1);
        }
        Entry::Occupied(_) => unreachable!(),
    }
    assert!(map.is_empty());

    assert_eq!(*map.entry(1).or_try_insert(10).unwrap(), 10);
    assert_eq!(*map.entry(1).or_try_insert(20).unwrap(), 10);
    assert_eq!(*map.entry(1).or_try_insert_with(|| unreachable!()).unwrap(), 10);
    assert_eq!(*map.entry(2).and_modify(|_| unreachable!()).or_try_insert(5).unwrap(), 5);
    assert_eq!(*map.entry(2).and_modify(|v| *v *= 3).or_try_insert(0).unwrap(), 15);
    assert_eq!(map.entry(3).key(), &3);

    let Entry::Occupied(mut e) = map.entry(1) else { unreachable!() };
    assert_eq!((e.key(), e.get()), (&1, &10));
    *e.get_mut() += 1;
    assert_eq!(e.insert(100), 11);
    *e.into_mut() += 1;
    assert_eq!(map.get(&1), Some(&101));

    let Entry::Occupied(e) = map.entry(1) else { unreachable!() };
    assert_eq!(e.remove_entry(), (1, 101));
    let Entry::Occupied(e) = map.entry(2) else { unreachable!() };
    assert_eq!(e.remove(), 15);
    assert!(map.is_empty());
    map.check_invariants();
}

#[test]
fn entry_remove_at_every_position() {
    for n in 0..=MAX_N.min(80) {
        for (name, base) in shapes(n) {
            for k in 0..n {
                let mut map = base.try_clone().unwrap();
                let Entry::Occupied(e) = map.entry(k) else { panic!("{name} {n} {k}") };
                assert_eq!(e.remove_entry(), (k, k));
                assert_keys(&map, (0..n).filter(move |&x| x != k));
            }
        }
    }
}

#[test]
fn retain_patterns() {
    let n = if cfg!(miri) { 60 } else { 600 };
    type Keep = fn(u32, u32) -> bool;
    let patterns: [(&str, Keep); 8] = [
        ("all", |_, _| true),
        ("none", |_, _| false),
        ("not first", |k, _| k != 0),
        ("not last", |k, n| k != n - 1),
        ("only last", |k, n| k == n - 1),
        ("even", |k, _| k % 2 == 0),
        ("every 12th", |k, _| k % 12 == 0),
        ("first half", |k, n| k < n / 2),
    ];
    for (name, keep) in patterns {
        for (_, mut map) in shapes(n) {
            let mut seen = Vec::new();
            map.retain(|k, v| {
                seen.push(*k);
                *v += 1;
                keep(*k, n)
            });
            assert!(seen.iter().copied().eq(0..n), "{name}: every pair visited once, in order");
            map.check_invariants();
            assert!(map.keys().copied().eq((0..n).filter(|&k| keep(k, n))), "{name}");
            assert!(map.iter().all(|(k, v)| *v == k + 1));
        }
    }
}

#[test]
fn iter_mut_and_values_mut() {
    let mut map = BTreeMap::new();
    for i in 0..100u32 {
        map.try_insert(i, i).unwrap();
    }
    let mut it = map.iter_mut();
    assert_eq!(it.len(), 100);
    *it.next().unwrap().1 = 1000;
    *it.next_back().unwrap().1 = 2000;
    assert_eq!(it.len(), 98);
    it.for_each(|(_, v)| *v *= 2);
    map.values_mut().rev().take(1).for_each(|v| *v += 1);
    assert_eq!(map.get(&0), Some(&1000));
    assert_eq!(map.get(&99), Some(&2001));
    assert_eq!(map.get(&50), Some(&100));
    for (_, v) in &mut map {
        *v = 0;
    }
    assert!(map.values().all(|&v| v == 0));
}

#[test]
fn into_iter_both_ends_and_partial_drop() {
    for n in [0u32, 1, 2, 11, 12, 100] {
        let mut map = BTreeMap::new();
        for i in 0..n {
            map.try_insert(i, i.to_string()).unwrap();
        }
        let mut it = map.into_iter();
        assert_eq!(it.len(), n as usize);
        if n > 0 {
            assert_eq!(it.next_back(), Some((n - 1, (n - 1).to_string())));
            assert_eq!(it.next().map(|(k, _)| k), (n > 1).then_some(0));
        }
        assert_eq!(it.size_hint().0, n.saturating_sub(2) as usize);
        drop(it);
    }
    let mut set = BTreeSet::new();
    for i in 0..30u8 {
        set.try_insert(i).unwrap();
    }
    let mut it = set.into_iter();
    assert_eq!((it.next(), it.next_back(), it.len()), (Some(0), Some(29), 28));
}

#[test]
fn try_insert_returning_replaces_without_handing_back() {
    let mut map = BTreeMap::new();
    assert!(matches!(map.try_insert_returning(1, "a"), Ok(None)));
    assert!(matches!(map.try_insert_returning(1, "b"), Ok(Some("a"))));
    assert_eq!(map.get(&1), Some(&"b"));
}

#[test]
fn try_clone_is_independent() {
    let mut map = BTreeMap::new();
    for i in 0..300u32 {
        map.try_insert(i, vec![i]).unwrap();
    }
    let mut copy = map.try_clone().unwrap();
    copy.check_invariants();
    copy.get_mut(&5).unwrap().push(1);
    copy.remove(&6);
    assert_eq!(map.get(&5), Some(&vec![5]));
    assert!(map.contains_key(&6));
    assert_eq!(copy.len(), 299);
}

#[test]
fn set_api() {
    let mut set = BTreeSet::default();
    assert_eq!(set.first(), None);
    for v in [5u32, 1, 9, 1, 5] {
        set.try_insert(v).unwrap();
    }
    assert_eq!(set.len(), 3);
    assert_eq!((set.first(), set.last()), (Some(&1), Some(&9)));
    assert!(set.range(2..9).eq([&5]));
    assert!(set.range(..).rev().eq([&9, &5, &1]));
    assert_eq!(format!("{set:?}"), "{1, 5, 9}");
    let copy = set.try_clone().unwrap();
    set.retain(|v| *v > 1);
    assert!(set.iter().eq([&5, &9]));
    assert_eq!(set.pop_last(), Some(9));
    assert!(!set.remove(&9));
    set.clear();
    assert!(set.is_empty());
    assert!(copy.iter().copied().eq([1, 5, 9]));
    assert!((&copy).into_iter().eq(copy.iter()));
    copy.check_invariants();
}

#[test]
fn debug_matches_std() {
    let mut map = BTreeMap::new();
    let mut expected = StdMap::new();
    for i in [3u8, 1, 2] {
        map.try_insert(i, char::from(b'a' + i)).unwrap();
        expected.insert(i, char::from(b'a' + i));
    }
    assert_eq!(format!("{map:?}"), format!("{expected:?}"));
    assert_eq!(format!("{map:#?}"), format!("{expected:#?}"));
}

#[test]
fn default_and_const_new() {
    static EMPTY: BTreeMap<u32, u32> = BTreeMap::new();
    assert!(EMPTY.is_empty());
    let map: BTreeMap<u32, u32> = BTreeMap::default();
    assert!(map.is_empty());
    let set: BTreeSet<u32> = BTreeSet::new();
    assert!(set.is_empty());
}

#[test]
fn shared_allocator_by_reference() {
    let global = kernel_btree::Global;
    let mut a = BTreeMap::new_in(&global);
    let mut b = BTreeMap::new_in(&global);
    for i in 0..100u32 {
        a.try_insert(i, i).unwrap();
        b.try_insert(i, i).unwrap();
    }
    assert!(a.iter().eq(b.iter()));
    assert_eq!(kernel_btree::Global.alloc(std::alloc::Layout::new::<()>()), std::ptr::null_mut());
}

// compile-time checks
#[allow(dead_code)]
fn auto_traits() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<BTreeMap<u32, String>>();
    sync::<BTreeMap<u32, String>>();
    send::<Iter<'static, u32, String>>();
    sync::<Iter<'static, u32, String>>();
    send::<kernel_btree::IterMut<'static, u32, String>>();
    send::<kernel_btree::IntoIter<u32, String>>();
    send::<BTreeSet<u32>>();
    sync::<kernel_btree::set::Iter<'static, u32>>();
}

#[allow(dead_code)]
fn covariance<'a>(
    map: BTreeMap<&'static str, &'static str>,
    iter: Iter<'a, &'static str, &'static str>,
) -> (BTreeMap<&'a str, &'a str>, Iter<'a, &'a str, &'a str>) {
    (map, iter)
}

#[allow(dead_code)]
fn holds_references_to_outer_data() {
    let data = String::from("x");
    let mut map = BTreeMap::new();
    map.try_insert(data.as_str(), &data).unwrap();
    drop(map);
    drop(data);
}

#[test]
fn errors_and_size_hints() {
    assert_eq!(kernel_btree::AllocError.to_string(), "memory allocation failed");
    let e = kernel_btree::InsertError { key: 1, value: "v" };
    assert_eq!(e.to_string(), "memory allocation failed");
    assert_eq!(format!("{e:?}"), "InsertError { .. }");
    let _: &dyn std::error::Error = &e;

    let mut map = BTreeMap::new();
    for i in 0..30u32 {
        map.try_insert(i, i).unwrap();
    }
    assert_eq!(map.entry(3).key(), &3);
    assert_eq!(map.keys().size_hint(), (30, Some(30)));
    assert_eq!(map.values().len(), 30);
    assert_eq!(map.values_mut().len(), 30);
    let mut set = BTreeSet::new();
    set.try_insert(1u8).unwrap();
    assert_eq!(set.iter().len(), 1);
}
