//! Regression tests for `MultiAction`'s `PartialEq`/`Hash` implementations
//! (`crates/syntax/src/syntax_tree/specs.rs`), which must treat a multi-action
//! as a *multiset* of actions: order-independent, but multiplicity-sensitive.

use merc_syntax::Action;
use merc_syntax::MultiAction;

/// `{a, a}` (two occurrences of `a`) must not equal `{a, b}`: `PartialEq` for
/// `MultiAction` checks only that every element of `self` occurs *somewhere*
/// in `other` (via `Vec::contains`), without consuming matched elements or
/// otherwise tracking multiplicity. For equal-length multisets that share at
/// least one repeated element, this reports two different multisets as equal.
#[test]
fn multi_action_eq_distinguishes_repeated_from_distinct_actions() {
    let aa = MultiAction::new(vec![
        Action::new("a".to_string(), Vec::new()),
        Action::new("a".to_string(), Vec::new()),
    ]);
    let ab = MultiAction::new(vec![
        Action::new("a".to_string(), Vec::new()),
        Action::new("b".to_string(), Vec::new()),
    ]);

    assert_ne!(
        aa, ab,
        "{{a, a}} and {{a, b}} are different multisets and must not compare equal"
    );
}

/// Same defect, demonstrated the other direction: `{a, a, b}` vs `{a, b, b}`
/// share every distinct element and have equal length, so the same
/// `contains`-without-consuming check reports them equal too.
#[test]
fn multi_action_eq_distinguishes_different_multiplicities() {
    let aab = MultiAction::new(vec![
        Action::new("a".to_string(), Vec::new()),
        Action::new("a".to_string(), Vec::new()),
        Action::new("b".to_string(), Vec::new()),
    ]);
    let abb = MultiAction::new(vec![
        Action::new("a".to_string(), Vec::new()),
        Action::new("b".to_string(), Vec::new()),
        Action::new("b".to_string(), Vec::new()),
    ]);

    assert_ne!(
        aab, abb,
        "{{a, a, b}} and {{a, b, b}} are different multisets and must not compare equal"
    );
}
