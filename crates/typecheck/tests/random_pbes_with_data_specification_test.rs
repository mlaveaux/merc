//! Randomized type-checking property test: a `random_pbes_with_data_specification` PBES draws
//! predicate-variable parameters from a freshly generated data specification (structs,
//! containers, function sorts), exercising far more of the type checker than
//! `random_pbes_test.rs`'s pure Bool/Nat parameters do.

use rand::RngExt;

use merc_syntax::UntypedPbes;
use merc_syntax::random_pbes_with_data_specification;
use merc_typecheck::PbesSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pbes_with_data_specification_type_checks() {
    random_test(200, |rng| {
        let use_quantifiers = rng.random_bool(0.5);
        let use_integers = rng.random_bool(0.5);
        let pbes = random_pbes_with_data_specification(rng, 4, 2, 3, 4, 4, use_quantifiers, use_integers);
        let printed = format!("{pbes}");

        let reparsed = UntypedPbes::parse(&printed)
            .unwrap_or_else(|e| panic!("generated PBES failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = PbesSpecification::from_untyped(reparsed) {
            panic!("generated PBES was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}
