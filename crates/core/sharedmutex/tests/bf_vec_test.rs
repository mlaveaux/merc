use std::thread;

use merc_sharedmutex::BfVec;

/// Concurrent pushes must neither lose nor duplicate elements.
#[test]
#[cfg_attr(miri, ignore)]
fn test_push() {
    let mut threads = vec![];

    let shared_vector = BfVec::<usize>::new();
    let num_threads = 10;
    let num_iterations = 10000;

    for t in 0..num_threads {
        let shared_vector = shared_vector.share();
        threads.push(thread::spawn(move || {
            for _ in 0..num_iterations {
                shared_vector.push(t);
            }
        }));
    }

    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(shared_vector.len(), num_threads * num_iterations);

    let mut occurrences = vec![0; num_threads];
    for i in 0..shared_vector.len() {
        occurrences[shared_vector.at(i)] += 1;
    }
    assert!(occurrences.iter().all(|&count| count == num_iterations));
}

/// Small enough for miri: pushes past two reallocations, then checks that `clear` drops the
/// elements once and that the vector is usable afterwards.
#[test]
fn test_clear_resets_length() {
    let vector = BfVec::<String>::new();
    for i in 0..20 {
        vector.push(i.to_string());
    }
    assert_eq!(vector.len(), 20);
    for i in 0..20 {
        assert_eq!(vector.at(i), i.to_string());
    }

    // Dropping the vector afterwards must not drop the elements again.
    vector.clear();
    assert!(vector.is_empty());

    vector.push("c".to_string());
    assert_eq!(vector.len(), 1);
    assert_eq!(vector.at(0), "c");
}
