//! Tests for the equivalence classes of non-linear left-hand sides, see
//! [merc_sabre::matching::nonlinear], and for the engines applying such rules.

use ahash::AHashSet;

use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;
use merc_sabre::matching::nonlinear::EquivalenceClass;
use merc_sabre::matching::nonlinear::check_equivalence_classes;
use merc_sabre::matching::nonlinear::derive_equivalence_classes;
use merc_sabre::test_utility::create_rewrite_rule;
use merc_sabre::utilities::DataPosition;

/// Parses `input` treating every name in `variables` as a variable.
fn term(input: &str, variables: &[&str]) -> DataExpression {
    let variables: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

#[test]
fn test_derive_equivalence_classes_of_a_repeated_variable() {
    let rule = create_rewrite_rule("f(x, h(x), y)", "result", &["x", "y"]).unwrap();

    // `y` occurs once, so only `x` gets a class.
    assert_eq!(
        derive_equivalence_classes(&rule),
        vec![EquivalenceClass {
            variable: DataVariable::new("x"),
            positions: vec![DataPosition::new(&[1]), DataPosition::new(&[2, 1])]
        }]
    );
}

#[test]
fn test_linear_rule_has_no_equivalence_classes() {
    let rule = create_rewrite_rule("f(x, h(y))", "result", &["x", "y"]).unwrap();

    assert_eq!(derive_equivalence_classes(&rule), vec![]);
}

#[test]
fn test_check_equivalence_classes() {
    let classes = derive_equivalence_classes(&create_rewrite_rule("f(x, h(x))", "result", &["x"]).unwrap());

    assert!(check_equivalence_classes(&term("f(a(b), h(a(b)))", &[]), &classes));
    assert!(!check_equivalence_classes(&term("f(a(b), h(a(c)))", &[]), &classes));
    assert!(!check_equivalence_classes(&term("f(a, h(b))", &[]), &classes));
}

/// `f(x, h(x)) = r` applies only where both occurrences of `x` are the same term, also below
/// the root and after an argument has been rewritten.
#[test]
fn test_non_linear_rule_applies_only_to_equal_subterms() {
    let spec = RewriteSpecification::new(vec![
        create_rewrite_rule("f(x, h(x))", "r", &["x"]).unwrap(),
        create_rewrite_rule("c", "a", &[]).unwrap(),
    ]);

    let cases = [
        ("f(a, h(a))", "r"),
        ("f(a, h(b))", "f(a, h(b))"),
        ("f(a, h(c))", "r"),
        ("f(f(b, h(b)), b)", "f(r, b)"),
    ];

    for (input, expected) in cases {
        let input = term(input, &[]);
        let expected = term(expected, &[]);
        assert_eq!(
            InnermostRewriter::new(&spec).rewrite(&input),
            expected,
            "innermost on {input}"
        );
        assert_eq!(SabreRewriter::new(&spec).rewrite(&input), expected, "sabre on {input}");
        assert_eq!(NaiveRewriter::new(&spec).rewrite(&input), expected, "naive on {input}");
    }
}
