use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use merc_collections::VecSet;

/// An antichain is a structure (<, S) such that < is a preorder on S and no
/// two elements of S are comparable under <; it is dual to a chain. Here,
/// values are grouped by key, and the sets stored under each key are kept
/// pairwise incomparable under the subset relation.
pub struct Antichain<K, V> {
    storage: HashMap<K, VecSet<VecSet<V>>>,

    /// The largest number of keys the antichain has held at once.
    max_antichain: usize,
    /// Number of `insert` calls that added a new pair.
    antichain_misses: usize,
    /// Total number of `insert` calls.
    antichain_inserts: usize,
}

impl<K: Eq + Hash, V: Clone + Ord> Antichain<K, V> {
    /// Creates a new empty antichain.
    pub fn new() -> Self {
        Antichain {
            storage: HashMap::new(),
            max_antichain: 0,
            antichain_misses: 0,
            antichain_inserts: 0,
        }
    }

    /// Checks whether the antichain contains a pair (s, T') such that T ⊆ T',
    /// i.e., a stored set that is a superset of `value`.
    pub fn contains_superset(&self, key: &K, value: &VecSet<V>) -> bool {
        self.storage
            .get(key)
            .is_some_and(|entry| entry.iter().any(|inner_value| value.is_subset(inner_value)))
    }

    /// Checks whether the antichain contains a pair (s, T') such that T' ⊆ T,
    /// i.e., a stored set that is a subset of `value`.
    pub fn contains_subset(&self, key: &K, value: &VecSet<V>) -> bool {
        self.storage
            .get(key)
            .is_some_and(|entry| entry.iter().any(|inner_value| inner_value.is_subset(value)))
    }

    /// Returns `(max_antichain_size, insert_misses, insert_calls)`: the largest
    /// number of keys held at once, the number of `insert` calls that added a
    /// new pair, and the total number of `insert` calls.
    pub fn metrics(&self) -> (usize, usize, usize) {
        (self.max_antichain, self.antichain_misses, self.antichain_inserts)
    }

    /// Returns an iterator over the pairs in the antichain.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &VecSet<V>)> {
        self.storage
            .iter()
            .flat_map(|(key, values)| values.iter().map(move |value| (key, value)))
    }
}

impl<K: Eq + Hash, V: Clone + Ord> Default for Antichain<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the antichain data structure used in the refinement checks.
pub trait AC<K: Eq + Hash, V: Clone + Ord> {
    /// Inserts `(key, value)` into the antichain.
    ///
    /// If a set already stored under `key` is a subset of `value`, `value` is
    /// dominated: nothing is inserted and this returns `false`. Otherwise
    /// `value` is inserted (any stored superset of `value` is removed, as it
    /// is now dominated) and this returns `true`.
    fn insert(&mut self, key: K, value: VecSet<V>) -> bool;

    /// Clears the antichain.
    fn clear(&mut self);
}

impl<K: Eq + Hash, V: Clone + Ord> AC<K, V> for Antichain<K, V> {
    fn insert(&mut self, key: K, value: VecSet<V>) -> bool {
        let mut inserted = false;
        self.storage
            .entry(key)
            .and_modify(|entry| {
                let mut contains = false;
                entry.retain(|inner_value| {
                    if inner_value.is_subset(&value) {
                        // The new value is a superset of an existing entry
                        contains = true;
                        true
                    } else if value.is_subset(inner_value) {
                        // Remove any entry that is a superset of the new value
                        false
                    } else {
                        // Leave incomparable entries unchanged
                        true
                    }
                });

                if !contains {
                    self.antichain_misses += 1; // Was not present
                    entry.insert(value.clone());
                    inserted = true;
                }
            })
            .or_insert_with(|| {
                self.antichain_misses += 1; // Was not present
                inserted = true;
                VecSet::singleton(value)
            });

        self.antichain_inserts += 1;
        self.max_antichain = self.max_antichain.max(self.storage.len());

        inserted
    }

    fn clear(&mut self) {
        self.storage.clear();
    }
}

impl<T: fmt::Debug, U: fmt::Debug> fmt::Debug for Antichain<T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Antichain {{")?;
        for (key, values) in &self.storage {
            writeln!(f, "  {:?}: {:?}", key, values)?;
        }
        writeln!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use merc_collections::VecSet;
    use merc_collections::vecset;
    use merc_utilities::random_test;
    use rand::RngExt;

    use crate::AC;
    use crate::Antichain;

    /// Returns the sorted (key, set) pairs stored in the antichain.
    fn contents(antichain: &Antichain<u32, u32>) -> Vec<(u32, Vec<u32>)> {
        let mut result: Vec<_> = antichain
            .iter()
            .map(|(key, value)| (*key, value.iter().copied().collect()))
            .collect();
        result.sort();
        result
    }

    #[test]
    fn test_antichain() {
        let mut antichain: Antichain<u32, u32> = Antichain::new();

        assert!(antichain.insert(1, vecset![2, 3]));
        assert!(!antichain.insert(1, vecset![2, 3, 6]), "a superset is dominated");
        assert!(antichain.insert(1, vecset![2]), "a subset replaces its supersets");
        assert!(antichain.insert(1, vecset![5, 6]), "an incomparable set is added");
        assert!(antichain.insert(2, vecset![2, 3]), "keys are independent");

        assert_eq!(
            contents(&antichain),
            vec![(1, vec![2]), (1, vec![5, 6]), (2, vec![2, 3])]
        );
        assert!(antichain.contains_subset(&1, &vecset![2, 7]));
        assert!(!antichain.contains_subset(&1, &vecset![5, 7]));
        assert!(antichain.contains_superset(&1, &vecset![5]));
        assert!(!antichain.contains_superset(&2, &vecset![4]));
        assert_eq!(antichain.metrics(), (2, 4, 5));
    }

    /// After inserting arbitrary pairs, the antichain holds exactly the minimal inserted sets
    /// per key, and `insert` returns true iff no inserted set under the key is a subset.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_antichain() {
        random_test(100, |rng| {
            let mut antichain: Antichain<u32, u32> = Antichain::new();
            let mut inserted: Vec<(u32, VecSet<u32>)> = Vec::new();

            for _ in 0..50 {
                let key = rng.random_range(0..5);
                let mut value = vecset![];
                for _ in 0..rng.random_range(0..5) {
                    value.insert(rng.random_range(0..8));
                }

                let dominated = inserted.iter().any(|(k, v)| *k == key && v.is_subset(&value));
                assert_eq!(antichain.insert(key, value.clone()), !dominated);
                inserted.push((key, value));

                let mut minimal: Vec<(u32, Vec<u32>)> = inserted
                    .iter()
                    .filter(|(k, v)| !inserted.iter().any(|(k2, v2)| k == k2 && v2.is_subset(v) && v2 != v))
                    .map(|(k, v)| (*k, v.iter().copied().collect()))
                    .collect();
                minimal.sort();
                minimal.dedup();
                assert_eq!(contents(&antichain), minimal);
            }
        })
    }
}
