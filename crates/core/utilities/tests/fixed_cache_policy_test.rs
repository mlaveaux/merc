use std::collections::HashMap;

use merc_utilities::CachePolicy;
use merc_utilities::FifoPolicy;
use merc_utilities::LruPolicy;
use merc_utilities::NoPolicy;

#[test]
fn test_no_policy() {
    let mut policy = NoPolicy;
    let mut cache = HashMap::new();
    assert_eq!(policy.replacement_candidate(&cache), None);

    cache.insert(1, "one");
    cache.insert(2, "two");

    let candidate = policy.replacement_candidate(&cache).unwrap();
    assert!(
        cache.contains_key(&candidate),
        "candidate {candidate} is not in the cache"
    );
}

#[test]
fn test_fifo_policy() {
    let mut policy = FifoPolicy::default();
    let mut cache = HashMap::new();

    for key in 1..=3 {
        cache.insert(key, ());
        policy.inserted(key);
    }

    // Accesses do not change the order.
    policy.touch(&1);

    // A key that left the cache is skipped.
    cache.remove(&2);
    for expected in [1, 3] {
        assert_eq!(policy.replacement_candidate(&cache), Some(expected));
        cache.remove(&expected);
    }
    assert_eq!(policy.replacement_candidate(&cache), None);
}

#[test]
fn test_lru_policy() {
    let mut policy = LruPolicy::default();
    let mut cache = HashMap::new();

    // Insert elements
    cache.insert(1, "one");
    policy.inserted(1);
    cache.insert(2, "two");
    policy.inserted(2);

    // Touch first element
    policy.touch(&1);

    // Least recently used should be 2
    assert_eq!(policy.replacement_candidate(&cache), Some(2));
}

#[test]
fn test_lru_policy_order_after_eviction() {
    let mut policy = LruPolicy::default();
    let mut cache = HashMap::new();
    for key in 1..=3 {
        cache.insert(key, ());
        policy.inserted(key);
    }

    policy.touch(&1);
    assert_eq!(policy.replacement_candidate(&cache), Some(2));
    cache.remove(&2);

    cache.insert(4, ());
    policy.inserted(4);
    policy.touch(&3);

    // Recency order is now 1, 4, 3.
    for expected in [1, 4, 3] {
        assert_eq!(policy.replacement_candidate(&cache), Some(expected));
        cache.remove(&expected);
    }
    assert_eq!(policy.replacement_candidate(&cache), None);
}

#[test]
fn test_policy_clear_forgets_keys() {
    let cache: HashMap<i32, ()> = HashMap::from([(1, ()), (2, ())]);

    let mut fifo = FifoPolicy::default();
    let mut lru = LruPolicy::default();
    fifo.inserted(1);
    lru.inserted(1);
    fifo.clear();
    lru.clear();
    fifo.inserted(2);
    lru.inserted(2);

    assert_eq!(fifo.replacement_candidate(&cache), Some(2));
    assert_eq!(lru.replacement_candidate(&cache), Some(2));
}
