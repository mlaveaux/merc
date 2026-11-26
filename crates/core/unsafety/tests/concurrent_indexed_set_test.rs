use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use rand::RngExt;

use merc_utilities::random_test;
use merc_utilities::random_test_threads;

use merc_unsafety::ConcurrentIndexedSet;

/// Inserts random values from a small space and checks the set against a
/// `HashMap<value, index>` oracle after every step: the fresh-insert flag,
/// the index assigned in insertion order (dense on a single thread), and
/// that every read path (`index`, `contains`, `get_by_index`) round-trips
/// against the model.
#[test]
#[cfg_attr(miri, ignore)]
fn random_insert_dedup_and_roundtrip() {
    random_test(50, |rng| {
        let set: ConcurrentIndexedSet<u64> = ConcurrentIndexedSet::new();
        let mut model: HashMap<u64, usize> = HashMap::new();

        for _ in 0..500 {
            let value = rng.random_range(0..32u64);
            let next = model.len();
            let expected_new = !model.contains_key(&value);

            let (index, is_new) = set.insert(value);
            assert_eq!(is_new, expected_new, "fresh-insert flag agrees with the model");
            assert_eq!(
                index,
                *model.entry(value).or_insert(next),
                "equal values share the index assigned in insertion order"
            );

            assert_eq!(set.get_by_index(index), Some(&value), "the index resolves to the value");
            assert_eq!(set.index(&value), Some(index), "lookup is the inverse of get_by_index");
            assert!(set.contains(&value));
        }

        assert_eq!(set.len(), model.len());

        // Values from outside the inserted space are absent.
        assert_eq!(set.index(&64), None);
        assert!(!set.contains(&64));
        assert_eq!(set.get_by_index(model.len()), None);
    });
}

/// Concurrently inserts random values from a fixed value space across
/// several threads. Across all threads each value yields exactly one fresh
/// insertion, so the count of fresh insertions must equal the set size, and
/// every returned index resolves to the value that was inserted even under
/// contention.
#[test]
#[cfg_attr(miri, ignore)]
fn random_concurrent_insert_counts_each_once() {
    let set: Arc<ConcurrentIndexedSet<u64>> = Arc::new(ConcurrentIndexedSet::new());
    let inserted = Arc::new(AtomicUsize::new(0));
    let values = 256u64;

    random_test_threads(
        2000,
        8,
        || (Arc::clone(&set), Arc::clone(&inserted)),
        move |rng, (set, inserted)| {
            let value = rng.random_range(0..values);
            let (index, is_new) = set.insert(value);
            assert_eq!(
                set.get_by_index(index),
                Some(&value),
                "the index resolves under contention"
            );
            if is_new {
                inserted.fetch_add(1, Ordering::Relaxed);
            }
        },
    );

    assert_eq!(
        inserted.load(Ordering::Relaxed),
        set.len(),
        "every fresh insertion is a distinct resident value"
    );

    // Indices may be sparse, so iterate the resident values rather than
    // `0..len`. Each must be in range and round-trip through `index` and
    // `get_by_index`.
    let mut residents = HashSet::new();
    for &value in set.iter() {
        assert!(value < values, "only inserted values are resident");
        assert!(residents.insert(value), "each value is resident once");
        let index = set.index(&value).expect("a resident value has an index");
        assert_eq!(set.get_by_index(index), Some(&value), "index is the inverse of lookup");
    }
    assert_eq!(residents.len(), set.len(), "iter visits every resident value once");
}
