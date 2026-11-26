use rand::RngExt;

use merc_collections::VecBag;
use merc_utilities::random_test;

/// Returns the number of occurrences of `value` in `values`.
fn count(values: &[u32], value: u32) -> usize {
    values.iter().filter(|&&element| element == value).count()
}

#[test]
fn test_random_vecbag_difference() {
    random_test(100, |rng| {
        let vec1: Vec<u32> = (0..rng.random_range(0..20)).map(|_| rng.random_range(0..10)).collect();
        let vec2: Vec<u32> = (0..rng.random_range(0..20)).map(|_| rng.random_range(0..10)).collect();

        let bag1 = VecBag::from_vec(vec1.clone());
        let bag2 = VecBag::from_vec(vec2.clone());

        // Every value occurs as often as in the first bag minus the second bag, in sorted order.
        let expected_difference: Vec<u32> = (0..10)
            .flat_map(|value| std::iter::repeat_n(value, count(&vec1, value).saturating_sub(count(&vec2, value))))
            .collect();
        let difference: Vec<u32> = bag1.difference(&bag2).cloned().collect();
        assert_eq!(difference, expected_difference);

        let expected_subset = (0..10).all(|value| count(&vec1, value) <= count(&vec2, value));
        assert_eq!(bag1.is_subset(&bag2), expected_subset);
    })
}
