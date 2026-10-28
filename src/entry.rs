//! Entry API. Only `VacantEntry::try_insert` (and the `or_try_*` helpers) allocate.

use crate::iter::Pos;
use crate::node::NodePtr;
use crate::{AllocError, BTreeMap, Global, RawAlloc};

/// A view into a single key, either present or absent; created by [`BTreeMap::entry`].
pub enum Entry<'a, K, V, A: RawAlloc = Global> {
    /// The key is not in the map.
    Vacant(VacantEntry<'a, K, V, A>),
    /// The key is in the map.
    Occupied(OccupiedEntry<'a, K, V, A>),
}

/// A key that is not yet in the map.
pub struct VacantEntry<'a, K, V, A: RawAlloc = Global> {
    pub(crate) key: K,
    // leaf edge where `key` goes; null for an empty map
    pub(crate) leaf: NodePtr<K, V>,
    pub(crate) idx: usize,
    pub(crate) map: &'a mut BTreeMap<K, V, A>,
}

/// A key that is already in the map.
pub struct OccupiedEntry<'a, K, V, A: RawAlloc = Global> {
    pub(crate) pos: Pos<K, V>,
    pub(crate) map: &'a mut BTreeMap<K, V, A>,
}

impl<'a, K: Ord, V, A: RawAlloc> Entry<'a, K, V, A> {
    /// Returns the key this entry refers to.
    pub fn key(&self) -> &K {
        match self {
            Entry::Vacant(entry) => entry.key(),
            Entry::Occupied(entry) => entry.key(),
        }
    }

    /// Returns a mutable reference to the value, inserting `default` if the key is absent.
    pub fn or_try_insert(self, default: V) -> Result<&'a mut V, AllocError> {
        self.or_try_insert_with(|| default)
    }

    /// Like [`or_try_insert`](Self::or_try_insert), but builds the default lazily.
    ///
    /// `default` is called only for a vacant entry.
    pub fn or_try_insert_with<F: FnOnce() -> V>(self, default: F) -> Result<&'a mut V, AllocError> {
        match self {
            Entry::Vacant(entry) => entry.try_insert(default()),
            Entry::Occupied(entry) => Ok(entry.into_mut()),
        }
    }

    /// Applies `f` to the value if the entry is occupied.
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, f: F) -> Self {
        if let Entry::Occupied(entry) = &mut self {
            f(entry.get_mut());
        }
        self
    }
}

impl<'a, K: Ord, V, A: RawAlloc> VacantEntry<'a, K, V, A> {
    /// Returns the key.
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Consumes the entry and returns the key.
    pub fn into_key(self) -> K {
        self.key
    }

    /// Inserts `value`. On `Err` the map is unchanged and the key and value are dropped.
    pub fn try_insert(self, value: V) -> Result<&'a mut V, AllocError> {
        let val = self.map.insert_at(self.leaf, self.idx, self.key, value).map_err(|_| AllocError)?;
        // SAFETY: `insert_at` returns the slot it just initialized; the map stays borrowed for `'a`
        Ok(unsafe { &mut *val })
    }
}

impl<'a, K: Ord, V, A: RawAlloc> OccupiedEntry<'a, K, V, A> {
    /// Returns the key.
    pub fn key(&self) -> &K {
        // SAFETY: `pos` is a live pair while the map is borrowed
        unsafe { self.pos.kv().0 }
    }

    /// Returns a reference to the value.
    pub fn get(&self) -> &V {
        // SAFETY: as in `key`
        unsafe { self.pos.kv().1 }
    }

    /// Returns a mutable reference to the value.
    pub fn get_mut(&mut self) -> &mut V {
        // SAFETY: as in `key`; `&mut self` makes it exclusive
        unsafe { self.pos.kv_mut().1 }
    }

    /// Consumes the entry and returns a mutable reference to the value.
    pub fn into_mut(self) -> &'a mut V {
        // SAFETY: as in `key`; the map stays borrowed for `'a`
        unsafe { self.pos.kv_mut().1 }
    }

    /// Replaces the value, returning the old one. Does not allocate.
    pub fn insert(&mut self, value: V) -> V {
        core::mem::replace(self.get_mut(), value)
    }

    /// Does not allocate.
    pub fn remove_entry(self) -> (K, V) {
        // SAFETY: `pos` is a live pair of `map`
        unsafe { self.map.remove_at(self.pos) }
    }

    /// Does not allocate.
    pub fn remove(self) -> V {
        self.remove_entry().1
    }
}

impl<K: Ord, V, A: RawAlloc> BTreeMap<K, V, A> {
    /// Finds the entry for `key`. Does not allocate.
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V, A> {
        match self.search(&key) {
            Ok(pos) => Entry::Occupied(OccupiedEntry { pos, map: self }),
            Err((leaf, idx)) => Entry::Vacant(VacantEntry { key, leaf, idx, map: self }),
        }
    }
}
