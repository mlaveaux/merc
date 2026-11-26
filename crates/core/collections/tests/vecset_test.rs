use itertools::Itertools;
use rand::RngExt;

use merc_collections::VecSet;
use merc_utilities::random_test;

#[test]
fn test_random_vecset_difference() {
    random_test(100, |rng| {
        let vec1: Vec<u32> = (0..rng.random_range(0..20)).map(|_| rng.random_range(0..100)).collect();
        let vec2: Vec<u32> = (0..rng.random_range(0..20)).map(|_| rng.random_range(0..100)).collect();

        let set1 = VecSet::from_vec(vec1.clone());
        let set2 = VecSet::from_vec(vec2.clone());

        let difference: Vec<u32> = set1.difference(&set2).cloned().collect();
        let expected_difference: Vec<u32> = vec1
            .iter()
            .copied()
            .filter(|x| !vec2.contains(x))
            .sorted()
            .dedup()
            .collect();
        assert_eq!(difference, expected_difference);

        assert_eq!(set1.is_subset(&set2), vec1.iter().all(|x| vec2.contains(x)));
    })
}
