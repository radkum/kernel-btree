//! B-tree nodes: raw layout and node-local operations.
//!
//! Everything takes raw node pointers. Callers keep them live and indices within `len`
//! (pairs) or `len + 1` (edges). Nothing here allocates except `new_node` or calls user code.

use core::alloc::Layout;
use core::mem::MaybeUninit;
use core::ptr::{self, addr_of_mut};

use crate::RawAlloc;

pub(crate) const B: usize = 6;
pub(crate) const CAPACITY: usize = 2 * B - 1;
pub(crate) const MIN_LEN: usize = B - 1;

pub(crate) type NodePtr<K, V> = *mut LeafNode<K, V>;

#[repr(C)]
pub(crate) struct LeafNode<K, V> {
    // always an `InternalNode`; null for the root. `*const` keeps `K`/`V` covariant
    pub(crate) parent: *const LeafNode<K, V>,
    pub(crate) parent_idx: u16,
    pub(crate) len: u16,
    keys: [MaybeUninit<K>; CAPACITY],
    vals: [MaybeUninit<V>; CAPACITY],
}

#[repr(C)]
struct InternalNode<K, V> {
    data: LeafNode<K, V>,
    edges: [MaybeUninit<NodePtr<K, V>>; CAPACITY + 1],
}

fn layout<K, V>(internal: bool) -> Layout {
    if internal {
        Layout::new::<InternalNode<K, V>>()
    } else {
        Layout::new::<LeafNode<K, V>>()
    }
}

/// Allocates an empty node with no parent; null on allocation failure.
pub(crate) fn new_node<K, V, A: RawAlloc>(alloc: &A, internal: bool) -> NodePtr<K, V> {
    // the layout is never zero-sized (it holds `parent`)
    let node = alloc.alloc(layout::<K, V>(internal)).cast::<LeafNode<K, V>>();
    if !node.is_null() {
        // SAFETY: freshly allocated; only the header needs initializing
        unsafe {
            addr_of_mut!((*node).parent).write(ptr::null());
            addr_of_mut!((*node).parent_idx).write(0);
            addr_of_mut!((*node).len).write(0);
        }
    }
    node
}

pub(crate) unsafe fn free_node<K, V, A: RawAlloc>(alloc: &A, node: NodePtr<K, V>, internal: bool) {
    alloc.dealloc(node.cast(), layout::<K, V>(internal));
}

pub(crate) unsafe fn len<K, V>(node: NodePtr<K, V>) -> usize {
    usize::from((*node).len)
}

unsafe fn set_len<K, V>(node: NodePtr<K, V>, len: usize) {
    (*node).len = len as u16;
}

pub(crate) unsafe fn parent<K, V>(node: NodePtr<K, V>) -> NodePtr<K, V> {
    (*node).parent.cast_mut()
}

pub(crate) unsafe fn parent_idx<K, V>(node: NodePtr<K, V>) -> usize {
    usize::from((*node).parent_idx)
}

pub(crate) unsafe fn key<K, V>(node: NodePtr<K, V>, idx: usize) -> *mut K {
    addr_of_mut!((*node).keys).cast::<K>().add(idx)
}

pub(crate) unsafe fn val<K, V>(node: NodePtr<K, V>, idx: usize) -> *mut V {
    addr_of_mut!((*node).vals).cast::<V>().add(idx)
}

unsafe fn edge<K, V>(node: NodePtr<K, V>, idx: usize) -> *mut NodePtr<K, V> {
    addr_of_mut!((*node.cast::<InternalNode<K, V>>()).edges).cast::<NodePtr<K, V>>().add(idx)
}

pub(crate) unsafe fn child<K, V>(node: NodePtr<K, V>, idx: usize) -> NodePtr<K, V> {
    *edge(node, idx)
}

/// Makes `child` edge `idx` of `node`.
pub(crate) unsafe fn set_child<K, V>(node: NodePtr<K, V>, idx: usize, child: NodePtr<K, V>) {
    *edge(node, idx) = child;
    (*child).parent = node;
    (*child).parent_idx = idx as u16;
}

unsafe fn fix_children<K, V>(node: NodePtr<K, V>, from: usize, to: usize) {
    for idx in from..=to {
        set_child(node, idx, child(node, idx));
    }
}

/// Copies `count` pairs; the ranges may overlap.
unsafe fn move_kvs<K, V>(src: NodePtr<K, V>, src_idx: usize, dst: NodePtr<K, V>, dst_idx: usize, count: usize) {
    ptr::copy(key(src, src_idx), key(dst, dst_idx), count);
    ptr::copy(val(src, src_idx), val(dst, dst_idx), count);
}

unsafe fn move_edges<K, V>(src: NodePtr<K, V>, src_idx: usize, dst: NodePtr<K, V>, dst_idx: usize, count: usize) {
    ptr::copy(edge(src, src_idx), edge(dst, dst_idx), count);
}

/// Inserts `k`/`v` at `idx` of a non-full node and, unless `right` is null, `right` as the
/// edge after it.
pub(crate) unsafe fn insert_fit<K, V>(node: NodePtr<K, V>, idx: usize, k: K, v: V, right: NodePtr<K, V>) {
    let len = len(node);
    move_kvs(node, idx, node, idx + 1, len - idx);
    key(node, idx).write(k);
    val(node, idx).write(v);
    if !right.is_null() {
        move_edges(node, idx + 1, node, idx + 2, len - idx);
        *edge(node, idx + 1) = right;
        fix_children(node, idx + 1, len + 1);
    }
    set_len(node, len + 1);
}

/// Moves the pairs (and edges) right of `mid` into the empty node `right` and returns the
/// pair at `mid`, which leaves `node`.
pub(crate) unsafe fn split<K, V>(node: NodePtr<K, V>, mid: usize, right: NodePtr<K, V>, internal: bool) -> (K, V) {
    let right_len = len(node) - mid - 1;
    move_kvs(node, mid + 1, right, 0, right_len);
    if internal {
        move_edges(node, mid + 1, right, 0, right_len + 1);
        fix_children(right, 0, right_len);
    }
    set_len(right, right_len);
    set_len(node, mid);
    (key(node, mid).read(), val(node, mid).read())
}

pub(crate) unsafe fn remove_leaf_kv<K, V>(node: NodePtr<K, V>, idx: usize) -> (K, V) {
    let len = len(node);
    let kv = (key(node, idx).read(), val(node, idx).read());
    move_kvs(node, idx + 1, node, idx, len - idx - 1);
    set_len(node, len - 1);
    kv
}

/// Merges separator `idx` and child `idx + 1` of `parent` into child `idx`; frees child `idx + 1`.
pub(crate) unsafe fn merge<K, V, A: RawAlloc>(alloc: &A, parent: NodePtr<K, V>, idx: usize, internal: bool) {
    let left = child(parent, idx);
    let right = child(parent, idx + 1);
    let (left_len, right_len, parent_len) = (len(left), len(right), len(parent));
    key(left, left_len).write(key(parent, idx).read());
    val(left, left_len).write(val(parent, idx).read());
    move_kvs(right, 0, left, left_len + 1, right_len);
    if internal {
        move_edges(right, 0, left, left_len + 1, right_len + 1);
        fix_children(left, left_len + 1, left_len + 1 + right_len);
    }
    set_len(left, left_len + 1 + right_len);
    move_kvs(parent, idx + 1, parent, idx, parent_len - idx - 1);
    move_edges(parent, idx + 2, parent, idx + 1, parent_len - idx - 1);
    fix_children(parent, idx + 1, parent_len - 1);
    set_len(parent, parent_len - 1);
    free_node(alloc, right, internal);
}

/// Rotates the last pair of child `idx - 1` through separator `idx - 1` into child `idx`.
pub(crate) unsafe fn steal_left<K, V>(parent: NodePtr<K, V>, idx: usize, internal: bool) {
    let left = child(parent, idx - 1);
    let node = child(parent, idx);
    let (left_len, node_len) = (len(left), len(node));
    move_kvs(node, 0, node, 1, node_len);
    key(node, 0).write(ptr::replace(key(parent, idx - 1), key(left, left_len - 1).read()));
    val(node, 0).write(ptr::replace(val(parent, idx - 1), val(left, left_len - 1).read()));
    if internal {
        move_edges(node, 0, node, 1, node_len + 1);
        *edge(node, 0) = child(left, left_len);
        fix_children(node, 0, node_len + 1);
    }
    set_len(left, left_len - 1);
    set_len(node, node_len + 1);
}

/// Rotates the first pair of child `idx + 1` through separator `idx` into child `idx`.
pub(crate) unsafe fn steal_right<K, V>(parent: NodePtr<K, V>, idx: usize, internal: bool) {
    let node = child(parent, idx);
    let right = child(parent, idx + 1);
    let (node_len, right_len) = (len(node), len(right));
    key(node, node_len).write(ptr::replace(key(parent, idx), key(right, 0).read()));
    val(node, node_len).write(ptr::replace(val(parent, idx), val(right, 0).read()));
    move_kvs(right, 1, right, 0, right_len - 1);
    if internal {
        set_child(node, node_len + 1, child(right, 0));
        move_edges(right, 1, right, 0, right_len);
        fix_children(right, 0, right_len - 1);
    }
    set_len(node, node_len + 1);
    set_len(right, right_len - 1);
}
