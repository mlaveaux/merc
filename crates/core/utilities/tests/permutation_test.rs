use rand::RngExt;
use rand::seq::SliceRandom;

use merc_utilities::is_valid_permutation;
use merc_utilities::random_test;

#[test]
fn test_is_valid_permutation() {
    assert!(is_valid_permutation(|i| i, 0));
    assert!(is_valid_permutation(|i| [2, 0, 1][i], 3));

    // Duplicate entries.
    assert!(!is_valid_permutation(|i| [0, 1, 2, 3, 4, 5, 6, 7, 8, 8][i], 10));
    // An entry out of range.
    assert!(!is_valid_permutation(|i| [0, 1, 3][i], 3));
}

/// A shuffled range is a permutation; overwriting one entry with another entry, or with a
/// value out of range, makes it invalid.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_is_valid_permutation() {
    random_test(100, |rng| {
        let len = rng.random_range(2..100);
        let mut permutation: Vec<usize> = (0..len).collect();
        permutation.shuffle(rng);
        assert!(is_valid_permutation(|i| permutation[i], len));

        let mut duplicate = permutation.clone();
        let (target, source) = (rng.random_range(0..len), rng.random_range(0..len));
        if target != source {
            duplicate[target] = duplicate[source];
            assert!(!is_valid_permutation(|i| duplicate[i], len));
        }

        let mut out_of_range = permutation;
        out_of_range[rng.random_range(0..len)] = rng.random_range(len..2 * len);
        assert!(!is_valid_permutation(|i| out_of_range[i], len));
    });
}
