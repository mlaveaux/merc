//! Adversarial tests for `SetAutomaton` construction (`crates/sabre/src/set_automaton/automaton.rs`),
//! specifically for `State::new`'s `label.unwrap()`, which the code comments assume always finds a
//! root-anchored match goal (`announcement.position.is_empty()`) in every state's goal set.

use ahash::AHashSet;

use merc_data::DataExpression;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;
use merc_sabre::test_utility::create_rewrite_rule;

fn term(input: &str, variables: &[&str]) -> DataExpression {
    let variables: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

/// A nested rule (`f(g(x)) = x`) forces `State::new` to be reached through a chain of
/// "fresh" match goals seeded by `add_fresh_match_goals` (see the comment on that function),
/// while a second, unrelated rule (`h(y) = y`) exercises the "discarded" classification path
/// that can strip the original root-tracked match goal out of a state's goal set before a
/// later transition needs to build a new state from what is left.
#[test]
fn test_nested_rule_with_sibling_rule_builds_automaton() {
    let spec = RewriteSpecification::new(vec![
        create_rewrite_rule("f(g(x))", "x", &["x"]).unwrap(),
        create_rewrite_rule("h(y)", "y", &["y"]).unwrap(),
    ]);

    // Constructing the automaton must not panic.
    let _sabre = SabreRewriter::new(&spec);
    let _innermost = InnermostRewriter::new(&spec);
    let _naive = NaiveRewriter::new(&spec);
}

/// Same shape as above but with three mutually unrelated rules and a deeper nesting, to widen
/// the set of "fresh" positions competing for a single state's match-goal partition.
#[test]
fn test_multiple_nested_rules_build_automaton() {
    let spec = RewriteSpecification::new(vec![
        create_rewrite_rule("f(g(x))", "x", &["x"]).unwrap(),
        create_rewrite_rule("h(k(y))", "y", &["y"]).unwrap(),
        create_rewrite_rule("p(q(z))", "z", &["z"]).unwrap(),
        create_rewrite_rule("a", "b", &[]).unwrap(),
    ]);

    let mut sabre = SabreRewriter::new(&spec);
    let mut inner = InnermostRewriter::new(&spec);

    // Normal form: f(g(h(k(a)))) -> h(k(a)) -> a -> b, applying all three
    // nested rules plus the flat one in sequence.
    let input = term("f(g(h(k(a))))", &[]);
    let expected = term("b", &[]);
    assert_eq!(sabre.rewrite(&input), expected);
    assert_eq!(inner.rewrite(&input), expected);
}

/// Rules where the outer symbol of one nested rule coincides with the outer symbol of a
/// non-nested rule, so that a "discarded" classification (head mismatch) can remove the
/// genuinely root-tracked match goal from a state while a differently-rooted "fresh" goal for
/// the same symbol survives as "unchanged" into the next state.
#[test]
fn test_shared_outer_symbol_with_nested_and_flat_rule() {
    let spec = RewriteSpecification::new(vec![
        create_rewrite_rule("f(g(x), y)", "x", &["x", "y"]).unwrap(),
        create_rewrite_rule("g(z)", "z", &["z"]).unwrap(),
    ]);

    let mut sabre = SabreRewriter::new(&spec);
    let input = term("f(g(a), b)", &[]);
    let expected = term("a", &[]);
    assert_eq!(sabre.rewrite(&input), expected);
}
