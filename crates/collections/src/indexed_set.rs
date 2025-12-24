use core::panic;
use std::fmt;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;
use std::ops::Index;
use std::ops::IndexMut;

use hashbrown::Equivalent;
use hashbrown::HashSet;
use rustc_hash::FxBuildHasher;

use merc_utilities::GenerationCounter;
use merc_utilities::GenerationalIndex;
use merc_utilities::NoHasherBuilder;
use merc_utilities::cast;

/// A type-safe index for use with [IndexedSet]. Uses generational indices in debug builds to assert
/// correct usage of indices.
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
pub struct IndexedSet<T, S = FxBuildHasher> {
    /// The table of elements, which can be either filled or empty.
    table: Vec<IndexSetEntry<T>>,
    /// Indexes of the elements in the set, using NoHasher to directly use precomputed hashes.
    index: HashSet<IndexEntry, NoHasherBuilder>,
    /// A list of free nodes, where the value is the first free node.
    free: Option<usize>,
    /// The number of generations
    generation_counter: GenerationCounter,
    /// The hasher used to compute hashes for elements
    hasher: S,
}

/// An entry in the indexed set, which can either be filled or empty.
enum IndexSetEntry<T> {
    Filled(T),
    Empty(usize),
}

impl<T, S: BuildHasher + Default> IndexedSet<T, S> {
    /// Creates a new empty IndexedSet with the default hasher.
    pub fn new() -> IndexedSet<T, S> {
        IndexedSet {
            table: Vec::default(),
            index: HashSet::with_hasher(NoHasherBuilder),
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
            index: HashSet::with_hasher(NoHasherBuilder),
            free: None,
            generation_counter: GenerationCounter::new(),
            hasher: hash_builder,
        }
    }

    /// Returns the number of elements in the set.
    pub fn len(&self) -> usize {
        self.table.len()
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
        let equivalent = IndexValueEquivalent::new(value, &self.hasher, &self.table);

        if let Some(entry) = self.index.get(&equivalent) {
            // The element is already in the set, so return the index.
            return (SetIndex(self.generation_counter.recall_index(entry.index)), false);
        }

        let value: T = value.into();
        let hash = self.hasher.hash_one(&value);

        debug_assert_eq!(hash, equivalent.hash(), "Hash values should be the same");

        let index = match self.free {
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
        };

        self.index.insert(IndexEntry::new(index, hash));
        (SetIndex(self.generation_counter.create_index(index)), true)
    }

    /// Inserts the given element into the set
    ///
    /// Returns the corresponding index and a boolean indicating if the element was inserted.
    pub fn insert(&mut self, value: T) -> (SetIndex, bool) {
        let equivalent = IndexValueEquivalent::new(&value, &self.hasher, &self.table);

        if let Some(entry) = self.index.get(&equivalent) {
            // The element is already in the set, so return the index.
            return (SetIndex(self.generation_counter.recall_index(entry.index)), false);
        }

        let hash = equivalent.hash();

        let index = match self.free {
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
        };

        self.index.insert(IndexEntry::new(index, hash));
        (SetIndex(self.generation_counter.create_index(index)), true)
    }

    /// Returns the index for the given element, or None if it does not exist.
    pub fn index<Q>(&self, key: &Q) -> Option<SetIndex>
    where
        Q: Hash + Equivalent<T>,
    {
        let equivalent = IndexValueEquivalent::new(key, &self.hasher, &self.table);

        self.index
            .get(&equivalent)
            .map(|entry| SetIndex(self.generation_counter.recall_index(entry.index)))
    }

    /// Erases all elements for which f(index, element) returns false. Allows
    /// modifying the given element (as long as the hash/equality does not change).
    pub fn retain_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(SetIndex, &mut T) -> bool,
    {
        for (index, element) in self.table.iter_mut().enumerate() {
            if let IndexSetEntry::Filled(value) = element {
                if !f(SetIndex(self.generation_counter.recall_index(index)), value) {
                    // Find and remove the IndexEntry from the index set
                    let entry_to_remove = self.index.iter().find(|entry| entry.index == index).cloned();

                    if let Some(entry) = entry_to_remove {
                        self.index.remove(&entry);
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
                }
            };
        }
    }

    /// Removes the given element from the set.
    pub fn remove(&mut self, element: &T) -> bool {
        let equivalent = IndexValueEquivalent::new(element, &self.hasher, &self.table);

        if let Some(entry) = self.index.take(&equivalent) {
            let next = match self.free {
                Some(next) => next,
                None => entry.index,
            };

            self.table[entry.index] = IndexSetEntry::Empty(next);
            self.free = Some(entry.index);
            true
        } else {
            // The element was not found in the set.
            false
        }
    }

    /// Returns true iff the set contains the given element.
    pub fn contains<Q>(&self, element: &Q) -> bool
    where
        Q: Hash + Equivalent<T>,
    {
        // Compute the hash using our hash_builder
        let equivalent = IndexValueEquivalent::new(element, &self.hasher, &self.table);
        self.index.contains(&equivalent)
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
        cast!(&self.table[*index], IndexSetEntry::Filled)
    }
}

impl<T, S: BuildHasher> IndexMut<SetIndex> for IndexedSet<T, S> {
    fn index_mut(&mut self, index: SetIndex) -> &mut Self::Output {
        cast!(&mut self.table[*index], IndexSetEntry::Filled)
    }
}

/// An entry in the index that stores both the index and a precomputed hash value
#[derive(Copy, Clone, PartialEq, Eq)]
struct IndexEntry {
    /// The index into the table
    index: usize,
    /// Precomputed hash value of the element at this index
    hash: u64,
}

impl IndexEntry {
    /// Creates a new IndexEntry with the given index and hash value
    fn new(index: usize, hash: u64) -> Self {
        Self { index, hash }
    }
}

impl Hash for IndexEntry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Simply use the precomputed hash value
        state.write_u64(self.hash);
    }
}

/// An equivalent wrapper that allows looking up elements in a set using the original value.
/// This avoids duplicating the key in both the table and index.
struct IndexValueEquivalent<'a, T, Q> {
    value: &'a Q,
    hash: u64,
    table: &'a Vec<IndexSetEntry<T>>,
}

impl<T, Q> IndexValueEquivalent<'_, T, Q> {
    fn hash(&self) -> u64 {
        // This is a placeholder for the actual hash function
        self.hash
    }
}

impl<'a, T, Q: Hash> IndexValueEquivalent<'a, T, Q> {
    /// Creates a new IndexValueEquivalent with the given value and table.
    fn new<S: BuildHasher>(value: &'a Q, hasher: &S, table: &'a Vec<IndexSetEntry<T>>) -> Self {
        // Constructor allows for centralized creation logic
        Self {
            value,
            table,
            hash: hasher.hash_one(value),
        }
    }
}

impl<T, Q: Equivalent<T>> Equivalent<IndexEntry> for IndexValueEquivalent<'_, T, Q> {
    fn equivalent(&self, key: &IndexEntry) -> bool {
        if let Some(IndexSetEntry::Filled(element)) = self.table.get(key.index) {
            self.value.equivalent(element)
        } else {
            false
        }
    }
}

impl<T, Q> Hash for IndexValueEquivalent<'_, T, Q> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand::Rng;

    use merc_utilities::random_test;

    use crate::IndexedSet;
    use crate::SetIndex;

    #[test]
    fn test_random_indexed_set_construction() {
        random_test(100, |rng| {
            let mut input = vec![];
            for _ in 0..100 {
                input.push(rng.random_range(0..32) as usize);
            }

            let mut indices: HashMap<usize, SetIndex> = HashMap::default();

            // Insert several elements and keep track of the resulting indices.
            let mut set: IndexedSet<usize> = IndexedSet::default();
            for element in &input {
                let index = set.insert(*element).0;
                indices.insert(*element, index);
            }

            // Check if the indices match the previously stored ones.
            for (index, value) in &set {
                assert_eq!(
                    indices[value], index,
                    "The resulting index does not match the returned value"
                );
            }

            // Remove some elements from the set.
            for value in &mut input.iter().take(10) {
                set.remove(value);
                indices.remove(value);
            }

            // Check consistency of the indexed set after removals.
            for (index, value) in &set {
                assert_eq!(
                    indices[value], index,
                    "The resulting index does not match the returned value"
                );
            }

            for (value, index) in &indices {
                assert!(
                    set.get(*index) == Some(value),
                    "Index {} should still match element {:?}",
                    *index,
                    value
                );
            }

            // Check the contains function
            for value in &input {
                let contains = indices.contains_key(value);
                assert_eq!(
                    set.contains(value),
                    contains,
                    "The contains function returned an incorrect result for value {:?}",
                    value
                );
            }
        })
    }
}
