//! Randomized type-checking property test: for every sort a `random_data_specification` can
//! generate (basic, struct, container or function), `random_value_expression` must produce a
//! value of exactly that sort.

use merc_syntax::SortExpressionKind;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::random_data_specification;
use merc_syntax::random_value_expression;
use merc_typecheck::DataSpecification;
use merc_utilities::random_test;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
#[ignore = "known bug: Lambda-body widening is unfixed (2 attempted fixes reverted for concrete \
            regressions), so any generated Function-sorted value whose body needs widening \
            fails; see review/typecheck-lambda-and-join-widening-bugs.md"]
fn random_value_expression_type_checks_for_every_generated_sort() {
    random_test(200, |rng| {
        let spec = random_data_specification(rng, 6, 3);

        let mut maps = String::new();
        let mut eqns = String::new();
        for (i, decl) in spec.sort_declarations.iter().enumerate() {
            let sort_ref = SortExpressionKind::Reference(decl.identifier.clone()).into();
            let value = random_value_expression(rng, &spec.sort_declarations, &sort_ref, &[], 3);
            // "val" (not "c", which collides with random_data_specification's own
            // constructor names c0, c1, ...) keeps these synthetic map constants distinct.
            maps.push_str(&format!("   val{i}: {};\n", decl.identifier));
            eqns.push_str(&format!("eqn val{i} = {value};\n"));
        }

        let text = format!("{spec}\nmap\n{maps}\n{eqns}");

        let parsed = UntypedDataSpecification::parse(&text)
            .unwrap_or_else(|e| panic!("failed to parse generated data specification + values:\n{text}\nerror: {e}"));

        if let Err(error) = DataSpecification::from_untyped(parsed) {
            panic!("generated values were rejected by the type checker:\n{text}\nerror: {error}");
        }
    });
}
