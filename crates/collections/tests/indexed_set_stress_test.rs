//! Stress test for `IndexedSet`'s free-list / generational-index bookkeeping under interleaved
//! `insert`/`remove`/`retain_mut`, which had no direct test coverage in `merc_collections` itself
//! (only indirectly, through the many unrelated crates that use it). Cross-checks against a plain
//! `HashMap`-backed reference model after every operation.

use std::collections::HashMap;

use rand::RngExt;

use merc_collections::IndexedSet;
use merc_collections::SetIndex;
use merc_utilities::random_test;

/// A random sequence of `insert`/`remove`/`retain_mut` on `IndexedSet<u32>`, cross-checked at
/// every step against a `HashMap<u32, SetIndex>` model: every value the model considers present
/// must be retrievable through its recorded index and reachable via `contains`/iteration, and
/// nothing else must be.
#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn test_random_indexed_set_survives_interleaved_insert_remove_retain() {
    random_test(200, |rng| {
        let mut set: IndexedSet<u32> = IndexedSet::default();
        // Model: value -> the index IndexedSet handed back for it, only while present.
        let mut model: HashMap<u32, SetIndex> = HashMap::new();

        for _ in 0..200 {
            match rng.random_range(0..3) {
                0 => {
                    // Insert a value from a small range, so both duplicate-insert and
                    // fresh-insert paths get exercised.
                    let value = rng.random_range(0..40u32);
                    let (index, inserted) = set.insert(value);
                    assert_eq!(
                        inserted,
                        !model.contains_key(&value),
                        "insert's `inserted` flag must match whether the value was already present"
                    );
                    model.insert(value, index);
                }
                1 => {
                    // Remove a value, preferentially one that is actually present so removal of a
                    // live element is exercised as often as removal of an absent one.
                    let value = if !model.is_empty() && rng.random_bool(0.7) {
                        *model.keys().nth(rng.random_range(0..model.len())).unwrap()
                    } else {
                        rng.random_range(0..40u32)
                    };
                    let removed = set.remove(&value);
                    assert_eq!(
                        removed,
                        model.contains_key(&value),
                        "remove's return value must match whether the value was present"
                    );
                    model.remove(&value);
                }
                _ => {
                    // Retain only even values; every removed value must disappear from the model
                    // too, and survivors must keep the *same* index they had before (retain_mut
                    // must not reshuffle live entries).
                    set.retain_mut(|_, value| *value % 2 == 0);
                    model.retain(|value, _| *value % 2 == 0);
                }
            }

            // The set and the model must agree on size, membership, and index validity.
            assert_eq!(set.len(), model.len(), "length mismatch between IndexedSet and model");

            for (&value, &index) in &model {
                assert!(set.contains(&value), "model value {value} should be contains()-able");
                assert_eq!(
                    set.get(index),
                    Some(&value),
                    "get() through the recorded index must still return {value}"
                );
                assert_eq!(
                    set[index], value,
                    "Index/IndexMut through the recorded index must still return {value}"
                );
            }

            // Every element actually stored in the set must be one the model expects, and the
            // index handed out by iteration must be the same one the model recorded.
            let mut seen = std::collections::HashSet::new();
            for (index, &value) in &set {
                assert!(seen.insert(value), "IndexedSet must not yield the same value twice");
                assert_eq!(
                    model.get(&value),
                    Some(&index),
                    "iteration must yield exactly the index recorded for {value}"
                );
            }
            assert_eq!(seen.len(), model.len());
        }
    });
}
