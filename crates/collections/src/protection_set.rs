use core::panic;
use std::fmt;
use std::hash::Hash;
use std::ops::Deref;
use std::ops::Index;

use merc_utilities::GenerationCounter;
use merc_utilities::GenerationalIndex;

/// A type-safe index for the ProtectionSet to prevent accidental use of wrong indices
#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct ProtectionIndex(GenerationalIndex<usize>);

impl Deref for ProtectionIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for ProtectionIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProtectionIndex({:?})", self.0)
    }
}

impl fmt::Display for ProtectionIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A collection that assigns a unique index to every object added to it, and allows
/// removing objects while reusing their indices later. This is useful for managing
/// objects that must not be garbage collected, and as such it is called a protection set.
/// Is is similar to a [ `crate::IndexedSet`], except that we cannot look up elements by value.
#[derive(Debug, Default)]
pub struct ProtectionSet<T> {
    roots: Vec<Entry<T>>, // The set of root active nodes.
    free: Option<usize>,
    number_of_insertions: u64,
    size: usize,
    /// The number of generations
    generation_counter: GenerationCounter,
}

/// TODO: Is it possible to get the size of entries down to a sizeof(NonZero<usize>)?
#[derive(Debug)]
enum Entry<T> {
    Filled(T),
    Free(usize),
}

impl<T> ProtectionSet<T> {
    /// Creates a new empty protection set.
    pub fn new() -> Self {
        ProtectionSet {
            roots: Vec::new(),
            free: None,
            number_of_insertions: 0,
            size: 0,
            generation_counter: GenerationCounter::new(),
        }
    }

    /// Returns the number of insertions into the protection set.
    pub fn number_of_insertions(&self) -> u64 {
        self.number_of_insertions
    }

    /// Returns maximum number of active instances.
    pub fn maximum_size(&self) -> usize {
        self.roots.capacity()
    }

    /// Returns the number of roots in the protection set
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns whether the protection set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an iterator over all root indices in the protection set.
    pub fn iter(&self) -> ProtSetIter<'_, T> {
        ProtSetIter {
            current: 0,
            protection_set: self,
            generation_counter: &self.generation_counter,
        }
    }

    /// Returns whether the protection set contains the given index.
    pub fn contains_root(&self, index: ProtectionIndex) -> bool {
        matches!(self.roots[self.generation_counter.get_index(index.0)], Entry::Filled(_))
    }

    /// Adds the given object to the protection set and returns its index.
    pub fn protect(&mut self, object: T) -> ProtectionIndex {
        self.number_of_insertions += 1;
        self.size += 1;

        let index = match self.free {
            Some(first) => {
                match &self.roots[first] {
                    Entry::Free(next) => {
                        if first == *next {
                            // The list is empty as its first element points to itself.
                            self.free = None;
                        } else {
                            // Update free to be the next element in the list.
                            self.free = Some(*next);
                        }
                    }
                    Entry::Filled(_) => {
                        panic!("The free list should not point a filled entry");
                    }
                }

                self.roots[first] = Entry::Filled(object);
                first
            }
            None => {
                // If free list is empty insert new entry into roots.
                self.roots.push(Entry::Filled(object));
                let index = self.roots.len() - 1;

                // Postcondition: verify the object was correctly added
                debug_assert!(
                    matches!(self.roots[index], Entry::Filled(_)),
                    "Failed to add object to protection set"
                );

                index
            }
        };

        ProtectionIndex(self.generation_counter.create_index(index))
    }

    /// Remove protection from the given object. Note that index must be the
    /// index returned by the [ProtectionSet::protect] call.
    pub fn unprotect(&mut self, index: ProtectionIndex) {
        let index = self.generation_counter.get_index(index.0);

        debug_assert!(
            matches!(self.roots[index], Entry::Filled(_)),
            "Index {index} is does not point to a filled entry"
        );

        self.size -= 1;

        match self.free {
            Some(next) => {
                self.roots[index] = Entry::Free(next);
            }
            None => {
                self.roots[index] = Entry::Free(index);
            }
        };

        self.free = Some(index);

        // Postcondition: verify the object was correctly removed from protection
        debug_assert!(
            matches!(self.roots[index], Entry::Free(_)),
            "Failed to unprotect object"
        );
    }

    /// Replaces the object at the given index with the new object.
    pub fn replace(&mut self, index: ProtectionIndex, object: T) {
        let index = self.generation_counter.get_index(index.0);

        debug_assert!(
            matches!(self.roots[index], Entry::Filled(_)),
            "Index {index} is does not point to a filled entry"
        );

        self.roots[index] = Entry::Filled(object);
    }
}

impl<T> Index<ProtectionIndex> for ProtectionSet<T> {
    type Output = T;

    fn index(&self, index: ProtectionIndex) -> &Self::Output {
        match &self.roots[*index] {
            Entry::Filled(value) => value,
            Entry::Free(_) => {
                panic!("Attempting to index free spot {}", index);
            }
        }
    }
}

pub struct ProtSetIter<'a, T> {
    current: usize,
    protection_set: &'a ProtectionSet<T>,
    generation_counter: &'a GenerationCounter,
}

impl<'a, T> Iterator for ProtSetIter<'a, T> {
    type Item = (ProtectionIndex, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        // Find the next valid entry, return it when found or None when end of roots is reached.
        while self.current < self.protection_set.roots.len() {
            let idx = self.current;
            self.current += 1;

            if let Entry::Filled(object) = &self.protection_set.roots[idx] {
                return Some((ProtectionIndex(self.generation_counter.recall_index(idx)), object));
            }
        }

        None
    }
}

impl<'a, T> IntoIterator for &'a ProtectionSet<T> {
    type Item = (ProtectionIndex, &'a T);
    type IntoIter = ProtSetIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::Rng;

    use merc_utilities::random_test;
    use merc_utilities::test_logger;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_protection_set() {
        random_test(100, |rng| {
            let mut protection_set = ProtectionSet::<usize>::new();

            // Protect a number of indices and record their roots.
            let mut indices: Vec<ProtectionIndex> = Vec::new();

            for _ in 0..5000 {
                indices.push(protection_set.protect(rng.random_range(0..1000)));
            }

            // Unprotect a number of roots.
            for index in 0..2500 {
                assert!(protection_set[indices[index]] <= 1000);
                protection_set.unprotect(indices[index]);
                indices.remove(index);
            }

            // Protect more to test the freelist
            for _ in 0..1000 {
                indices.push(protection_set.protect(rng.random_range(0..1000)));
            }

            for index in &indices {
                assert!(
                    protection_set.contains_root(*index),
                    "All indices that are not unprotected should occur in the protection set"
                );
            }

            assert_eq!(
                protection_set.iter().count(),
                6000 - 2500,
                "This is the number of roots remaining"
            );
            assert_eq!(protection_set.number_of_insertions(), 6000);
            assert!(protection_set.maximum_size() >= 5000);
            assert!(!protection_set.is_empty());
        });
    }

    #[test]
    fn test_protection_set_basic() {
        test_logger();

        let mut set = ProtectionSet::<String>::new();

        // Protect some values
        let idx1 = set.protect(String::from("value1"));
        let idx2 = set.protect(String::from("value2"));

        // Verify contains_root works
        assert!(set.contains_root(idx1));
        assert!(set.contains_root(idx2));

        // Test indexing
        assert_eq!(set[idx1], "value1");
        assert_eq!(set[idx2], "value2");

        // Test unprotect
        set.unprotect(idx1);
        assert!(!set.contains_root(idx1));
        assert!(set.contains_root(idx2));

        // Re-use freed slot
        let idx3 = set.protect(String::from("value3"));
        assert_eq!(set[idx3], "value3");
    }
}
