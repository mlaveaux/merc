//! A non-linear rule whose first matching occurrence still needs rewriting must reach the normal form.
//!
//! Known defect: with `f(x, h(x)) = r` and `c = a`, [SabreRewriter] returns `f(r, h(r))` for
//! `f(f(a, h(a)), h(r))` and `f(a, h(a))` for `f(c, h(a))`, where the innermost and naive
//! rewriters return `r`. The test asserts the correct result and is marked `should_panic` so that it
//! keeps passing until the defect is fixed; remove the attribute then.

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

fn check(input: &str, expected: &str) {
    let spec = RewriteSpecification::new(vec![
        Rule::new(term("f(x, h(x))", &["x"]), term("r", &[])),
        Rule::new(term("c", &[]), term("a", &[])),
    ]);
    let input = term(input, &[]);
    let expected = term(expected, &[]);

    assert_eq!(InnermostRewriter::new(&spec).rewrite(&input), expected, "innermost");
    assert_eq!(NaiveRewriter::new(&spec).rewrite(&input), expected, "naive");
    assert_eq!(SabreRewriter::new(&spec).rewrite(&input), expected, "sabre");
}

#[test]
#[should_panic(expected = "sabre")]
fn test_first_occurrence_that_needs_rewriting() {
    check("f(f(a, h(a)), h(r))", "r");
    check("f(c, h(a))", "r");
}
