use std::collections::HashMap;

use rand::RngExt;

use merc_collections::IndexedSet;
use merc_collections::SetIndex;
use merc_utilities::random_test;

/// A small deterministic case: duplicates get the index of the first insertion, and removed
/// elements can no longer be found.
#[test]
fn test_insert_remove() {
    let mut set: IndexedSet<&str> = IndexedSet::new();
    let (a, inserted_a) = set.insert("a");
    let (b, inserted_b) = set.insert("b");
    assert!(inserted_a && inserted_b);
    assert_ne!(a, b);
    assert_eq!(set.insert("a"), (a, false));
    assert_eq!(set.len(), 2);
    assert_eq!((set[a], set[b]), ("a", "b"));

    assert!(set.remove(&"a"));
    assert!(!set.remove(&"a"), "an element can only be removed once");
    assert_eq!(set.len(), 1);
    assert!(!set.contains(&"a"));
    assert_eq!(set.index(&"a"), None);
    assert_eq!(set.index(&"b"), Some(b));
    assert_eq!(set.to_vec(), vec!["b"]);

    let (c, inserted_c) = set.insert("c");
    assert!(inserted_c);
    assert_eq!(set[c], "c");
    assert_eq!(set.get(b), Some(&"b"));

    set.retain_mut(|_, element| *element != "b");
    assert_eq!(set.len(), 1);
    assert_eq!(set.to_vec(), vec!["c"]);
    assert!(!set.contains(&"b"));

    set.clear();
    assert!(set.is_empty());
    assert_eq!(set.iter().count(), 0);
}

/// Random inserts and removals checked against a `HashMap` from element to index.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_indexed_set_construction() {
    random_test(100, |rng| {
        let mut set: IndexedSet<usize> = IndexedSet::default();
        let mut model: HashMap<usize, SetIndex> = HashMap::default();

        for _ in 0..200 {
            let value = rng.random_range(0..32);
            if rng.random_bool(0.7) {
                let (index, inserted) = set.insert(value);
                assert_eq!(inserted, !model.contains_key(&value), "insert({value})");
                assert_eq!(*model.entry(value).or_insert(index), index, "insert({value})");
            } else {
                assert_eq!(set.remove(&value), model.remove(&value).is_some(), "remove({value})");
            }
            assert_eq!(set.len(), model.len());
        }

        // Every element of the model is found at its index, and iteration yields exactly the model.
        for (value, &index) in &model {
            assert_eq!(set.get(index), Some(value));
            assert_eq!(set.index(value), Some(index));
        }
        let mut iterated: Vec<(usize, SetIndex)> = set.iter().map(|(index, &value)| (value, index)).collect();
        let mut expected: Vec<(usize, SetIndex)> = model.iter().map(|(&value, &index)| (value, index)).collect();
        iterated.sort();
        expected.sort();
        assert_eq!(iterated, expected);

        for value in 0..32 {
            assert_eq!(set.contains(&value), model.contains_key(&value), "contains({value})");
        }
    })
}
