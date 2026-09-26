//! Randomized type-checking property test: a `random_pres_with_data_specification` PRES draws
//! predicate-variable parameters from a freshly generated data specification (structs,
//! containers, function sorts), exercising far more of the type checker than
//! `random_pres_test.rs`'s pure `Int` parameters do.

use rand::RngExt;

use merc_syntax::UntypedPres;
use merc_syntax::random_pres_with_data_specification;
use merc_typecheck::PresSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pres_with_data_specification_type_checks() {
    random_test(200, |rng| {
        let use_bounds = rng.random_bool(0.5);
        let pres = random_pres_with_data_specification(rng, 4, 2, 3, 4, 4, use_bounds);
        let printed = format!("{pres}");

        let reparsed = UntypedPres::parse(&printed)
            .unwrap_or_else(|e| panic!("generated PRES failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = PresSpecification::from_untyped(reparsed) {
            panic!("generated PRES was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}
