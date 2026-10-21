//! Ordered set: a [`BTreeMap`] with `()` values and the same guarantees.

use core::borrow::Borrow;
use core::fmt;
use core::ops::RangeBounds;

use crate::{iter, AllocError, BTreeMap, Global, RawAlloc};

pub struct BTreeSet<T, A: RawAlloc = Global> {
    map: BTreeMap<T, (), A>,
}

impl<T: Ord> BTreeSet<T> {
    /// Creates an empty set. Does not allocate.
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<T: Ord, A: RawAlloc> BTreeSet<T, A> {
    /// Creates an empty set whose nodes come from `alloc`. Does not allocate.
    pub const fn new_in(alloc: A) -> Self {
        Self { map: BTreeMap::new_in(alloc) }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains<Q: ?Sized + Ord>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
    {
        self.map.contains_key(value)
    }

    /// Adds `value`; returns whether it was new. Atomic: on `Err` the set is unchanged.
    pub fn try_insert(&mut self, value: T) -> Result<bool, AllocError> {
        Ok(self.map.try_insert(value, ())?.is_none())
    }

    /// Never allocates.
    pub fn remove<Q: ?Sized + Ord>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
    {
        self.map.remove(value).is_some()
    }

    pub fn first(&self) -> Option<&T> {
        self.map.first_key_value().map(|(k, _)| k)
    }

    pub fn last(&self) -> Option<&T> {
        self.map.last_key_value().map(|(k, _)| k)
    }

    pub fn pop_first(&mut self) -> Option<T> {
        self.map.pop_first().map(|(k, _)| k)
    }

    pub fn pop_last(&mut self) -> Option<T> {
        self.map.pop_last().map(|(k, _)| k)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut f: F) {
        self.map.retain(|k, _| f(k));
    }

    pub fn range<Q: ?Sized + Ord, R: RangeBounds<Q>>(&self, range: R) -> Range<'_, T>
    where
        T: Borrow<Q>,
    {
        Range(self.map.range(range))
    }

    pub fn try_clone(&self) -> Result<Self, AllocError>
    where
        T: Clone,
        A: Clone,
    {
        Ok(Self { map: self.map.try_clone()? })
    }

    /// Panics if the tree structure is broken. For tests.
    #[cfg(feature = "debug-invariants")]
    pub fn check_invariants(&self) {
        self.map.check_invariants();
    }
}

impl<T, A: RawAlloc> BTreeSet<T, A> {
    /// Iterates in ascending order.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter(self.map.keys())
    }
}

impl<T: Ord> Default for BTreeSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug, A: RawAlloc> fmt::Debug for BTreeSet<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

pub struct Iter<'a, T>(iter::Keys<'a, T, ()>);

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

pub struct Range<'a, T>(iter::Range<'a, T, ()>);

impl<'a, T> Iterator for Range<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }
}

impl<T> DoubleEndedIterator for Range<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, _)| k)
    }
}

pub struct IntoIter<T, A: RawAlloc = Global>(iter::IntoIter<T, (), A>);

impl<T, A: RawAlloc> Iterator for IntoIter<T, A> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T, A: RawAlloc> DoubleEndedIterator for IntoIter<T, A> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, _)| k)
    }
}

impl<T, A: RawAlloc> ExactSizeIterator for IntoIter<T, A> {}

impl<'a, T, A: RawAlloc> IntoIterator for &'a BTreeSet<T, A> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T, A: RawAlloc> IntoIterator for BTreeSet<T, A> {
    type Item = T;
    type IntoIter = IntoIter<T, A>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter(self.map.into_iter())
    }
}
