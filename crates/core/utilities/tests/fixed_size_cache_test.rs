use std::collections::HashMap;

use rand::RngExt;
use rand::rngs::StdRng;

use merc_utilities::CachePolicy;
use merc_utilities::FifoPolicy;
use merc_utilities::FixedSizeCache;
use merc_utilities::LruPolicy;
use merc_utilities::NoPolicy;
use merc_utilities::random_test;

#[test]
fn test_fixed_size_cache() {
    let mut cache = FixedSizeCache::new(2, FifoPolicy::new());

    assert!(cache.insert(1, "one").is_none());
    assert!(cache.insert(2, "two").is_none());

    // Should evict first entry
    assert!(cache.insert(3, "three").is_none());

    assert_eq!(cache.len(), 2);
    assert!(cache.get(&1).is_none());
    assert_eq!(cache.get(&2), Some(&"two"));
    assert_eq!(cache.get(&3), Some(&"three"));
}

#[test]
fn test_fixed_size_cache_unbounded() {
    let mut cache = FixedSizeCache::new(0, FifoPolicy::new());

    for i in 0..100 {
        assert!(cache.insert(i, i * i).is_none());
    }

    assert_eq!(cache.len(), 100);
    assert!((0..100).all(|i| cache.get(&i) == Some(&(i * i))));
}

#[test]
fn test_fixed_size_cache_replace_when_full() {
    let mut cache = FixedSizeCache::new(2, FifoPolicy::new());

    assert!(cache.insert(1, "one").is_none());
    assert!(cache.insert(2, "two").is_none());

    // Replacing a resident key must not evict another entry.
    assert_eq!(cache.insert(2, "TWO"), Some("two"));
    assert_eq!(cache.get(&1), Some(&"one"));
    assert_eq!(cache.get(&2), Some(&"TWO"));
}

#[test]
fn test_fixed_size_cache_lru_overwrite() {
    let mut cache = FixedSizeCache::new(2, LruPolicy::default());

    assert!(cache.insert(1, "one").is_none());
    assert!(cache.insert(2, "two").is_none());

    // Overwriting counts as an access, so key 2 is now the least recently
    // used and must be the one evicted.
    assert_eq!(cache.insert(1, "ONE"), Some("one"));
    assert!(cache.insert(3, "three").is_none());

    assert_eq!(cache.get(&1), Some(&"ONE"));
    assert!(cache.get(&2).is_none());
    assert_eq!(cache.get(&3), Some(&"three"));
}

#[test]
fn test_fixed_size_cache_no_policy_stays_bounded() {
    let mut cache = FixedSizeCache::new(3, NoPolicy);
    for i in 0..20 {
        assert!(cache.insert(i, i).is_none());
        assert_eq!(cache.len(), (i + 1).min(3));
        assert!(cache.contains_key(&i), "the inserted key {i} was evicted");
    }
}

/// Runs random operations on a cache with policy `P` and compares it with a model that keeps
/// the resident keys in eviction order. `touch_reorders` states whether an access moves a key to
/// the back of that order.
fn random_cache_against_model<P: CachePolicy<u32>>(rng: &mut StdRng, touch_reorders: bool) {
    let max_size = rng.random_range(1..5);
    let mut cache = FixedSizeCache::new(max_size, P::default());
    let mut order: Vec<u32> = Vec::new();
    let mut values: HashMap<u32, u32> = HashMap::new();

    let touch = |order: &mut Vec<u32>, key: u32| {
        if touch_reorders {
            let position = order.iter().position(|&k| k == key).unwrap();
            order.remove(position);
            order.push(key);
        }
    };

    for _ in 0..100 {
        let key = rng.random_range(0..8);
        match rng.random_range(0..20) {
            0..10 => {
                let value = rng.random();
                let expected = values.insert(key, value);
                if expected.is_some() {
                    touch(&mut order, key);
                } else {
                    if order.len() == max_size {
                        let evicted = order.remove(0);
                        values.remove(&evicted);
                    }
                    order.push(key);
                }
                assert_eq!(cache.insert(key, value), expected);
            }
            10..18 => {
                let expected = values.get(&key);
                if expected.is_some() {
                    touch(&mut order, key);
                }
                assert_eq!(cache.get(&key), expected);
            }
            18 => assert_eq!(cache.contains_key(&key), values.contains_key(&key)),
            _ => {
                cache.clear();
                order.clear();
                values.clear();
            }
        }

        assert_eq!(cache.len(), values.len());
        assert_eq!(cache.is_empty(), values.is_empty());
    }

    let mut entries: Vec<(u32, u32)> = cache.iter().map(|(&k, &v)| (k, v)).collect();
    let mut expected: Vec<(u32, u32)> = values.into_iter().collect();
    entries.sort_unstable();
    expected.sort_unstable();
    assert_eq!(entries, expected);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_fixed_size_cache_lru() {
    random_test(100, |rng| random_cache_against_model::<LruPolicy<u32>>(rng, true));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_fixed_size_cache_fifo() {
    random_test(100, |rng| random_cache_against_model::<FifoPolicy<u32>>(rng, false));
}
