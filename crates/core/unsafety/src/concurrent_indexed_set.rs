use std::hash::BuildHasher;
use std::hash::Hash;

use rustc_hash::FxBuildHasher;

use crate::ConcurrentAppendVec;
use crate::ShardedHashMap;

/// An append-only set mapping each distinct value of type `T` to a stable
/// `usize` index.
///
/// The set is thread-safe: values can be inserted and looked up concurrently
/// through `&self` from multiple threads. To keep concurrent insertion
/// contention-free the indices are unique and stable but **not dense**, so walk
/// [`ConcurrentIndexedSet::iter`] rather than `0..len`. There is no removal; an
/// index stays valid for the lifetime of the set (until
/// [`ConcurrentIndexedSet::clear`], which requires `&mut self`).
pub struct ConcurrentIndexedSet<T, S = FxBuildHasher> {
    /// Stores each distinct value at the index handed out for it. Indices are
    /// stable but may be sparse.
    values: ConcurrentAppendVec<T>,
    /// Hash index from a value's hash to its index in `values`. Stores only the
    /// index; the hash and equality closures dereference `values`, so the
    /// payload is not duplicated.
    table: ShardedHashMap<usize, S>,
}

impl<T, S: Default> ConcurrentIndexedSet<T, S> {
    /// Creates a new empty set with the default hasher.
    pub fn new() -> ConcurrentIndexedSet<T, S> {
        ConcurrentIndexedSet {
            values: ConcurrentAppendVec::new(),
            table: ShardedHashMap::with_hasher(S::default()),
        }
    }

    /// Creates a new empty set whose hash index has room for at least
    /// `capacity` values before reallocating.
    pub fn with_capacity(capacity: usize) -> ConcurrentIndexedSet<T, S> {
        ConcurrentIndexedSet {
            values: ConcurrentAppendVec::new(),
            table: ShardedHashMap::with_capacity_and_hasher(capacity, S::default()),
        }
    }
}

impl<T, S> ConcurrentIndexedSet<T, S>
where
    T: Hash + Eq + Clone,
    S: BuildHasher,
{
    /// Inserts `value` and returns its index together with a boolean that is
    /// true when the value was newly inserted and false when it was already
    /// present.
    pub fn insert(&self, value: T) -> (usize, bool) {
        let hash = self.table.hash(&value);
        // SAFETY (this and the hash closure below): the table only ever holds
        // indices returned by `push`, and every read here runs under the shard
        // lock that ordered that push's publish, so the slot is initialized and
        // visible — `get_unchecked` is sound.
        let eq = |&index: &usize| unsafe { self.values.get_unchecked(index) } == &value;

        // Fast path: the value is already present, so no write lock is needed.
        if let Some(index) = self.table.find(hash, eq) {
            return (index, false);
        }

        // The vacant branch runs while the shard for `hash` is write-locked, so
        // two threads inserting the same new value cannot both push and hand
        // out duplicate indices. `push` publishes the slot before returning, so
        // recording its index in the table afterwards guarantees a returned
        // index always resolves in `get_by_index`.
        self.table.find_or_insert_with(
            hash,
            eq,
            |&index| self.table.hash(unsafe { self.values.get_unchecked(index) }),
            || self.values.push(value.clone()),
        )
    }

    /// Returns the index of `value` if it is present, or `None` otherwise.
    pub fn index(&self, value: &T) -> Option<usize> {
        let hash = self.table.hash(value);
        // SAFETY: indices in the table were returned by `push` and published
        // under the shard lock this read takes, so `get_unchecked` is sound.
        self.table
            .find(hash, |&index| unsafe { self.values.get_unchecked(index) } == value)
    }

    /// Returns true if `value` is present in the set.
    pub fn contains(&self, value: &T) -> bool {
        self.index(value).is_some()
    }
}

impl<T, S> ConcurrentIndexedSet<T, S> {
    /// Returns a reference to the value at `index`, or `None` if the index was
    /// never handed out (out of range or a sparse gap).
    pub fn get_by_index(&self, index: usize) -> Option<&T> {
        self.values.get(index)
    }

    /// Returns an iterator over the resident values. Order is unspecified, and
    /// indices are not exposed because they may be sparse.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.values.iter()
    }

    /// Returns the number of distinct values in the set.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Removes all values, invalidating every previously returned index.
    pub fn clear(&mut self) {
        self.values.clear();
        self.table.clear();
    }
}

impl<T, S: Default> Default for ConcurrentIndexedSet<T, S> {
    fn default() -> ConcurrentIndexedSet<T, S> {
        ConcurrentIndexedSet::new()
    }
}
