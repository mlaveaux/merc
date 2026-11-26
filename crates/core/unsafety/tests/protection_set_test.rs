use rand::RngExt;

use merc_unsafety::ProtectionIndex;
use merc_unsafety::ProtectionSet;
use merc_utilities::random_test;

/// Protects and unprotects random values, checking against a model of the live roots that
/// every live index resolves to its own value, and that freed slots are reused without
/// disturbing live ones.
#[test]
#[cfg_attr(miri, ignore)]
fn test_random_protection_set() {
    random_test(100, |rng| {
        let mut protection_set = ProtectionSet::<usize>::new();
        let mut live: Vec<(ProtectionIndex, usize)> = Vec::new();

        for _ in 0..5000 {
            let value = rng.random_range(0..1000);
            live.push((protection_set.protect(value), value));
        }

        // Unprotect a random half of the roots.
        for _ in 0..2500 {
            let (index, value) = live.swap_remove(rng.random_range(0..live.len()));
            assert_eq!(protection_set[index], value);
            // SAFETY: each recorded index is protected exactly once and removed from `live`
            // here, so it is never unprotected twice.
            unsafe {
                protection_set.unprotect(index);
            }
        }

        // Protect more to reuse the freed slots.
        for _ in 0..1000 {
            let value = rng.random_range(0..1000);
            live.push((protection_set.protect(value), value));
        }

        for (index, value) in &live {
            assert!(
                protection_set.contains_root(*index),
                "All indices that are not unprotected should occur in the protection set"
            );
            assert_eq!(protection_set[*index], *value);
        }

        assert_eq!(protection_set.len(), live.len());

        let mut expected: Vec<usize> = live.iter().map(|(_, value)| *value).collect();
        let mut actual: Vec<usize> = protection_set.iter().map(|(_, value)| *value).collect();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(actual, expected, "iter visits exactly the live roots");

        assert_eq!(protection_set.number_of_insertions(), 6000);
        assert!(protection_set.maximum_size() >= 5000);
        assert!(!protection_set.is_empty());
    });
}

/// Small enough for miri: owned values are dropped on unprotect and on drop of the set.
#[test]
fn test_protection_set_basic() {
    let mut set = ProtectionSet::<String>::new();

    let idx1 = set.protect(String::from("value1"));
    let idx2 = set.protect(String::from("value2"));

    assert!(set.contains_root(idx1));
    assert!(set.contains_root(idx2));
    assert_eq!(set[idx1], "value1");
    assert_eq!(set[idx2], "value2");
    assert_eq!(set.len(), 2);

    // SAFETY: `idx1` is currently protected and is unprotected only once.
    unsafe {
        set.unprotect(idx1);
    }
    assert!(!set.contains_root(idx1));
    assert!(set.contains_root(idx2));
    assert_eq!(set.len(), 1);

    // Re-use the freed slot without disturbing the other root.
    let idx3 = set.protect(String::from("value3"));
    assert_eq!(set[idx3], "value3");
    assert_eq!(set[idx2], "value2");
    assert_eq!(set.len(), 2);
}
