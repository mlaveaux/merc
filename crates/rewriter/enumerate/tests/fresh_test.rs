use rand::RngExt;

use merc_data::SortExpression;
use merc_enumerate::FreshVariableGenerator;
use merc_utilities::random_test;

fn names(generator: &mut FreshVariableGenerator, count: usize) -> Vec<String> {
    (0..count)
        .map(|_| generator.generate(SortExpression::nat().copy()).name().to_string())
        .collect()
}

#[test]
fn test_generate_avoids_seeded_names() {
    let mut generator = FreshVariableGenerator::new("v", ["v0".to_string(), "v1".to_string()]);
    let fresh = generator.generate(SortExpression::bool().copy());
    assert_eq!(fresh.name(), "v2");
    assert_eq!(fresh.sort().protect(), SortExpression::bool());
}

/// The generator continues past the highest seeded index, not at the smallest unused one.
#[test]
fn test_generate_starts_past_the_highest_seeded_index() {
    let mut generator = FreshVariableGenerator::new("v", ["v5".to_string(), "v2".to_string()]);
    assert_eq!(names(&mut generator, 2), ["v6", "v7"]);
}

#[test]
fn test_generate_avoids_its_own_earlier_output() {
    let mut generator = FreshVariableGenerator::new("v", std::iter::empty());
    assert_eq!(names(&mut generator, 3), ["v0", "v1", "v2"]);
}

#[test]
fn test_reset_undoes_generated_names_but_keeps_the_seed() {
    let mut generator = FreshVariableGenerator::new("v", ["v0".to_string()]);
    assert_eq!(names(&mut generator, 2), ["v1", "v2"]);

    generator.reset();

    // The seed name is still avoided, but the generated ones are forgotten, so the same fresh
    // names are produced again.
    assert_eq!(names(&mut generator, 2), ["v1", "v2"]);
}

/// Only names that `generate` itself could produce for the prefix are taken into account: the
/// prefix followed by the canonical decimal digits of an index.
#[test]
fn test_seeding_only_counts_canonical_names_with_the_prefix() {
    for (used, expected) in [
        ("w5", "v0"),
        ("v", "v0"),
        ("v1a", "v0"),
        ("v-1", "v0"),
        // `generate` never writes "v012", so it does not shadow "v12".
        ("v012", "v0"),
        ("v12", "v13"),
        ("v0", "v1"),
    ] {
        let mut generator = FreshVariableGenerator::new("v", [used.to_string()]);
        assert_eq!(names(&mut generator, 1), [expected], "seeded with {used}");
    }

    // A longer prefix that starts with the shorter one.
    let mut generator = FreshVariableGenerator::new("var", ["v7".to_string(), "var3".to_string()]);
    assert_eq!(names(&mut generator, 1), ["var4"]);
}

/// Generated names never collide with a seeded name or with each other, and follow the documented
/// order: one past the highest canonical index of the prefix, counting up.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_generate_is_fresh() {
    const PREFIXES: [&str; 3] = ["v", "va", "w"];

    random_test(100, |rng| {
        let used: Vec<String> = (0..rng.random_range(0..10))
            .map(|_| {
                let prefix = PREFIXES[rng.random_range(0..PREFIXES.len())];
                if rng.random_bool(0.2) {
                    format!("{prefix}0{}", rng.random_range(0..20))
                } else {
                    format!("{prefix}{}", rng.random_range(0..20))
                }
            })
            .collect();

        let start = used
            .iter()
            .filter_map(|name| name.strip_prefix("v"))
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .filter(|digits| digits.len() == 1 || !digits.starts_with('0'))
            .map(|digits| digits.parse::<u64>().unwrap() + 1)
            .max()
            .unwrap_or(0);

        let mut generator = FreshVariableGenerator::new("v", used.clone());
        let count = rng.random_range(1..10);
        let generated = names(&mut generator, count);
        let expected: Vec<String> = (start..start + count as u64).map(|index| format!("v{index}")).collect();
        assert_eq!(generated, expected, "seeded with {used:?}");
        assert!(generated.iter().all(|name| !used.contains(name)));
    });
}
