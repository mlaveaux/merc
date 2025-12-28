use std::collections::HashMap;
use std::fmt;
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

    /// Inserts the given (s, T) pair into the antichain and returns true iff it was
    /// not already present.
    pub fn insert(&mut self, key: K, value: VecSet<V>) -> bool {
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
}

impl<K, V: fmt::Debug + Ord> Antichain<K, V> {
    /// Checks the internal consistency of the antichain invariant.
    fn check_consistency(&self) {
        for (_key, values) in &self.storage {
            for i in values.iter() {
                for j in values.iter() {
                    if i == j {
                        // Ignore identical entries
                        continue;
                    }

                    assert!(
                        !i.is_subset(j) && !j.is_subset(i),
                        "Antichain invariant violated: {:?} and {:?} are comparable.",
                        i,
                        j
                    );
                }
            }
        }
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
    use merc_collections::vecset;
    use merc_utilities::random_test;
    use rand::Rng;

    use crate::Antichain;

    #[test]
    fn test_antichain() {
        let mut antichain: Antichain<u32, u32> = Antichain::new();

        let inserted = antichain.insert(1, vecset![2, 3]);
        assert!(inserted);

        println!("{:?}", antichain);

        let inserted = antichain.insert(1, vecset![2, 3, 6]);
        assert!(
            !inserted,
            "The pair (1, {{2,3,6}}) should not be inserted in {:?}.",
            antichain
        );

        let inserted = antichain.insert(1, vecset![2]);
        assert!(
            inserted,
            "The pair (1, {{2}}) should overwrite (1, {{2, 3}}) in {:?}.",
            antichain
        );

        let inserted = antichain.insert(1, vecset![5, 6]);
        assert!(
            inserted,
            "The pair (1, {{5, 6}}) should be inserted since it is incomparable to existing pairs in {:?}.",
            antichain
        );
    }

    #[test]
    fn test_random_antichain() {
        random_test(100, |rng| {
            let mut antichain: Antichain<u32, u32> = Antichain::new();

            // Insert random pairs into the antichain.
            for _ in 0..50 {
                let key = rng.random_range(0..10);
                let set_size = rng.random_range(1..5);
                let mut value = vecset![];

                for _ in 0..set_size {
                    value.insert(rng.random_range(0..20));
                }

                antichain.insert(key, value);
            }

            antichain.check_consistency();
        })
    }
}
