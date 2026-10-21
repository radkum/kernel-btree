//! Iterators. They walk the tree through `parent` links and never allocate.

use core::marker::PhantomData;

use crate::node::{self, NodePtr};
use crate::{BTreeMap, Global, RawAlloc};

/// A key/value pair in the tree.
pub(crate) struct Pos<K, V> {
    // `*const` keeps `K`/`V` covariant
    pub(crate) node: *const node::LeafNode<K, V>,
    pub(crate) idx: usize,
    pub(crate) height: usize,
}

impl<K, V> Clone for Pos<K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K, V> Copy for Pos<K, V> {}

impl<K, V> Pos<K, V> {
    pub(crate) unsafe fn kv<'a>(self) -> (&'a K, &'a V) {
        (&*node::key(self.node.cast_mut(), self.idx), &*node::val(self.node.cast_mut(), self.idx))
    }

    pub(crate) unsafe fn kv_mut<'a>(self) -> (&'a K, &'a mut V) {
        (&*node::key(self.node.cast_mut(), self.idx), &mut *node::val(self.node.cast_mut(), self.idx))
    }
}

pub(crate) unsafe fn first_leaf<K, V>(mut node: NodePtr<K, V>, height: usize) -> NodePtr<K, V> {
    for _ in 0..height {
        node = node::child(node, 0);
    }
    node
}

pub(crate) unsafe fn last_leaf<K, V>(mut node: NodePtr<K, V>, height: usize) -> NodePtr<K, V> {
    for _ in 0..height {
        node = node::child(node, node::len(node));
    }
    node
}

/// The first pair after leaf edge `idx` of `node`, if any.
pub(crate) unsafe fn next_kv<K, V>(mut node: NodePtr<K, V>, mut idx: usize) -> Option<Pos<K, V>> {
    let mut height = 0;
    while idx >= node::len(node) {
        let parent = node::parent(node);
        if parent.is_null() {
            return None;
        }
        (node, idx, height) = (parent, node::parent_idx(node), height + 1);
    }
    Some(Pos { node, idx, height })
}

/// The last pair before leaf edge `idx` of `node`, if any.
pub(crate) unsafe fn prev_kv<K, V>(mut node: NodePtr<K, V>, mut idx: usize) -> Option<Pos<K, V>> {
    let mut height = 0;
    while idx == 0 {
        let parent = node::parent(node);
        if parent.is_null() {
            return None;
        }
        (node, idx, height) = (parent, node::parent_idx(node), height + 1);
    }
    Some(Pos { node, idx: idx - 1, height })
}

pub(crate) unsafe fn successor<K, V>(pos: Pos<K, V>) -> Option<Pos<K, V>> {
    if pos.height == 0 {
        next_kv(pos.node.cast_mut(), pos.idx + 1)
    } else {
        next_kv(first_leaf(node::child(pos.node.cast_mut(), pos.idx + 1), pos.height - 1), 0)
    }
}

unsafe fn predecessor<K, V>(pos: Pos<K, V>) -> Option<Pos<K, V>> {
    if pos.height == 0 {
        prev_kv(pos.node.cast_mut(), pos.idx)
    } else {
        let leaf = last_leaf(node::child(pos.node.cast_mut(), pos.idx), pos.height - 1);
        prev_kv(leaf, node::len(leaf))
    }
}

/// Pairs from `front` to `back` inclusive; `None` once exhausted.
pub(crate) struct Raw<K, V> {
    range: Option<(Pos<K, V>, Pos<K, V>)>,
}

impl<K, V> Raw<K, V> {
    pub(crate) fn new(front: Option<Pos<K, V>>, back: Option<Pos<K, V>>) -> Self {
        Self { range: front.zip(back) }
    }

    // stops at the end of the tree even if `back` is never met (inconsistent `Ord`)
    unsafe fn next(&mut self) -> Option<Pos<K, V>> {
        let (front, back) = self.range?;
        let last = front.node == back.node && front.idx == back.idx;
        self.range = if last { None } else { successor(front).map(|front| (front, back)) };
        Some(front)
    }

    unsafe fn next_back(&mut self) -> Option<Pos<K, V>> {
        let (front, back) = self.range?;
        let last = front.node == back.node && front.idx == back.idx;
        self.range = if last { None } else { predecessor(back).map(|back| (front, back)) };
        Some(back)
    }
}

impl<K, V, A: RawAlloc> BTreeMap<K, V, A> {
    pub(crate) fn raw(&self) -> Raw<K, V> {
        Raw::new(self.first_pos(), self.last_pos())
    }
}

pub struct Range<'a, K, V> {
    raw: Raw<K, V>,
    _marker: PhantomData<&'a (K, V)>,
}

impl<K, V> Range<'_, K, V> {
    pub(crate) fn new(raw: Raw<K, V>) -> Self {
        Self { raw, _marker: PhantomData }
    }
}

// SAFETY: behaves like `&BTreeMap`
unsafe impl<K: Sync, V: Sync> Send for Range<'_, K, V> {}
unsafe impl<K: Sync, V: Sync> Sync for Range<'_, K, V> {}

impl<'a, K, V> Iterator for Range<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY: the map is borrowed for `'a`, so `raw` stays valid
        unsafe { self.raw.next().map(|pos| pos.kv()) }
    }
}

impl<K, V> DoubleEndedIterator for Range<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        // SAFETY: as in `next`
        unsafe { self.raw.next_back().map(|pos| pos.kv()) }
    }
}

pub struct RangeMut<'a, K, V> {
    raw: Raw<K, V>,
    _marker: PhantomData<&'a mut (K, V)>,
}

impl<K, V> RangeMut<'_, K, V> {
    pub(crate) fn new(raw: Raw<K, V>) -> Self {
        Self { raw, _marker: PhantomData }
    }
}

// SAFETY: behaves like `&mut BTreeMap` that hands out `&K` and `&mut V`
unsafe impl<K: Sync, V: Send> Send for RangeMut<'_, K, V> {}
unsafe impl<K: Sync, V: Sync> Sync for RangeMut<'_, K, V> {}

impl<'a, K, V> Iterator for RangeMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY: the map is borrowed mutably for `'a`; each pair is yielded once
        unsafe { self.raw.next().map(|pos| pos.kv_mut()) }
    }
}

impl<K, V> DoubleEndedIterator for RangeMut<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        // SAFETY: as in `next`
        unsafe { self.raw.next_back().map(|pos| pos.kv_mut()) }
    }
}

pub struct Iter<'a, K, V> {
    range: Range<'a, K, V>,
    remaining: usize,
}

impl<'a, K, V> Iter<'a, K, V> {
    pub(crate) fn new<A: RawAlloc>(map: &'a BTreeMap<K, V, A>) -> Self {
        Self { range: Range::new(map.raw()), remaining: map.len }
    }
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let kv = self.range.next()?;
        self.remaining -= 1;
        Some(kv)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> DoubleEndedIterator for Iter<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let kv = self.range.next_back()?;
        self.remaining -= 1;
        Some(kv)
    }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}

pub struct IterMut<'a, K, V> {
    range: RangeMut<'a, K, V>,
    remaining: usize,
}

impl<'a, K, V> IterMut<'a, K, V> {
    pub(crate) fn new<A: RawAlloc>(map: &'a mut BTreeMap<K, V, A>) -> Self {
        Self { range: RangeMut::new(map.raw()), remaining: map.len }
    }
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        let kv = self.range.next()?;
        self.remaining -= 1;
        Some(kv)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> DoubleEndedIterator for IterMut<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let kv = self.range.next_back()?;
        self.remaining -= 1;
        Some(kv)
    }
}

impl<K, V> ExactSizeIterator for IterMut<'_, K, V> {}

pub struct Keys<'a, K, V>(pub(crate) Iter<'a, K, V>);

impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> DoubleEndedIterator for Keys<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, _)| k)
    }
}

impl<K, V> ExactSizeIterator for Keys<'_, K, V> {}

pub struct Values<'a, K, V>(pub(crate) Iter<'a, K, V>);

impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> DoubleEndedIterator for Values<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(_, v)| v)
    }
}

impl<K, V> ExactSizeIterator for Values<'_, K, V> {}

pub struct ValuesMut<'a, K, V>(pub(crate) IterMut<'a, K, V>);

impl<'a, K, V> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> DoubleEndedIterator for ValuesMut<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(_, v)| v)
    }
}

impl<K, V> ExactSizeIterator for ValuesMut<'_, K, V> {}

/// Owning iterator; pops pairs off the map, so it never allocates.
pub struct IntoIter<K, V, A: RawAlloc = Global>(pub(crate) BTreeMap<K, V, A>);

impl<K, V, A: RawAlloc> Iterator for IntoIter<K, V, A> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_first()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len, Some(self.0.len))
    }
}

impl<K, V, A: RawAlloc> DoubleEndedIterator for IntoIter<K, V, A> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.pop_last()
    }
}

impl<K, V, A: RawAlloc> ExactSizeIterator for IntoIter<K, V, A> {}

impl<'a, K, V, A: RawAlloc> IntoIterator for &'a BTreeMap<K, V, A> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        Iter::new(self)
    }
}

impl<'a, K, V, A: RawAlloc> IntoIterator for &'a mut BTreeMap<K, V, A> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IterMut::new(self)
    }
}

impl<K, V, A: RawAlloc> IntoIterator for BTreeMap<K, V, A> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V, A>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter(self)
    }
}
