use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use rand::RngExt;

use merc_sharedmutex::BfSharedMutex;
use merc_utilities::random_test_threads;
use merc_utilities::test_threads;

#[test]
#[cfg_attr(miri, ignore)]
fn test_random_bf_shared_mutex_exclusive() {
    let shared_number = BfSharedMutex::new(5);
    let num_iterations = 500;
    let num_threads = 3;

    test_threads(
        num_threads,
        || shared_number.clone(),
        move |number| {
            for _ in 0..num_iterations {
                *number.write().unwrap() += 5;
            }
        },
    );

    assert_eq!(*shared_number.write().unwrap(), num_threads * num_iterations * 5 + 5);
}

/// Readers must only observe fully pushed elements, and no push may be lost.
#[test]
#[cfg_attr(miri, ignore)]
fn test_random_bf_shared_mutex() {
    let shared_vector = BfSharedMutex::new(vec![]);
    let num_pushes = Arc::new(AtomicUsize::new(0));

    let num_threads = 20;
    let num_iterations = 5000;

    random_test_threads(
        num_iterations,
        num_threads,
        || (shared_vector.clone(), num_pushes.clone()),
        |rng, (shared_vector, num_pushes)| {
            if rng.random_bool(0.95) {
                // Read a random index.
                let read = shared_vector.read().unwrap();
                if !read.is_empty() {
                    let index = rng.random_range(0..read.len());
                    assert_eq!(read[index], 5);
                }
            } else {
                // Add a new vector element.
                shared_vector.write().unwrap().push(5);
                num_pushes.fetch_add(1, Ordering::Relaxed);
            }
        },
    );

    assert_eq!(shared_vector.read().unwrap().len(), num_pushes.load(Ordering::Relaxed));
}

/// A `try_write` that observes the inner registration mutex already locked must report
/// failure instead of handing out a second exclusive reference.
#[test]
#[cfg_attr(miri, ignore)]
fn test_bf_shared_mutex_try_write_fails_when_locked() {
    let shared_mutex = BfSharedMutex::new(0);
    let other = shared_mutex.clone();

    let guard = shared_mutex.write().unwrap();

    // While the write guard holds the inner mutex, no clone can acquire it.
    assert!(other.try_write().unwrap().is_none());
    assert!(shared_mutex.try_write().unwrap().is_none());

    drop(guard);

    // Once released, a non-blocking write succeeds again.
    let mut guard = other
        .try_write()
        .unwrap()
        .expect("try_write should succeed after unlock");
    *guard += 1;
    drop(guard);

    assert_eq!(*shared_mutex.read().unwrap(), 1);
}

/// Many threads racing through `try_write` must never lose an increment: each successful
/// attempt is guaranteed exclusive access, so retrying until success yields an exact count.
#[test]
#[cfg_attr(miri, ignore)]
fn test_concurrent_bf_shared_mutex_try_write() {
    let shared_number = BfSharedMutex::new(0);
    let num_iterations = 500;
    let num_threads = 4;

    test_threads(
        num_threads,
        || shared_number.clone(),
        move |number| {
            for _ in 0..num_iterations {
                // Retry until exclusive access is granted; a contended attempt returns None.
                loop {
                    if let Some(mut guard) = number.try_write().unwrap() {
                        *guard += 1;
                        break;
                    }
                }
            }
        },
    );

    assert_eq!(*shared_number.write().unwrap(), num_threads * num_iterations);
}
