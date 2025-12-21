use std::collections::HashMap;
use std::hash::Hash;

use merc_collections::VecSet;

/// An antichain is a data structure that stores pairs of (s, T) \subset S x 2^S, where `S` is a set of elements that have a total order <.
/// The antichain maintains the invariant that for any two pairs (s1, T1) and (s2, T2) in the antichain, neither s1 < s2 nor s2 < s1 holds, i.e.,
/// it is dual to a chain.
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
    /// Inserts the given (s, T) pair into the antichain and returns true iff it was
    /// not already present.
    pub fn insert(&mut self, key: K, value: VecSet<V>) -> bool {
        self.storage.entry(key).or_insert_with(|| {
            self.antichain_misses += 1; // Was not present
            VecSet::singleton(value)
        });

        self.antichain_inserts += 1;
        self.max_antichain = self.max_antichain.max(self.storage.len());

        true
    }
}

#[cfg(test)]
mod tests {}
