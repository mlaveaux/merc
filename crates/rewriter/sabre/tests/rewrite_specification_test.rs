//! Tests for building a [RewriteSpecification] from a lowered mCRL2 data specification.

use merc_data::Mcrl2DataSpecification;
use merc_lowering::lower_data_specification;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// Parses and type-checks the given mCRL2 data specification text.
fn lower(source: &str) -> Mcrl2DataSpecification {
    let untyped = UntypedDataSpecification::parse(source).unwrap();
    let data_spec = DataSpecification::from_untyped(untyped).unwrap();
    lower_data_specification(&data_spec)
}

/// Returns the rules of `spec` whose left-hand side has head symbol `f`.
fn rules_for_f(spec: &RewriteSpecification) -> Vec<&Rule> {
    spec.rewrite_rules()
        .iter()
        .filter(|rule| rule.lhs.to_string().starts_with("f("))
        .collect()
}

#[test]
#[cfg_attr(miri, ignore)] // miri is too slow to run this test
fn test_from_data_specification_unconditional_equation() {
    let spec = RewriteSpecification::from_data_specification(&lower(
        "map f: Nat -> Nat;
         var x: Nat;
         eqn f(x) = x;",
    ));

    let rules = rules_for_f(&spec);
    assert_eq!(rules.len(), 1);
    assert!(rules[0].conditions.is_empty());
    assert_eq!(rules[0].lhs.to_string(), "f(x)");
    assert_eq!(rules[0].rhs.to_string(), "x");
}

#[test]
#[cfg_attr(miri, ignore)] // miri is too slow to run this test
fn test_from_data_specification_conditional_equation() {
    let spec = RewriteSpecification::from_data_specification(&lower(
        "map f: Nat -> Nat;
         var x: Nat;
         eqn x == 0 -> f(x) = x;",
    ));

    let rules = rules_for_f(&spec);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].lhs.to_string(), "f(x)");
    assert_eq!(rules[0].rhs.to_string(), "x");
    assert_eq!(rules[0].conditions.len(), 1);
    assert!(rules[0].conditions[0].equality);
    assert_eq!(rules[0].conditions[0].lhs.to_string(), "==(x, @c0)");
    assert_eq!(rules[0].conditions[0].rhs.to_string(), "true");
}
