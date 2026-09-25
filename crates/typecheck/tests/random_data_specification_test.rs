//! Randomized type-checking property test: every well-formed data specification
//! `random_data_specification` generates (acyclic by construction) is expected to type-check
//! without error.

use merc_syntax::UntypedDataSpecification;
use merc_syntax::random_data_specification;
use merc_typecheck::DataSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_data_specification_type_checks() {
    random_test(200, |rng| {
        let spec = random_data_specification(rng, 6, 3);
        let printed = format!("{spec}");

        let reparsed = UntypedDataSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("generated data specification failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = DataSpecification::from_untyped(reparsed) {
            panic!("generated data specification was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}
