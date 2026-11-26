use std::collections::HashSet;

use merc_utilities::GenerationCounter;
use merc_utilities::GenerationalIndex;

#[test]
fn test_default_index_is_reflexive() {
    // A default (sentinel) index must equal itself, otherwise `Eq` is
    // violated and it cannot be used as a hash-map key.
    let default = GenerationalIndex::<usize>::default();
    assert_eq!(default, default);

    let mut set = HashSet::new();
    set.insert(default);
    assert!(set.contains(&default));
}

#[test]
fn test_index_round_trips() {
    let mut counter = GenerationCounter::new();
    let first = counter.create_index(42usize);
    let other = counter.create_index(43usize);

    assert_eq!(*first, 42);
    assert_eq!(counter.get_index(first), 42);
    assert_eq!(counter.get_index(other), 43);
    assert_eq!(counter.recall_index(42usize), first);
    assert_ne!(first, other);
    assert!(counter.is_valid(first));
}

/// Recreating an index invalidates the earlier handles to it, and only the latest one is
/// recalled.
#[test]
#[cfg(debug_assertions)]
fn test_recreated_index_invalidates_old_handle() {
    let mut counter = GenerationCounter::new();
    let old = counter.create_index(42usize);
    let new = counter.create_index(42usize);

    assert!(!counter.is_valid(old));
    assert!(counter.is_valid(new));
    assert_eq!(counter.recall_index(42usize), new);
    assert_eq!(counter.get_index(new), 42);
}

/// A sentinel index never aliases a live index with the same raw value.
#[test]
#[cfg(debug_assertions)]
fn test_default_index_differs_from_live_index() {
    let mut counter = GenerationCounter::new();
    let live = counter.create_index(0usize);
    assert_ne!(GenerationalIndex::default(), live);
    assert_ne!(live, GenerationalIndex::default());
}

#[test]
#[should_panic(expected = "Comparing indices of different generations")]
#[cfg(debug_assertions)]
fn test_comparing_generations_panics() {
    let mut counter = GenerationCounter::new();
    let old = counter.create_index(42usize);
    let new = counter.create_index(42usize);

    let _ = old == new;
}

#[test]
#[should_panic(expected = "Attempting to access an invalid index")]
#[cfg(debug_assertions)]
fn test_get_index_of_stale_handle_panics() {
    let mut counter = GenerationCounter::new();
    let old = counter.create_index(42usize);
    counter.create_index(42usize);

    counter.get_index(old);
}

#[test]
#[should_panic(expected = "never created")]
#[cfg(debug_assertions)]
fn test_recall_of_unknown_index_panics() {
    let mut counter = GenerationCounter::new();
    counter.create_index(1usize);

    counter.recall_index(2usize);
}
