//! Tests for the rules the engines can apply, see [merc_sabre::is_supported_rule]: a rule is
//! dropped, leaving the terms it would rewrite in place, when it is outside what an engine can
//! instantiate.

use ahash::AHashSet;

use merc_data::BinderType;
use merc_data::DataAbstraction;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::DataWhereClause;
use merc_data::DataWhrDecl;
use merc_sabre::Condition;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_sabre::SabreRewriter;
use merc_sabre::is_supported_rule;

/// Parses `input` treating `x` and `y` as variables.
fn term(input: &str) -> DataExpression {
    let variables: AHashSet<String> = ["x", "y"].iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

fn rule(lhs: &str, rhs: &str) -> Rule {
    Rule::new(term(lhs), term(rhs))
}

/// `g(x) = binder(y. p(x, y))` for the given binder.
fn binder_rule(binder: BinderType) -> Rule {
    let y = DataVariable::from(term("y"));
    Rule::new(
        term("g(x)"),
        DataAbstraction::new(binder, std::slice::from_ref(&y), term("p(x, y)")).into(),
    )
}

/// The engines rewrite with `rules` plus `h(x) = x`, which shows that the specification is
/// used at all. The set-automaton rewriter only rewrites below function symbols that occur in
/// a supported rule, so `known(x) = pair(f(x), g(x))` adds `f` and `g`.
fn spec_with(rules: Vec<Rule>) -> RewriteSpecification {
    let mut rules = rules;
    rules.push(rule("h(x)", "x"));
    rules.push(rule("known(x)", "pair(f(x), g(x))"));
    RewriteSpecification::new(rules)
}

/// Asserts that all three engines rewrite `input` to `expected`.
fn assert_all_engines(spec: &RewriteSpecification, input: &DataExpression, expected: &DataExpression) {
    assert_eq!(&InnermostRewriter::new(spec).rewrite(input), expected, "innermost");
    assert_eq!(&SabreRewriter::new(spec).rewrite(input), expected, "sabre");
    assert_eq!(&NaiveRewriter::new(spec).rewrite(input), expected, "naive");
}

#[test]
fn test_rule_with_bound_variables_is_supported() {
    assert!(is_supported_rule(&rule("f(x)", "x")));

    let conditional = Rule::with_condition(
        vec![Condition::new(term("x"), term("a"), true)],
        term("f(x)"),
        term("x"),
    );
    assert!(is_supported_rule(&conditional));
}

/// `f(x) = y` is well-typed as an equation, but `y` has no value when `f(x)` matches.
#[test]
fn test_unbound_right_hand_side_variable_is_unsupported() {
    let unbound = rule("f(x)", "y");
    assert!(!is_supported_rule(&unbound));

    assert_all_engines(&spec_with(vec![unbound]), &term("f(h(a))"), &term("f(a)"));
}

#[test]
fn test_unbound_condition_variable_is_unsupported() {
    let unbound = Rule::with_condition(
        vec![Condition::new(term("y"), term("x"), true)],
        term("f(x)"),
        term("x"),
    );
    assert!(!is_supported_rule(&unbound));

    assert_all_engines(&spec_with(vec![unbound]), &term("f(h(a))"), &term("f(a)"));
}

/// A where clause in the right-hand side is not supported, even when it binds every variable.
#[test]
fn test_where_clause_in_right_hand_side_is_unsupported() {
    let y = DataVariable::from(term("y"));
    let mut where_rule = rule("g(x)", "x");
    where_rule.rhs = DataWhereClause::new(term("p(x, y)"), &[DataWhrDecl::new(y, term("x"))]).into();
    assert!(!is_supported_rule(&where_rule));

    assert_all_engines(&spec_with(vec![where_rule]), &term("g(h(a))"), &term("g(a)"));
}

/// Only the innermost rewriter instantiates a quantifier in a right-hand side; the other
/// engines drop such a rule.
#[test]
fn test_quantifier_in_right_hand_side_is_only_applied_by_innermost() {
    let exists = binder_rule(BinderType::Exists);
    assert!(!is_supported_rule(&exists));

    let spec = spec_with(vec![exists]);
    let input = term("g(h(a))");
    let y = DataVariable::from(term("y"));
    let instance: DataExpression =
        DataAbstraction::new(BinderType::Exists, std::slice::from_ref(&y), term("p(a, y)")).into();

    assert_eq!(InnermostRewriter::new(&spec).rewrite(&input), instance);
    assert_eq!(SabreRewriter::new(&spec).rewrite(&input), term("g(a)"));
    assert_eq!(NaiveRewriter::new(&spec).rewrite(&input), term("g(a)"));
}

/// A lambda in a right-hand side is never instantiated, since its reduction is not implemented.
#[test]
fn test_lambda_in_right_hand_side_is_unsupported() {
    let lambda = binder_rule(BinderType::Lambda);
    assert!(!is_supported_rule(&lambda));

    assert_all_engines(&spec_with(vec![lambda]), &term("g(h(a))"), &term("g(a)"));
}
