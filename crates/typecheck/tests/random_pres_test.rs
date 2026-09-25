//! Randomized type-checking property test: every well-formed PRES `random_pres` generates is
//! expected to type-check without error, exercising far more constructor/operator combinations
//! than the hand-written examples in `pres_specification_test.rs`.

use rand::RngExt;

use merc_syntax::UntypedPres;
use merc_syntax::random_pres;
use merc_typecheck::PresSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pres_type_checks() {
    random_test(200, |rng| {
        let use_bounds = rng.random_bool(0.5);
        let pres = random_pres(rng, 3, 4, 4, use_bounds);
        let printed = format!("{pres}");

        let reparsed = UntypedPres::parse(&printed)
            .unwrap_or_else(|e| panic!("generated PRES failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = PresSpecification::from_untyped(reparsed) {
            panic!("generated PRES was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}
