//! Randomized tests on generated specifications.

use rand::RngExt;

use merc_linearisation::LinearisationOptions;
use merc_syntax::RandomLinearisationOptions;
use merc_syntax::random_linearisation_specification;
use merc_typecheck::ProcessSpecification;
use merc_utilities::random_test;

/// Linearisation either succeeds or reports an error; it never panics, whatever subset of the
/// language the generated specification uses.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_specification_never_panics() {
    random_test(200, |rng| {
        let options = RandomLinearisationOptions {
            equation_count: rng.random_range(1..=4),
            depth: rng.random_range(1..=4),
            components: rng.random_range(1..=3),
            operators: rng.random_range(0..=3),
        };
        let spec = random_linearisation_specification(rng, options);
        let spec = ProcessSpecification::from_untyped(spec)
            .and_then(|mut spec| spec.lower())
            .expect("generated specifications type check");
        let _ = merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default());
    });
}
