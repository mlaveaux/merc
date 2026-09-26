//! Randomized type-checking property test: every well-formed process specification
//! `make_process_specification`/`make_process_specification_with_data_specification` generates is
//! expected to type-check without error. The latter draws process-variable parameters from a
//! freshly generated data specification (structs, containers, function sorts), exercising far more
//! of the type checker than the former's pure `Bool`/`Nat` parameters do.

use rand::RngExt;

use merc_syntax::UntypedProcessSpecification;
use merc_syntax::make_process_specification;
use merc_syntax::make_process_specification_with_data_specification;
use merc_typecheck::ProcessSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_process_specification_type_checks() {
    random_test(200, |rng| {
        let use_integers = rng.random_bool(0.5);
        let spec = make_process_specification(rng, 3, 4, use_integers);
        let printed = format!("{spec}");

        let reparsed = UntypedProcessSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("generated process spec failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = ProcessSpecification::from_untyped(reparsed) {
            panic!("generated process spec was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_process_specification_with_data_specification_type_checks() {
    random_test(200, |rng| {
        let use_integers = rng.random_bool(0.5);
        let spec = make_process_specification_with_data_specification(rng, 4, 2, 3, 4, use_integers);
        let printed = format!("{spec}");

        let reparsed = UntypedProcessSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("generated process spec failed to reparse:\n{printed}\nerror: {e}"));

        if let Err(error) = ProcessSpecification::from_untyped(reparsed) {
            panic!("generated process spec was rejected by the type checker:\n{printed}\nerror: {error}");
        }
    });
}
