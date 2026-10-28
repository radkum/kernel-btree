//! Random operation sequences against std::collections::BTreeMap.

use std::collections::{BTreeMap as StdMap, BTreeSet as StdSet};
use std::ops::Bound;

use kernel_btree::{BTreeMap, BTreeSet, Entry};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Insert(u16, u32),
    Remove(u16),
    Get(u16),
    Clear,
    Range(Bound<u16>, Bound<u16>),
    RangeMut(u16, u16),
    Entry(u16, u32),
    Retain(u16),
    PopFirst,
    PopLast,
    IterMut,
    Clone,
}

fn bound() -> impl Strategy<Value = Bound<u16>> {
    prop_oneof![(0..520u16).prop_map(Bound::Included), (0..520u16).prop_map(Bound::Excluded), Just(Bound::Unbounded),]
}

fn op() -> impl Strategy<Value = Op> {
    // small key space: frequent hits on existing keys
    prop_oneof![
        8 => (0..512u16, any::<u32>()).prop_map(|(k, v)| Op::Insert(k, v)),
        4 => (0..512u16).prop_map(Op::Remove),
        2 => (0..512u16).prop_map(Op::Get),
        1 => Just(Op::Clear),
        2 => (bound(), bound()).prop_map(|(lo, hi)| Op::Range(lo, hi)),
        1 => (0..512u16, 0..512u16).prop_map(|(lo, hi)| Op::RangeMut(lo, hi)),
        3 => (0..512u16, any::<u32>()).prop_map(|(k, v)| Op::Entry(k, v)),
        1 => (2..8u16).prop_map(Op::Retain),
        1 => Just(Op::PopFirst),
        1 => Just(Op::PopLast),
        1 => Just(Op::IterMut),
        1 => Just(Op::Clone),
    ]
}

// std panics on these; kernel-btree yields nothing
fn std_range_panics(lo: Bound<u16>, hi: Bound<u16>) -> bool {
    match (lo, hi) {
        (Bound::Excluded(a), Bound::Excluded(b)) => a >= b,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => a > b,
        _ => false,
    }
}

proptest! {
    #[test]
    fn matches_std(ops in prop::collection::vec(op(), 0..2000)) {
        let mut map = BTreeMap::new();
        let mut expected = StdMap::new();
        for op in ops {
            match op {
                Op::Insert(k, v) => prop_assert_eq!(map.try_insert(k, v).unwrap(), expected.insert(k, v)),
                Op::Remove(k) => prop_assert_eq!(map.remove(&k), expected.remove(&k)),
                Op::Get(k) => {
                    prop_assert_eq!(map.get(&k), expected.get(&k));
                    prop_assert_eq!(map.contains_key(&k), expected.contains_key(&k));
                }
                Op::Clear => {
                    map.clear();
                    expected.clear();
                }
                Op::Range(lo, hi) => {
                    if std_range_panics(lo, hi) {
                        prop_assert_eq!(map.range((lo, hi)).count(), 0);
                    } else {
                        prop_assert!(map.range((lo, hi)).eq(expected.range((lo, hi))));
                        prop_assert!(map.range((lo, hi)).rev().eq(expected.range((lo, hi)).rev()));
                    }
                }
                Op::RangeMut(lo, hi) => {
                    let (lo, hi) = (lo.min(hi), lo.max(hi));
                    map.range_mut(lo..hi).for_each(|(_, v)| *v = v.wrapping_add(1));
                    expected.range_mut(lo..hi).for_each(|(_, v)| *v = v.wrapping_add(1));
                }
                Op::Entry(k, v) => {
                    let ours = match map.entry(k) {
                        Entry::Occupied(mut e) if v % 3 == 0 => Some(e.insert(v)),
                        Entry::Occupied(e) if v % 3 == 1 => Some(e.remove()),
                        e => {
                            *e.and_modify(|x| *x ^= 1).or_try_insert(v).unwrap() += 1;
                            None
                        }
                    };
                    let theirs = match expected.entry(k) {
                        std::collections::btree_map::Entry::Occupied(mut e) if v % 3 == 0 => Some(e.insert(v)),
                        std::collections::btree_map::Entry::Occupied(e) if v % 3 == 1 => Some(e.remove()),
                        e => {
                            *e.and_modify(|x| *x ^= 1).or_insert(v) += 1;
                            None
                        }
                    };
                    prop_assert_eq!(ours, theirs);
                }
                Op::Retain(m) => {
                    map.retain(|k, v| k.wrapping_add(*v as u16) % m != 0);
                    expected.retain(|k, v| k.wrapping_add(*v as u16) % m != 0);
                }
                Op::PopFirst => prop_assert_eq!(map.pop_first(), expected.pop_first()),
                Op::PopLast => prop_assert_eq!(map.pop_last(), expected.pop_last()),
                Op::IterMut => {
                    map.values_mut().for_each(|v| *v = v.rotate_left(1));
                    expected.values_mut().for_each(|v| *v = v.rotate_left(1));
                }
                Op::Clone => {
                    let copy = map.try_clone().unwrap();
                    copy.check_invariants();
                    prop_assert!(copy.iter().eq(map.iter()));
                    map = copy;
                }
            }
            map.check_invariants();
            prop_assert_eq!(map.len(), expected.len());
        }
        prop_assert!(map.iter().eq(expected.iter()));
        prop_assert!(map.iter().rev().eq(expected.iter().rev()));
        prop_assert!(map.keys().eq(expected.keys()));
        prop_assert!(map.values().rev().eq(expected.values().rev()));
        prop_assert!(map.iter_mut().rev().eq(expected.iter_mut().rev()));
        prop_assert_eq!(map.first_key_value(), expected.first_key_value());
        prop_assert_eq!(map.last_key_value(), expected.last_key_value());
        // alternate ends until they meet
        let (mut ours, mut theirs) = (map.iter(), expected.iter());
        loop {
            let (a, b) = (ours.next(), theirs.next());
            prop_assert_eq!(a, b);
            let (c, d) = (ours.next_back(), theirs.next_back());
            prop_assert_eq!(c, d);
            prop_assert_eq!(ours.len(), theirs.len());
            if b.is_none() || d.is_none() {
                break;
            }
        }
        let mut ours = map.into_iter();
        let mut theirs = expected.into_iter();
        prop_assert_eq!(ours.next_back(), theirs.next_back());
        prop_assert!(ours.eq(theirs));
    }

    #[test]
    fn set_matches_std(ops in prop::collection::vec((0..3u8, 0..300u16), 0..1000)) {
        let mut set = BTreeSet::new();
        let mut expected = StdSet::new();
        for (op, v) in ops {
            match op {
                0 => prop_assert_eq!(set.try_insert(v).unwrap(), expected.insert(v)),
                1 => prop_assert_eq!(set.remove(&v), expected.remove(&v)),
                _ => prop_assert!(set.range(v..).eq(expected.range(v..))),
            }
            set.check_invariants();
        }
        prop_assert!(set.iter().rev().eq(expected.iter().rev()));
        prop_assert_eq!(set.first(), expected.first());
        prop_assert_eq!(set.last(), expected.last());
        prop_assert!(set.into_iter().eq(expected.into_iter()));
    }
}
