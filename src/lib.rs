//! Ordered map that never panics or aborts on allocation failure.
//!
//! - [`BTreeMap::try_insert`] is atomic: on [`AllocError`] the map is unchanged.
//! - [`BTreeMap::remove`] never allocates.
//! - The crate builds with `alloc` compiled under `--cfg no_global_oom_handling`.
//! - Nodes come from a [`RawAlloc`] per map ([`BTreeMap::new_in`]); [`Global`] by default.
//!
//! A B-tree like `std::collections::BTreeMap`; all algorithms are iterative (small kernel
//! stacks). See `docs/DESIGN.md`.
#![no_std]
#![deny(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

extern crate alloc;

mod entry;
mod iter;
mod node;
pub mod set;

use core::alloc::Layout;
use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Bound, RangeBounds};
use core::ptr;

pub use entry::{Entry, OccupiedEntry, VacantEntry};
use iter::{first_leaf, last_leaf, next_kv, prev_kv, successor, Pos};
pub use iter::{IntoIter, Iter, IterMut, Keys, Range, RangeMut, Values, ValuesMut};
use node::{NodePtr, B, CAPACITY, MIN_LEN};
pub use set::BTreeSet;

/// Allocation failed; the map was not modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocError;

impl fmt::Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("memory allocation failed")
    }
}

impl core::error::Error for AllocError {}

/// Allocation failed; the map was not modified and the pair is handed back.
pub struct InsertError<K, V> {
    pub key: K,
    pub value: V,
}

impl<K, V> fmt::Debug for InsertError<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InsertError").finish_non_exhaustive()
    }
}

impl<K, V> fmt::Display for InsertError<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&AllocError, f)
    }
}

impl<K, V> core::error::Error for InsertError<K, V> {}

/// Source of node memory, e.g. a kernel pool with its own tag and pool type.
///
/// # Safety
///
/// `alloc` returns null or a block valid for `layout` until it is passed to `dealloc` of the
/// same allocator. Neither method may panic.
pub unsafe trait RawAlloc {
    /// Returns null on failure. The map never asks for a zero-sized layout.
    fn alloc(&self, layout: Layout) -> *mut u8;

    /// # Safety
    ///
    /// `ptr` came from `alloc` of this allocator with the same `layout`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout);
}

/// The global allocator (`#[global_allocator]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Global;

// SAFETY: forwards to the global allocator
unsafe impl RawAlloc for Global {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return ptr::null_mut();
        }
        // SAFETY: `layout` is not zero-sized
        unsafe { alloc::alloc::alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        alloc::alloc::dealloc(ptr, layout);
    }
}

// SAFETY: forwards to `A`
unsafe impl<A: RawAlloc + ?Sized> RawAlloc for &A {
    fn alloc(&self, layout: Layout) -> *mut u8 {
        (**self).alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        (**self).dealloc(ptr, layout);
    }
}

pub struct BTreeMap<K, V, A: RawAlloc = Global> {
    // null when empty; `*const` keeps `K`/`V` covariant
    root: *const node::LeafNode<K, V>,
    height: usize,
    len: usize,
    alloc: A,
    _marker: PhantomData<(K, V)>,
}

// SAFETY: the map owns its nodes exclusively, like a `Box`
unsafe impl<K: Send, V: Send, A: RawAlloc + Send> Send for BTreeMap<K, V, A> {}
unsafe impl<K: Sync, V: Sync, A: RawAlloc + Sync> Sync for BTreeMap<K, V, A> {}

impl<K: Ord, V> BTreeMap<K, V> {
    /// Creates an empty map. Does not allocate.
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<K: Ord, V, A: RawAlloc> BTreeMap<K, V, A> {
    /// Creates an empty map whose nodes come from `alloc`. Does not allocate.
    pub const fn new_in(alloc: A) -> Self {
        Self { root: ptr::null(), height: 0, len: 0, alloc, _marker: PhantomData }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get<Q: ?Sized + Ord>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let pos = self.search(key).ok()?;
        // SAFETY: `search` returns a live pair
        Some(unsafe { pos.kv().1 })
    }

    pub fn get_mut<Q: ?Sized + Ord>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        let pos = self.search(key).ok()?;
        // SAFETY: `search` returns a live pair; `&mut self` makes it exclusive
        Some(unsafe { pos.kv_mut().1 })
    }

    pub fn contains_key<Q: ?Sized + Ord>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.search(key).is_ok()
    }

    /// Inserts `key`/`value`, returning the previous value for `key`.
    ///
    /// Replacing an existing key does not allocate. On `Err` the map is unchanged
    /// and `key`/`value` are dropped.
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, AllocError> {
        self.try_insert_returning(key, value).map_err(|_| AllocError)
    }

    /// Like [`try_insert`](Self::try_insert), but on `Err` hands `key`/`value` back.
    pub fn try_insert_returning(&mut self, key: K, value: V) -> Result<Option<V>, InsertError<K, V>> {
        let (leaf, idx) = match self.search(&key) {
            // SAFETY: `search` returns a live pair
            Ok(pos) => return Ok(Some(unsafe { ptr::replace(node::val(pos.node.cast_mut(), pos.idx), value) })),
            Err(edge) => edge,
        };
        match self.insert_at(leaf, idx, key, value) {
            Ok(_) => Ok(None),
            Err((key, value)) => Err(InsertError { key, value }),
        }
    }

    /// Removes `key`, returning its value. Never allocates.
    pub fn remove<Q: ?Sized + Ord>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        let pos = self.search(key).ok()?;
        // SAFETY: `search` returns a live pair
        Some(unsafe { self.remove_at(pos) }.1)
    }

    /// Removes all entries and frees the memory.
    pub fn clear(&mut self) {
        self.free_all();
    }

    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        // SAFETY: a live pair, borrowed with `self`
        self.first_pos().map(|pos| unsafe { pos.kv() })
    }

    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        // SAFETY: as above
        self.last_pos().map(|pos| unsafe { pos.kv() })
    }

    /// Iterates over the pairs in `range`. An empty or inverted range yields nothing
    /// (std panics on `start > end`).
    pub fn range<Q: ?Sized + Ord, R: RangeBounds<Q>>(&self, range: R) -> Range<'_, K, V>
    where
        K: Borrow<Q>,
    {
        Range::new(self.range_raw(range))
    }

    pub fn range_mut<Q: ?Sized + Ord, R: RangeBounds<Q>>(&mut self, range: R) -> RangeMut<'_, K, V>
    where
        K: Borrow<Q>,
    {
        RangeMut::new(self.range_raw(range))
    }

    /// Keeps only the pairs for which `f` returns `true`. Never allocates.
    pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, mut f: F) {
        let mut pos = self.first_pos();
        while let Some(p) = pos {
            // SAFETY: `p` is a live pair; after a removal the walk resumes from a fresh search
            unsafe {
                let (k, v) = p.kv_mut();
                if f(k, v) {
                    pos = successor(p);
                    continue;
                }
                let (key, _value) = self.remove_at(p);
                pos = self.bound_pos(Bound::Excluded(&key), false);
            }
        }
    }

    /// Clones the map. On `Err` the partial copy is freed; `self` is untouched.
    pub fn try_clone(&self) -> Result<Self, AllocError>
    where
        K: Clone,
        V: Clone,
        A: Clone,
    {
        let mut map = Self::new_in(self.alloc.clone());
        for (k, v) in self {
            map.try_insert(k.clone(), v.clone())?;
        }
        Ok(map)
    }

    /// Panics if the tree structure is broken. For tests.
    #[cfg(feature = "debug-invariants")]
    pub fn check_invariants(&self) {
        if self.root.is_null() {
            assert!(self.len == 0 && self.height == 0);
            return;
        }
        let mut count = 0;
        // SAFETY: pre-order walk over live nodes
        unsafe {
            assert!((*self.root).parent.is_null());
            let (mut node, mut height) = (self.root.cast_mut(), self.height);
            'walk: loop {
                let len = node::len(node);
                assert!(len > 0 && len <= CAPACITY && (len >= MIN_LEN || node.cast_const() == self.root));
                count += len;
                if height > 0 {
                    for idx in 0..=len {
                        let child = node::child(node, idx);
                        assert!(node::parent(child) == node && node::parent_idx(child) == idx);
                    }
                    node = node::child(node, 0);
                    height -= 1;
                    continue;
                }
                // leaf: go to the next unvisited subtree
                loop {
                    let parent = node::parent(node);
                    if parent.is_null() {
                        break 'walk;
                    }
                    let next = node::parent_idx(node) + 1;
                    if next <= node::len(parent) {
                        node = node::child(parent, next);
                        break;
                    }
                    node = parent;
                    height += 1;
                }
            }
        }
        assert_eq!(count, self.len);
        assert!(self.iter().zip(self.iter().skip(1)).all(|((a, _), (b, _))| a < b));
    }

    /// Finds `key`: `Ok` with its pair, or `Err((leaf, idx))` with the edge where it would be
    /// inserted (null leaf for an empty map).
    pub(crate) fn search<Q: ?Sized + Ord>(&self, key: &Q) -> Result<Pos<K, V>, (NodePtr<K, V>, usize)>
    where
        K: Borrow<Q>,
    {
        let (mut node, mut height) = (self.root.cast_mut(), self.height);
        if node.is_null() {
            return Err((node, 0));
        }
        loop {
            // SAFETY: `node` is live at `height`; `idx` stays within `0..=len`
            unsafe {
                let len = node::len(node);
                let mut idx = 0;
                while idx < len {
                    match key.cmp((*node::key(node, idx)).borrow()) {
                        Ordering::Greater => idx += 1,
                        Ordering::Equal => return Ok(Pos { node, idx, height }),
                        Ordering::Less => break,
                    }
                }
                if height == 0 {
                    return Err((node, idx));
                }
                node = node::child(node, idx);
                height -= 1;
            }
        }
    }

    /// Inserts at leaf edge `leaf`/`idx` (null leaf: empty map) and returns the value's slot.
    /// On allocation failure the map is unchanged and `key`/`value` are handed back.
    pub(crate) fn insert_at(&mut self, leaf: NodePtr<K, V>, idx: usize, key: K, value: V) -> Result<*mut V, (K, V)> {
        let slot = if leaf.is_null() {
            let root = node::new_node(&self.alloc, false);
            if root.is_null() {
                return Err((key, value));
            }
            // SAFETY: `root` is a fresh empty leaf
            unsafe {
                node::insert_fit(root, 0, key, value, ptr::null_mut());
                self.root = root;
                node::val(root, 0)
            }
        } else {
            // SAFETY: `leaf`/`idx` is a leaf edge of this tree; every node the splits need
            // is allocated before anything is touched
            unsafe {
                let Ok((spare_leaf, spare_internals)) = reserve(&self.alloc, leaf) else {
                    return Err((key, value));
                };
                self.insert_split(leaf, idx, key, value, spare_leaf, spare_internals)
            }
        };
        self.len += 1;
        Ok(slot)
    }

    /// Pairs within `range`; empty if the bounds cross.
    fn range_raw<Q: ?Sized + Ord, R: RangeBounds<Q>>(&self, range: R) -> iter::Raw<K, V>
    where
        K: Borrow<Q>,
    {
        let front = self.bound_pos(range.start_bound(), false);
        let back = self.bound_pos(range.end_bound(), true);
        match (front, back) {
            // SAFETY: both are live pairs
            (Some(f), Some(b)) if unsafe { *f.kv().0 <= *b.kv().0 } => iter::Raw::new(front, back),
            _ => iter::Raw::new(None, None),
        }
    }

    /// The first pair after a lower `bound`, or the last pair before an upper one.
    fn bound_pos<Q: ?Sized + Ord>(&self, bound: Bound<&Q>, upper: bool) -> Option<Pos<K, V>>
    where
        K: Borrow<Q>,
    {
        let (mut node, mut height) = (self.root.cast_mut(), self.height);
        if node.is_null() {
            return None;
        }
        loop {
            // SAFETY: `node` is live at `height`; `idx` stays within `0..=len`
            unsafe {
                let len = node::len(node);
                let mut idx = 0;
                while idx < len && before((*node::key(node, idx)).borrow(), bound, upper) {
                    idx += 1;
                }
                if height == 0 {
                    return if upper { prev_kv(node, idx) } else { next_kv(node, idx) };
                }
                node = node::child(node, idx);
                height -= 1;
            }
        }
    }

    /// Inserts into `leaf` at `idx`, splitting full nodes upwards with the reserved nodes,
    /// and returns the value's slot. Cannot fail and calls no user code.
    unsafe fn insert_split(
        &mut self,
        mut node: NodePtr<K, V>,
        mut idx: usize,
        mut key: K,
        mut value: V,
        spare_leaf: NodePtr<K, V>,
        mut spare_internals: NodePtr<K, V>,
    ) -> *mut V {
        let mut edge: NodePtr<K, V> = ptr::null_mut();
        // leaves never move, so the slot found at the leaf level stays valid
        let mut slot: *mut V = ptr::null_mut();
        loop {
            if node::len(node) < CAPACITY {
                node::insert_fit(node, idx, key, value, edge);
                return if slot.is_null() { node::val(node, idx) } else { slot };
            }
            let internal = !edge.is_null();
            let right = if internal { pop(&mut spare_internals) } else { spare_leaf };
            // both halves keep at least MIN_LEN pairs after the insert
            let (mid, into_right, at) = if idx < B - 1 {
                (B - 2, false, idx)
            } else if idx == B - 1 {
                (B - 1, false, idx)
            } else if idx == B {
                (B - 1, true, 0)
            } else {
                (B, true, idx - B - 1)
            };
            let (mid_key, mid_value) = node::split(node, mid, right, internal);
            let target = if into_right { right } else { node };
            node::insert_fit(target, at, key, value, edge);
            if slot.is_null() {
                slot = node::val(target, at);
            }
            let parent = node::parent(node);
            if parent.is_null() {
                let root = pop(&mut spare_internals);
                node::set_child(root, 0, node);
                node::insert_fit(root, 0, mid_key, mid_value, right);
                self.root = root;
                self.height += 1;
                return slot;
            }
            idx = node::parent_idx(node);
            node = parent;
            (key, value, edge) = (mid_key, mid_value, right);
        }
    }
}

/// Allocates every node an insert into `leaf` needs: a leaf if it is full, an internal node
/// per full ancestor that splits, and one for a new root if all of them are full.
/// The internal nodes are chained through `parent`. On failure frees them all.
#[allow(clippy::type_complexity)]
unsafe fn reserve<K, V, A: RawAlloc>(alloc: &A, leaf: NodePtr<K, V>) -> Result<(NodePtr<K, V>, NodePtr<K, V>), AllocError> {
    let mut internals: NodePtr<K, V> = ptr::null_mut();
    if node::len(leaf) < CAPACITY {
        return Ok((ptr::null_mut(), internals));
    }
    let spare_leaf = node::new_node(alloc, false);
    if spare_leaf.is_null() {
        return Err(AllocError);
    }
    // `node` is full, so it splits and pushes a pair up
    let mut node = leaf;
    loop {
        let parent = node::parent(node);
        if !parent.is_null() && node::len(parent) < CAPACITY {
            return Ok((spare_leaf, internals));
        }
        let spare = node::new_node(alloc, true);
        if spare.is_null() {
            while !internals.is_null() {
                node::free_node(alloc, pop(&mut internals), true);
            }
            node::free_node(alloc, spare_leaf, false);
            return Err(AllocError);
        }
        (*spare).parent = internals;
        internals = spare;
        if parent.is_null() {
            return Ok((spare_leaf, internals));
        }
        node = parent;
    }
}

/// Whether `key` lies before a lower `bound` (`upper == false`) or not after an upper one.
fn before<Q: ?Sized + Ord>(key: &Q, bound: Bound<&Q>, upper: bool) -> bool {
    match bound {
        Bound::Included(b) if upper => key <= b,
        Bound::Included(b) => key < b,
        Bound::Excluded(b) if upper => key < b,
        Bound::Excluded(b) => key <= b,
        Bound::Unbounded => upper,
    }
}

unsafe fn pop<K, V>(chain: &mut NodePtr<K, V>) -> NodePtr<K, V> {
    let node = *chain;
    debug_assert!(!node.is_null());
    *chain = node::parent(node);
    (*node).parent = ptr::null();
    node
}

impl<K, V, A: RawAlloc> BTreeMap<K, V, A> {
    /// Iterates in ascending key order.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter::new(self)
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        IterMut::new(self)
    }

    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys(self.iter())
    }

    pub fn values(&self) -> Values<'_, K, V> {
        Values(self.iter())
    }

    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
        ValuesMut(self.iter_mut())
    }

    /// Never allocates.
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let pos = self.first_pos()?;
        // SAFETY: a live pair
        Some(unsafe { self.remove_at(pos) })
    }

    /// Never allocates.
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        let pos = self.last_pos()?;
        // SAFETY: a live pair
        Some(unsafe { self.remove_at(pos) })
    }

    pub(crate) fn first_pos(&self) -> Option<Pos<K, V>> {
        if self.root.is_null() {
            return None;
        }
        // SAFETY: descends live nodes
        unsafe { next_kv(first_leaf(self.root.cast_mut(), self.height), 0) }
    }

    pub(crate) fn last_pos(&self) -> Option<Pos<K, V>> {
        if self.root.is_null() {
            return None;
        }
        // SAFETY: descends live nodes
        unsafe {
            let leaf = last_leaf(self.root.cast_mut(), self.height);
            prev_kv(leaf, node::len(leaf))
        }
    }

    /// Removes the pair at `pos`. Never allocates and calls no user code.
    pub(crate) unsafe fn remove_at(&mut self, pos: Pos<K, V>) -> (K, V) {
        let Pos { node, idx, height } = pos;
        let node = node.cast_mut();
        // a non-root leaf is never empty
        let (leaf, kv) = if height == 0 {
            (node, node::remove_leaf_kv(node, idx))
        } else {
            // swap with the predecessor: the last pair of the left subtree
            let leaf = last_leaf(node::child(node, idx), height - 1);
            let (k, v) = node::remove_leaf_kv(leaf, node::len(leaf) - 1);
            (leaf, (ptr::replace(node::key(node, idx), k), ptr::replace(node::val(node, idx), v)))
        };
        self.rebalance(leaf);
        self.len -= 1;
        kv
    }

    /// Restores minimum occupancy from `node` (a leaf that just lost a pair) upwards.
    unsafe fn rebalance(&mut self, mut node: NodePtr<K, V>) {
        let mut height = 0;
        loop {
            let len = node::len(node);
            let parent = node::parent(node);
            if parent.is_null() {
                if len == 0 {
                    if height == 0 {
                        self.root = ptr::null();
                    } else {
                        let child = node::child(node, 0);
                        (*child).parent = ptr::null();
                        self.root = child;
                        self.height -= 1;
                    }
                    node::free_node(&self.alloc, node, height > 0);
                }
                return;
            }
            if len >= MIN_LEN {
                return;
            }
            let idx = node::parent_idx(node);
            let internal = height > 0;
            if idx > 0 {
                if node::len(node::child(parent, idx - 1)) > MIN_LEN {
                    return node::steal_left(parent, idx, internal);
                }
                node::merge(&self.alloc, parent, idx - 1, internal);
            } else {
                if node::len(node::child(parent, 1)) > MIN_LEN {
                    return node::steal_right(parent, 0, internal);
                }
                node::merge(&self.alloc, parent, 0, internal);
            }
            node = parent;
            height += 1;
        }
    }

    /// Drops all pairs and frees all nodes. The map is detached from the tree first, so a
    /// panicking `Drop` of a key or value leaks the rest instead of freeing it twice.
    fn free_all(&mut self) {
        let mut node = core::mem::replace(&mut self.root, ptr::null()).cast_mut();
        let tree_height = core::mem::replace(&mut self.height, 0);
        self.len = 0;
        if node.is_null() {
            return;
        }
        // SAFETY: visits every pair once in order and frees each node after its last edge
        unsafe {
            for _ in 0..tree_height {
                node = node::child(node, 0);
            }
            let (mut idx, mut height) = (0, 0);
            loop {
                while idx >= node::len(node) {
                    let (parent, parent_idx) = (node::parent(node), node::parent_idx(node));
                    node::free_node(&self.alloc, node, height > 0);
                    if parent.is_null() {
                        return;
                    }
                    (node, idx, height) = (parent, parent_idx, height + 1);
                }
                ptr::drop_in_place(node::key(node, idx));
                ptr::drop_in_place(node::val(node, idx));
                idx += 1;
                if height > 0 {
                    node = node::child(node, idx);
                    for _ in 1..height {
                        node = node::child(node, 0);
                    }
                    (idx, height) = (0, 0);
                }
            }
        }
    }
}

impl<K, V, A: RawAlloc> Drop for BTreeMap<K, V, A> {
    fn drop(&mut self) {
        self.free_all();
    }
}

impl<K: Ord, V> Default for BTreeMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: fmt::Debug, V: fmt::Debug, A: RawAlloc> fmt::Debug for BTreeMap<K, V, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn insert_replace_remove() {
        let mut m = BTreeMap::new();
        assert_eq!(m.try_insert(2, "b"), Ok(None));
        assert_eq!(m.try_insert(1, "a"), Ok(None));
        assert_eq!(m.try_insert(2, "B"), Ok(Some("b")));
        assert_eq!(m.len(), 2);
        assert_eq!(m.get(&2), Some(&"B"));
        assert_eq!(m.iter().map(|(k, _)| *k).collect::<std::vec::Vec<_>>(), [1, 2]);
        assert_eq!(m.remove(&1), Some("a"));
        assert_eq!(m.remove(&1), None);
        assert_eq!(m.first_key_value(), Some((&2, &"B")));
    }
}
