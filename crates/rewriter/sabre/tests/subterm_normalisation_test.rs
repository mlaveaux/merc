//! Subterms below a symbol that no rule mentions must still be normalised, and every engine agrees.
//!
//! Known defect: [SabreRewriter] only normalises below symbols that occur in the rules. With the
//! single rule `f(a) -> b`, `g(f(a))` stays as it is, and so does `g(a, f(a))`, while the innermost
//! and naive rewriters give `g(b)` and `g(a, b)`. The test asserts the correct result and is marked
//! `should_panic` so that it keeps passing until the defect is fixed; remove the attribute then.

use ahash::AHashSet;

use merc_aterm::ATerm;
use merc_data::DataExpression;
use merc_data::to_untyped_data_expression;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_sabre::SabreRewriter;

fn term(input: &str, variables: &[&str]) -> DataExpression {
    let vars: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    to_untyped_data_expression(ATerm::from_string(input).unwrap(), Some(&vars))
}

/// `f(a) -> b` is the only rule, so `g` and `h` have no rules at all.
fn spec() -> RewriteSpecification {
    RewriteSpecification::new(vec![Rule::new(term("f(a)", &[]), term("b", &[]))])
}

fn check(input: &str, expected: &str) {
    let spec = spec();
    let input = term(input, &[]);
    let expected = term(expected, &[]);

    assert_eq!(InnermostRewriter::new(&spec).rewrite(&input), expected, "innermost");
    assert_eq!(NaiveRewriter::new(&spec).rewrite(&input), expected, "naive");
    assert_eq!(SabreRewriter::new(&spec).rewrite(&input), expected, "sabre");
}

#[test]
#[should_panic(expected = "sabre")]
fn test_rewrites_below_a_symbol_without_rules() {
    check("f(a)", "b");
    check("g(f(a))", "g(b)");
    check("g(h(f(a)), f(a))", "g(h(b), b)");
}
