use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use rand::RngExt;

use merc_utilities::random_test;
use merc_utilities::random_test_threads;

use merc_unsafety::ConcurrentAppendVec;

/// Pushes random values, checking that every returned index resolves to the
/// value pushed, that the length matches the number of pushes, and that `iter`
/// yields the values in index order.
#[test]
#[cfg_attr(miri, ignore)]
fn random_push_get_len() {
    random_test(50, |rng| {
        // A small block size makes gaps frequent so they are exercised.
        let vec: ConcurrentAppendVec<u64, 4> = ConcurrentAppendVec::new();
        let mut model: HashMap<usize, u64> = HashMap::new();

        for _ in 0..200 {
            let value = rng.random_range(0..1000u64);
            let index = vec.push(value);
            assert!(model.insert(index, value).is_none(), "indices are unique");
            assert_eq!(vec.get(index), Some(&value), "index resolves to its value");
        }

        assert_eq!(vec.len(), model.len(), "length counts written elements");
        for (&index, &value) in &model {
            assert_eq!(vec.get(index), Some(&value));
        }

        let mut expected: Vec<(usize, u64)> = model.into_iter().collect();
        expected.sort_unstable();
        let expected: Vec<u64> = expected.into_iter().map(|(_, value)| value).collect();
        let collected: Vec<u64> = vec.iter().copied().collect();
        assert_eq!(
            collected, expected,
            "iter visits every written element once, in index order"
        );
    });
}

/// A single short block leaves the rest of the reserved block as gaps.
#[test]
fn gaps_read_back_as_none() {
    let vec: ConcurrentAppendVec<u64, 4> = ConcurrentAppendVec::new();
    let i0 = vec.push(10);
    let i1 = vec.push(20);
    assert_eq!((i0, i1), (0, 1));
    assert_eq!(vec.get(0), Some(&10));
    assert_eq!(vec.get(1), Some(&20));
    assert_eq!(vec.get(2), None, "reserved but unwritten slot is a gap");
    assert_eq!(vec.get(3), None);
    assert_eq!(vec.get(1000), None, "out of range");
    assert_eq!(vec.len(), 2);
}

/// `get_unchecked` returns the same reference as `get` for written indices,
/// across the block and bucket boundaries that a small `BLOCK` produces.
#[test]
fn get_unchecked_matches_get() {
    let vec: ConcurrentAppendVec<u64, 4> = ConcurrentAppendVec::new();
    let entries: Vec<(usize, u64)> = (0..40u64).map(|value| (vec.push(value * 7), value * 7)).collect();
    for (index, value) in entries {
        // SAFETY: `index` was just returned by `push` on this thread.
        assert_eq!(unsafe { vec.get_unchecked(index) }, &value);
        assert_eq!(vec.get(index), Some(&value));
    }
}

/// Pushes across many threads, then verifies after the join that every
/// returned index still resolves and that indices are globally unique.
#[test]
#[cfg_attr(miri, ignore)]
fn random_concurrent_push() {
    let vec: Arc<ConcurrentAppendVec<u64>> = Arc::new(ConcurrentAppendVec::new());
    let results: Arc<Mutex<Vec<(usize, u64)>>> = Arc::new(Mutex::new(Vec::new()));

    random_test_threads(
        1000,
        8,
        || (Arc::clone(&vec), Arc::clone(&results)),
        move |rng, (vec, results)| {
            let value = rng.random_range(0..1_000_000u64);
            let index = vec.push(value);
            results.lock().unwrap().push((index, value));
        },
    );

    let results = results.lock().unwrap();
    let mut seen = HashSet::new();
    for &(index, value) in results.iter() {
        assert!(seen.insert(index), "indices are globally unique");
        assert_eq!(vec.get(index), Some(&value), "index resolves after the join");
    }
    assert_eq!(vec.len(), results.len(), "length counts every push");
}

/// A small concurrent smoke test kept light enough for Miri's data-race
/// detector: threads push and concurrently read back recorded indices,
/// exercising the per-block release/acquire publication and the bucket
/// allocation race.
#[test]
fn concurrent_push_and_get() {
    let vec: Arc<ConcurrentAppendVec<u64, 4>> = Arc::new(ConcurrentAppendVec::new());
    let recorded: Arc<Mutex<Vec<(usize, u64)>>> = Arc::new(Mutex::new(Vec::new()));

    let threads: Vec<_> = (0..4u64)
        .map(|thread| {
            let vec = Arc::clone(&vec);
            let recorded = Arc::clone(&recorded);
            std::thread::spawn(move || {
                for step in 0..25u64 {
                    let value = thread * 1000 + step;
                    let index = vec.push(value);
                    recorded.lock().unwrap().push((index, value));

                    // Read back an arbitrary already-recorded entry, which
                    // may belong to another thread.
                    let entry = {
                        let guard = recorded.lock().unwrap();
                        guard[(value as usize) % guard.len()]
                    };
                    assert_eq!(vec.get(entry.0), Some(&entry.1));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }

    let recorded = recorded.lock().unwrap();
    assert_eq!(vec.len(), recorded.len());
    for &(index, value) in recorded.iter() {
        assert_eq!(vec.get(index), Some(&value));
    }
}

/// Verifies that dropping (and clearing) the vector drops each written value
/// exactly once, even with the gaps that block reservation leaves behind.
#[test]
fn drops_each_written_value_once() {
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));

    {
        let vec: ConcurrentAppendVec<DropCounter, 4> = ConcurrentAppendVec::new();
        for _ in 0..10 {
            vec.push(DropCounter(Arc::clone(&drops)));
        }
        // Ten pushes over blocks of four leave two trailing gaps.
        assert_eq!(vec.len(), 10);
    }
    assert_eq!(
        drops.load(Ordering::Relaxed),
        10,
        "dropping frees every written value once"
    );

    drops.store(0, Ordering::Relaxed);
    let mut vec: ConcurrentAppendVec<DropCounter, 4> = ConcurrentAppendVec::new();
    for _ in 0..7 {
        vec.push(DropCounter(Arc::clone(&drops)));
    }
    vec.clear();
    assert_eq!(drops.load(Ordering::Relaxed), 7, "clear frees every written value once");
    assert_eq!(vec.len(), 0, "clear empties the vector");
    assert!(vec.get(0).is_none());

    // The cleared vector is reusable.
    assert_eq!(vec.push(DropCounter(Arc::clone(&drops))), 0);
}
