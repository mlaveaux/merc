use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use rand::RngExt;

use merc_utilities::random_test;
use merc_utilities::random_test_threads;

use merc_unsafety::ShardedHashMap;

/// Drives the convenience API with random operations against a `HashSet`
/// oracle, checking that the table agrees with the model after each step.
#[test]
#[cfg_attr(miri, ignore)]
fn random_insert_get_remove() {
    random_test(100, |rng| {
        let map: ShardedHashMap<u64> = ShardedHashMap::with_shards(rng.random_range(1..=16));
        let mut model: HashSet<u64> = HashSet::new();

        for _ in 0..1000 {
            let value = rng.random_range(0..64u64);
            match rng.random_range(0..4) {
                0 => assert_eq!(map.insert(value).1, model.insert(value), "fresh-insert flags agree"),
                1 => assert_eq!(map.contains(&value), model.contains(&value)),
                2 => {
                    let expected = model.contains(&value).then_some(value);
                    assert_eq!(map.get(&value), expected);
                }
                _ => assert_eq!(map.remove_equiv(&value).is_some(), model.remove(&value)),
            }
            assert_eq!(map.len(), model.len());
        }
    });
}

/// Exercises the raw API the way the forest does: the table stores indices
/// into a side vector and the equality/hash closures dereference it. Random
/// payloads are interned and checked against a `HashMap` oracle mapping each
/// payload to the index it should resolve to.
#[test]
#[cfg_attr(miri, ignore)]
fn random_raw_interning_dedup() {
    random_test(50, |rng| {
        let map: ShardedHashMap<usize> = ShardedHashMap::with_shards(rng.random_range(1..=8));
        let store: Mutex<Vec<u64>> = Mutex::new(Vec::new());

        let intern = |value: u64| -> usize {
            let hash = map.hash(&value);
            let eq = |&index: &usize| store.lock().unwrap()[index] == value;
            if let Some(index) = map.find(hash, eq) {
                return index;
            }
            map.find_or_insert_with(
                hash,
                eq,
                |&index: &usize| map.hash(&store.lock().unwrap()[index]),
                || {
                    let mut store = store.lock().unwrap();
                    store.push(value);
                    store.len() - 1
                },
            )
            .0
        };

        let mut model: HashMap<u64, usize> = HashMap::new();
        for _ in 0..500 {
            let value = rng.random_range(0..32u64);
            let index = intern(value);
            let next = model.len();
            assert_eq!(
                index,
                *model.entry(value).or_insert(next),
                "equal payloads share an index"
            );
        }

        assert_eq!(map.len(), model.len());
        assert_eq!(
            store.lock().unwrap().len(),
            model.len(),
            "no payload pushed for a duplicate"
        );
    });
}

/// Concurrently inserts random values from a fixed value space. Across all
/// threads each value yields exactly one fresh insertion, so the count of
/// fresh insertions must equal the table size.
#[test]
#[cfg_attr(miri, ignore)]
fn random_concurrent_insert_counts_each_once() {
    let map: Arc<ShardedHashMap<u64>> = Arc::new(ShardedHashMap::with_shards(8));
    let inserted = Arc::new(AtomicUsize::new(0));
    let values = 256u64;

    random_test_threads(
        2000,
        8,
        || (Arc::clone(&map), Arc::clone(&inserted)),
        move |rng, (map, inserted)| {
            let value = rng.random_range(0..values);
            if map.insert(value).1 {
                inserted.fetch_add(1, Ordering::Relaxed);
            }
        },
    );

    assert_eq!(
        inserted.load(Ordering::Relaxed),
        map.len(),
        "every fresh insertion is a distinct resident value"
    );
    map.for_each(|&value| assert!(value < values, "only inserted values are resident"));
}
