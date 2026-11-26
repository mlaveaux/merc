//! Tests that [SabreRewriter] normalises the arguments of a duplicating rule before applying
//! it, so that the copies do not each have to be rewritten again.

use ahash::AHashSet;

use merc_data::DataExpression;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;
use merc_sabre::test_utility::create_rewrite_rule;

/// Parses the closed term `input`.
fn term(input: &str) -> DataExpression {
    DataExpression::from_string_untyped(input, &AHashSet::new()).unwrap()
}

/// `dup(x) = pair(x, x)` duplicates its argument; `slow(n)` takes `n + 1` steps to reach `done`.
fn spec() -> RewriteSpecification {
    RewriteSpecification::new(vec![
        create_rewrite_rule("dup(x)", "pair(x, x)", &["x"]).unwrap(),
        create_rewrite_rule("keep(x)", "wrap(x)", &["x"]).unwrap(),
        create_rewrite_rule("slow(d0)", "done", &[]).unwrap(),
        create_rewrite_rule("slow(s(n))", "slow(n)", &["n"]).unwrap(),
    ])
}

#[test]
fn test_duplicating_rule_rewrites_its_argument_once() {
    let spec = spec();
    let input = term("dup(slow(s(s(s(d0)))))");

    let (result, sabre_stats) = SabreRewriter::new(&spec).rewrite_with_statistics(&input);
    assert_eq!(result, term("pair(done, done)"));

    // One step for `dup` and four for `slow`, as for the innermost strategy; rewriting each
    // copy of `slow(...)` separately would take nine.
    let (_, inner_stats) = InnermostRewriter::new(&spec).rewrite_with_statistics(&input);
    assert_eq!(inner_stats.rewrite_steps, 5);
    assert_eq!(sabre_stats.rewrite_steps, 5);
}

/// A rule that does not duplicate is applied as is, so its argument is still rewritten once.
#[test]
fn test_non_duplicating_rule_rewrites_its_argument_once() {
    let spec = spec();
    let input = term("keep(slow(s(s(s(d0)))))");

    let (result, stats) = SabreRewriter::new(&spec).rewrite_with_statistics(&input);
    assert_eq!(result, term("wrap(done)"));
    assert_eq!(stats.rewrite_steps, 5);
}
