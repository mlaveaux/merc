use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use itertools::Itertools;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VecSet<T> {

    /// The internal storage with the invariant that the array is sorted.
    sorted_array: Vec<T>,
}

impl<T: Ord> VecSet<T> {
    pub fn new() -> Self {
        Self {
            sorted_array: Vec::new(),
        }
    }

    pub fn singleton(element: T) -> Self {
        Self {
            sorted_array: vec![element],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sorted_array.is_empty()
    }

    /// Inserts the given element into the set, returns true iff the element was
    /// inserted.
    pub fn insert(&mut self, element: T) -> bool {
        // Finds the location where to insert the element to keep the array sorted.
        if let Err(position) = self.sorted_array.binary_search(&element) {
            self.sorted_array.insert(position, element);
            return true;
        }

        false
    }
}

impl<T: fmt::Debug> fmt::Debug for VecSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{:?}}}", self.sorted_array.iter().format(", "))
    }
}

/// Keep
pub struct Antichain<K, V> {
    storage: HashMap<K, VecSet<VecSet<V>>>,

    /// The largest size of the antichain.
    max_antichain: usize,
    /// Number of times a pair was inserted into the antichain.
    antichain_misses: usize, 
    /// Number of times antichain_insert was called.
    antichain_inserts: usize,     
}

impl<K: Eq + Hash, V: Ord> Antichain<K, V> {

    /// Inserts the given (impl, spec) pair into the antichain and returns true iff it was
    /// not already present.
    pub fn insert(&mut self, key: K, value: VecSet<V>) -> bool {
        self.storage.entry(key)
            .or_insert(VecSet::singleton(value));

        true
    }
}