use core::panic;
use std::fmt;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::ops::Deref;
use std::ops::Index;
use std::ops::IndexMut;

use hashbrown::Equivalent;
use hashbrown::HashTable;
use rustc_hash::FxBuildHasher;

use merc_utilities::GenerationCounter;
use merc_utilities::GenerationalIndex;
use merc_utilities::cast;

/// A type-safe index into an `IndexedSet`, using generational indices in debug builds to catch
/// reuse of a stale index.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct SetIndex(GenerationalIndex<usize>);

impl Deref for SetIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for SetIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SetIndex({})", self.0)
    }
}

impl fmt::Display for SetIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A set that assigns a unique index to every entry. The returned index can be used to access the inserted entry.
#[derive(Clone)]
pub struct IndexedSet<T, S = FxBuildHasher> {
    /// The table of elements, which can be either filled or empty.
    table: Vec<IndexSetEntry<T>>,
    /// Indexes of the elements in the set. Stores only the index; the hash is recomputed via the
    /// stored `hasher` on lookup using the hash table's explicit-hash API.
    index: HashTable<usize>,
    /// A list of free nodes, where the value is the first free node.
    free: Option<usize>,
    /// The number of generations
    generation_counter: GenerationCounter,
    /// The hasher used to compute hashes for elements
    hasher: S,
}

/// An entry in the indexed set, which can either be filled or empty.
#[derive(Clone)]
enum IndexSetEntry<T> {
    Filled(T),
    Empty(usize),
}

impl<T, S: BuildHasher + Default> IndexedSet<T, S> {
    /// Creates a new empty IndexedSet with the default hasher.
    pub fn new() -> IndexedSet<T, S> {
        IndexedSet {
            table: Vec::default(),
            index: HashTable::new(),
            free: None,
            generation_counter: GenerationCounter::new(),
            hasher: S::default(),
        }
    }
}

impl<T, S> IndexedSet<T, S> {
    /// Creates a new empty IndexedSet with the specified hasher.
    pub fn with_hasher(hash_builder: S) -> IndexedSet<T, S> {
        IndexedSet {
            table: Vec::default(),
            index: HashTable::new(),
            free: None,
            generation_counter: GenerationCounter::new(),
            hasher: hash_builder,
        }
    }

    /// Returns the number of elements in the set.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a reference to the element at the given index, if it exists.
    pub fn get(&self, index: SetIndex) -> Option<&T> {
        if let Some(entry) = self.table.get(self.generation_counter.get_index(index.0)) {
            match entry {
                IndexSetEntry::Filled(element) => Some(element),
                IndexSetEntry::Empty(_) => None,
            }
        } else {
            None
        }
    }

    /// Returns a reference to the element at the given raw table index, if a
    /// filled entry exists there.
    ///
    /// Takes a plain `usize` and performs no generation check, unlike `get`;
    /// it only verifies that the slot is in bounds and filled.
    pub fn get_by_index(&self, index: usize) -> Option<&T> {
        if let Some(entry) = self.table.get(index) {
            match entry {
                IndexSetEntry::Filled(element) => Some(element),
                IndexSetEntry::Empty(_) => None,
            }
        } else {
            None
        }
    }

    /// Returns the capacity of the set.
    pub fn capacity(&self) -> usize {
        self.table.capacity()
    }

    /// Returns an iterator over the elements in the set.
    pub fn iter(&self) -> Iter<'_, T, S> {
        Iter {
            reference: self,
            index: 0,
            generation_counter: &self.generation_counter,
        }
    }
}

impl<T: Clone, S> IndexedSet<T, S> {
    /// Returns a vector containing all elements of this indexed set.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().map(|(_, entry)| entry.clone()).collect()
    }
}

impl<T: Hash + Eq, S: BuildHasher> IndexedSet<T, S> {
    /// Inserts the given element into the set
    ///
    /// Returns the corresponding index and a boolean indicating if the element was inserted.
    pub fn insert_equiv<'a, Q>(&mut self, value: &'a Q) -> (SetIndex, bool)
    where
        Q: Hash + Equivalent<T>,
        T: From<&'a Q>,
    {
        let hash = self.hasher.hash_one(value);

        if let Some(&existing) = self.index.find(hash, |&i| entry_matches(&self.table, i, value)) {
            // The element is already in the set, so return the index.
            return (SetIndex(self.generation_counter.recall_index(existing)), false);
        }

        let value: T = value.into();
        debug_assert_eq!(hash, self.hasher.hash_one(&value), "Hash values should be the same");

        let index = self.insert_into_table(value);
        let hasher_ref = &self.hasher;
        let table_ref = &self.table;
        self.index
            .insert_unique(hash, index, |&i| entry_hash(table_ref, hasher_ref, i));
        (SetIndex(self.generation_counter.create_index(index)), true)
    }

    /// Inserts the given element into the set
    ///
    /// Returns the corresponding index and a boolean indicating if the element was inserted.
    pub fn insert(&mut self, value: T) -> (SetIndex, bool) {
        let hash = self.hasher.hash_one(&value);

        if let Some(&existing) = self.index.find(hash, |&i| entry_matches(&self.table, i, &value)) {
            // The element is already in the set, so return the index.
            return (SetIndex(self.generation_counter.recall_index(existing)), false);
        }

        let index = self.insert_into_table(value);
        let hasher_ref = &self.hasher;
        let table_ref = &self.table;
        self.index
            .insert_unique(hash, index, |&i| entry_hash(table_ref, hasher_ref, i));
        (SetIndex(self.generation_counter.create_index(index)), true)
    }

    /// Returns the index for the given element, or None if it does not exist.
    pub fn index<Q>(&self, key: &Q) -> Option<SetIndex>
    where
        Q: Hash + Equivalent<T> + ?Sized,
    {
        let hash = self.hasher.hash_one(key);
        self.index
            .find(hash, |&i| entry_matches(&self.table, i, key))
            .map(|&i| SetIndex(self.generation_counter.recall_index(i)))
    }

    /// Erases all elements for which f(index, element) returns false. Allows
    /// modifying the given element (as long as the hash/equality does not change).
    pub fn retain_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(SetIndex, &mut T) -> bool,
    {
        for (index, element) in self.table.iter_mut().enumerate() {
            if let IndexSetEntry::Filled(value) = element
                && !f(SetIndex(self.generation_counter.recall_index(index)), value)
            {
                let hash = self.hasher.hash_one(value);
                if let Ok(entry) = self.index.find_entry(hash, |&i| i == index) {
                    entry.remove();
                }

                match self.free {
                    Some(next) => {
                        *element = IndexSetEntry::Empty(next);
                    }
                    None => {
                        *element = IndexSetEntry::Empty(index);
                    }
                };
                self.free = Some(index);
            };
        }
    }

    /// Removes the given element from the set.
    pub fn remove(&mut self, element: &T) -> bool {
        let hash = self.hasher.hash_one(element);

        if let Ok(entry) = self.index.find_entry(hash, |&i| entry_matches(&self.table, i, element)) {
            let (removed_index, _) = entry.remove();
            let next = match self.free {
                Some(next) => next,
                None => removed_index,
            };

            self.table[removed_index] = IndexSetEntry::Empty(next);
            self.free = Some(removed_index);
            true
        } else {
            // The element was not found in the set.
            false
        }
    }

    /// Removes all elements in this indexed set.
    pub fn clear(&mut self) {
        self.table.clear();
        self.index.clear();
        self.free = None;
        self.generation_counter = GenerationCounter::new();
    }

    /// Returns true iff the set contains the given element.
    pub fn contains<Q>(&self, element: &Q) -> bool
    where
        Q: Hash + Equivalent<T>,
    {
        let hash = self.hasher.hash_one(element);
        self.index
            .find(hash, |&i| entry_matches(&self.table, i, element))
            .is_some()
    }

    /// Inserts `value` into the `table`, reusing a free slot if one is available, and returns
    /// its index. Does not modify the secondary hash index.
    fn insert_into_table(&mut self, value: T) -> usize {
        match self.free {
            Some(first) => {
                let next = match self.table[first] {
                    IndexSetEntry::Empty(x) => x,
                    IndexSetEntry::Filled(_) => panic!("The free list contains a filled element"),
                };

                if first == next {
                    // The list is now empty as its first element points to itself.
                    self.free = None;
                } else {
                    // Update free to be the next element in the list.
                    self.free = Some(next);
                }

                self.table[first] = IndexSetEntry::Filled(value);
                first
            }
            None => {
                // No free positions so insert new.
                self.table.push(IndexSetEntry::Filled(value));
                self.table.len() - 1
            }
        }
    }
}

impl<T, S> fmt::Debug for IndexedSet<T, S>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T, S: BuildHasher + Default> Default for IndexedSet<T, S> {
    fn default() -> IndexedSet<T, S> {
        IndexedSet::new()
    }
}

impl<T, S> Index<SetIndex> for IndexedSet<T, S> {
    type Output = T;

    fn index(&self, index: SetIndex) -> &Self::Output {
        // Go through the generation counter so a stale index is detected in
        // debug builds, consistent with [Self::get].
        let raw = self.generation_counter.get_index(index.0);
        cast!(&self.table[raw], IndexSetEntry::Filled)
    }
}

impl<T, S: BuildHasher> IndexMut<SetIndex> for IndexedSet<T, S> {
    fn index_mut(&mut self, index: SetIndex) -> &mut Self::Output {
        // Go through the generation counter so a stale index is detected in
        // debug builds, consistent with [Self::get].
        let raw = self.generation_counter.get_index(index.0);
        cast!(&mut self.table[raw], IndexSetEntry::Filled)
    }
}

/// Returns whether the element stored at `table[index]` is equivalent to `value`.
///
/// Returns `false` for free slots, which should never appear in the secondary hash index.
fn entry_matches<T, Q: Equivalent<T> + ?Sized>(table: &[IndexSetEntry<T>], index: usize, value: &Q) -> bool {
    match &table[index] {
        IndexSetEntry::Filled(element) => value.equivalent(element),
        IndexSetEntry::Empty(_) => false,
    }
}

/// Computes the hash of the filled element stored at `table[index]` using `hasher`.
///
/// Used as the rehasher callback for [`HashTable`] operations.
fn entry_hash<T: Hash, S: BuildHasher>(table: &[IndexSetEntry<T>], hasher: &S, index: usize) -> u64 {
    match &table[index] {
        IndexSetEntry::Filled(element) => hasher.hash_one(element),
        IndexSetEntry::Empty(_) => panic!("entry_hash called on an empty slot"),
    }
}

/// An iterator over the elements in the IndexedSet.
pub struct Iter<'a, T, S> {
    reference: &'a IndexedSet<T, S>,
    index: usize,
    generation_counter: &'a GenerationCounter,
}

impl<'a, T, S> Iterator for Iter<'a, T, S> {
    type Item = (SetIndex, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.reference.table.len() {
            let current_index = self.index;
            self.index += 1;

            if let IndexSetEntry::Filled(element) = &self.reference.table[current_index] {
                return Some((SetIndex(self.generation_counter.recall_index(current_index)), element));
            }
        }

        None
    }
}

impl<'a, T, S> IntoIterator for &'a IndexedSet<T, S> {
    type Item = (SetIndex, &'a T);
    type IntoIter = Iter<'a, T, S>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
