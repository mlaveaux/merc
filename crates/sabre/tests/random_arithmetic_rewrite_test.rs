//! Cross-checks [SabreRewriter] against [NaiveRewriter] and an independent Rust
//! evaluator on randomly generated nested Peano-arithmetic/boolean terms.
//!
//! `crates/sabre/src/set_automaton` has no randomized test coverage at all
//! (only a handful of hand-written cases in `automaton_construction_tests.rs`),
//! despite building match goals through several layers of position-shifting
//! (greatest-common-prefix stripping, fresh sub-goals rooted at branch
//! positions) that keep each announcement's `symbols_seen` counter in sync
//! with `SabreRewriter`'s flat term stack (see `MatchAnnouncement::symbols_seen`
//! and `SabreRewriter::apply_rewrite_rule`'s `prune_point` computation). A
//! wrong `symbols_seen` would prune the term stack to the wrong point and
//! produce a wrong rewritten term, or panic on subtraction overflow in debug
//! builds - exactly the kind of defect this test is built to surface, using
//! rules whose overlapping prefixes (`plus`/`mult` sharing the `zero`/`s(x)`
//! split, `eq`'s four head combinations) exercise the same GCP/partition
//! machinery as the hand-written automaton tests, across many random inputs
//! and depths instead of one fixed shape.

use ahash::AHashSet;

use merc_data::DataExpression;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;
use merc_sabre::test_utility::create_rewrite_rule;

use merc_utilities::random_test;
use rand::RngExt;
use rand::rngs::StdRng;

/// Parses a closed (variable-free) term.
fn term(input: &str) -> DataExpression {
    DataExpression::from_string_untyped(input, &AHashSet::default()).unwrap()
}

/// A standard, terminating and confluent Peano-arithmetic/boolean specification.
fn arithmetic_spec() -> RewriteSpecification {
    RewriteSpecification::new(vec![
        create_rewrite_rule("plus(zero, y)", "y", &["y"]).unwrap(),
        create_rewrite_rule("plus(s(x), y)", "s(plus(x, y))", &["x", "y"]).unwrap(),
        create_rewrite_rule("mult(zero, y)", "zero", &["y"]).unwrap(),
        create_rewrite_rule("mult(s(x), y)", "plus(y, mult(x, y))", &["x", "y"]).unwrap(),
        create_rewrite_rule("eq(zero, zero)", "true", &[]).unwrap(),
        create_rewrite_rule("eq(zero, s(y))", "false", &["y"]).unwrap(),
        create_rewrite_rule("eq(s(x), zero)", "false", &["x"]).unwrap(),
        create_rewrite_rule("eq(s(x), s(y))", "eq(x, y)", &["x", "y"]).unwrap(),
        create_rewrite_rule("and(true, b)", "b", &["b"]).unwrap(),
        create_rewrite_rule("and(false, b)", "false", &["b"]).unwrap(),
        create_rewrite_rule("not(true)", "false", &[]).unwrap(),
        create_rewrite_rule("not(false)", "true", &[]).unwrap(),
    ])
}

/// Renders `n` as a Peano numeral `s(s(...zero...))`.
fn numeral(n: u64) -> String {
    let mut result = "zero".to_string();
    for _ in 0..n {
        result = format!("s({result})");
    }
    result
}

/// Generates a random closed arithmetic term of bounded depth, along with the
/// value it evaluates to (under the intended semantics of `plus`/`mult`).
fn gen_arith(rng: &mut StdRng, depth: usize) -> (String, u64) {
    if depth == 0 || rng.random_bool(0.3) {
        let n = rng.random_range(0..=2);
        (numeral(n), n)
    } else {
        match rng.random_range(0..3) {
            0 => {
                let (s, v) = gen_arith(rng, depth - 1);
                (format!("s({s})"), v + 1)
            }
            1 => {
                let (s1, v1) = gen_arith(rng, depth - 1);
                let (s2, v2) = gen_arith(rng, depth - 1);
                (format!("plus({s1}, {s2})"), v1 + v2)
            }
            _ => {
                let (s1, v1) = gen_arith(rng, depth - 1);
                let (s2, v2) = gen_arith(rng, depth - 1);
                (format!("mult({s1}, {s2})"), v1 * v2)
            }
        }
    }
}

/// Generates a random closed boolean term of bounded depth, along with the
/// value it evaluates to.
fn gen_bool(rng: &mut StdRng, depth: usize) -> (String, bool) {
    if depth == 0 || rng.random_bool(0.3) {
        let v = rng.random_bool(0.5);
        (v.to_string(), v)
    } else {
        match rng.random_range(0..3) {
            0 => {
                let (s1, v1) = gen_arith(rng, depth - 1);
                let (s2, v2) = gen_arith(rng, depth - 1);
                (format!("eq({s1}, {s2})"), v1 == v2)
            }
            1 => {
                let (s1, v1) = gen_bool(rng, depth - 1);
                let (s2, v2) = gen_bool(rng, depth - 1);
                (format!("and({s1}, {s2})"), v1 && v2)
            }
            _ => {
                let (s1, v1) = gen_bool(rng, depth - 1);
                (format!("not({s1})"), !v1)
            }
        }
    }
}

/// `SabreRewriter` must normalize every random arithmetic term to the Peano
/// numeral of its value, agreeing with `NaiveRewriter` and the independent
/// Rust evaluator.
#[test]
#[cfg_attr(miri, ignore)] // Term rewriting is far too slow under miri.
fn test_random_arithmetic_terms_normalize_to_their_evaluated_value() {
    let spec = arithmetic_spec();

    random_test(200, |rng| {
        let (input_str, expected_value) = gen_arith(rng, 3);
        let input = term(&input_str);
        let expected = term(&numeral(expected_value));

        let sabre_result = SabreRewriter::new(&spec).rewrite(&input);
        assert_eq!(
            sabre_result, expected,
            "SabreRewriter normalized `{input_str}` (expected value {expected_value}) to {sabre_result}, \
             expected {expected}"
        );

        let naive_result = NaiveRewriter::new(&spec).rewrite(&input);
        assert_eq!(
            naive_result, expected,
            "NaiveRewriter normalized `{input_str}` (expected value {expected_value}) to {naive_result}, \
             expected {expected}"
        );
    });
}

/// Same postcondition as above, but for boolean terms built from `eq`, `and`
/// and `not` over nested arithmetic subterms - stressing `eq`'s four
/// head-symbol combinations and the automaton states shared between `plus`
/// and `mult`'s common `zero`/`s(x)` split.
#[test]
#[cfg_attr(miri, ignore)] // Term rewriting is far too slow under miri.
fn test_random_boolean_terms_normalize_to_their_evaluated_value() {
    let spec = arithmetic_spec();

    random_test(200, |rng| {
        let (input_str, expected_value) = gen_bool(rng, 3);
        let input = term(&input_str);
        let expected = term(&expected_value.to_string());

        let sabre_result = SabreRewriter::new(&spec).rewrite(&input);
        assert_eq!(
            sabre_result, expected,
            "SabreRewriter normalized `{input_str}` (expected value {expected_value}) to {sabre_result}, \
             expected {expected}"
        );

        let naive_result = NaiveRewriter::new(&spec).rewrite(&input);
        assert_eq!(
            naive_result, expected,
            "NaiveRewriter normalized `{input_str}` (expected value {expected_value}) to {naive_result}, \
             expected {expected}"
        );
    });
}
