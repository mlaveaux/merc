use std::collections::HashSet;

use rand::RngExt;

use merc_utilities::FreshNameGenerator;
use merc_utilities::random_test;

#[test]
fn test_generate_avoids_used_and_generated_names() {
    let mut generator = FreshNameGenerator::new(["X0".to_string()]);
    assert_eq!(generator.generate("X"), "X1");
    assert_eq!(generator.generate("X"), "X2");
    assert_eq!(generator.generate("Y"), "Y0");
}

#[test]
fn test_generate_avoids_names_generated_for_another_base() {
    let mut generator = FreshNameGenerator::new([]);
    assert_eq!(generator.generate("X1"), "X10");
    for _ in 0..10 {
        generator.generate("X");
    }
    assert_eq!(generator.generate("X"), "X11");
}

/// Every generated name is `<base><i>` for the smallest `i` that is neither in the initial set
/// nor generated before, for bases that are prefixes of each other.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_generate_returns_smallest_fresh_name() {
    const BASES: [&str; 4] = ["X", "X1", "X11", "Y"];

    random_test(100, |rng| {
        let mut taken: HashSet<String> = (0..rng.random_range(0..10))
            .map(|_| format!("{}{}", BASES[rng.random_range(0..BASES.len())], rng.random_range(0..15)))
            .collect();
        let mut generator = FreshNameGenerator::new(taken.clone());

        for _ in 0..50 {
            let base = BASES[rng.random_range(0..BASES.len())];
            let expected = (0..)
                .map(|index| format!("{base}{index}"))
                .find(|name| !taken.contains(name))
                .unwrap();

            assert_eq!(generator.generate(base), expected);
            taken.insert(expected);
        }
    });
}
