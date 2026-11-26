use std::sync::Arc;
use std::thread;

use merc_utilities::ShardedCounter;

#[test]
fn test_single_thread() {
    let counter = ShardedCounter::new();
    assert_eq!(counter.get(), 0);

    counter.increment();
    counter.add(41);
    assert_eq!(counter.get(), 42);
}

/// The total sums the shards of every thread, and a reset clears all of them, not only the shard
/// of the calling thread.
#[test]
fn test_reset_clears_every_shard() {
    let mut counter = ShardedCounter::new();
    counter.add(10);
    thread::scope(|scope| {
        for value in [1, 2, 3] {
            let counter = &counter;
            scope.spawn(move || counter.add(value));
        }
    });
    assert_eq!(counter.get(), 16);

    counter.reset();
    assert_eq!(counter.get(), 0);

    counter.increment();
    assert_eq!(counter.get(), 1);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_concurrent_increments() {
    const THREADS: u64 = 8;
    const PER_THREAD: u64 = 100_000;

    let counter = Arc::new(ShardedCounter::new());
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let counter = Arc::clone(&counter);
            thread::spawn(move || {
                for _ in 0..PER_THREAD {
                    counter.increment();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // No increments are lost: each thread owns its own shard.
    assert_eq!(counter.get(), THREADS * PER_THREAD);
}
