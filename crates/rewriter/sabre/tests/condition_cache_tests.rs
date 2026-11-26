//! Effectiveness and correctness of `matching::condition_cache`: two
//! *unrelated* rules (different root symbols) each have a condition on the
//! same subterm, called for the same concrete argument from a common
//! caller — a large-enough cache normalises that subterm once instead of
//! twice, and a cache too small to retain it must still produce the correct
//! result, just without the speedup. The contract of [ConditionCache] itself
//! (hits, disabling and least recently used eviction) is tested directly.

use ahash::AHashSet;

use merc_aterm::ATerm;
use merc_data::DataExpression;
use merc_data::to_untyped_data_expression;
use merc_sabre::Condition;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_sabre::SabreRewriter;
use merc_sabre::matching::condition_cache::ConditionCache;

/// Parses `input` into an untyped data expression, treating `variables` as variables.
fn term(input: &str, variables: &[&str]) -> DataExpression {
    let vars: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    to_untyped_data_expression(ATerm::from_string(input).unwrap(), Some(&vars))
}

/// Builds `s(s(...d0...))` with `n` applications of `s`.
fn nat_literal(n: u32) -> String {
    let mut result = String::from("d0");
    for _ in 0..n {
        result = format!("s({result})");
    }
    result
}

const N: u32 = 100;

/// `slow(N)` takes `n` rewrite steps to reach `true`; `p`/`q` are two
/// unrelated rules (different root symbols) whose only condition is on
/// `slow` of their argument; `check` calls both on the same argument.
fn spec() -> RewriteSpecification {
    let rules = vec![
        Rule::new(term("slow(d0)", &[]), term("true", &[])),
        Rule::new(term("slow(s(n))", &["n"]), term("slow(n)", &["n"])),
        Rule::with_condition(
            vec![Condition::new(term("slow(x)", &["x"]), term("true", &["x"]), true)],
            term("p(x)", &["x"]),
            term("ok", &["x"]),
        ),
        Rule::with_condition(
            vec![Condition::new(term("slow(x)", &["x"]), term("true", &["x"]), true)],
            term("q(x)", &["x"]),
            term("ok", &["x"]),
        ),
        Rule::new(term("check(x)", &["x"]), term("and(p(x), q(x))", &["x"])),
        Rule::new(term("and(ok, ok)", &[]), term("ok", &[])),
    ];
    RewriteSpecification::new(rules)
}

#[test]
fn test_condition_cache_reduces_steps_across_unrelated_rules() {
    let spec = spec();
    let input = term(&format!("check({})", nat_literal(N)), &[]);
    let expected = term("ok", &[]);

    let mut uncached = InnermostRewriter::with_condition_cache_capacity(&spec, None);
    let (result, uncached_stats) = uncached.rewrite_with_statistics(&input);
    assert_eq!(result, expected, "InnermostRewriter: uncached result mismatch");

    let mut cached = InnermostRewriter::new(&spec);
    let (result, cached_stats) = cached.rewrite_with_statistics(&input);
    assert_eq!(result, expected, "InnermostRewriter: default-capacity result mismatch");

    assert!(
        cached_stats.rewrite_steps < uncached_stats.rewrite_steps,
        "InnermostRewriter: sharing slow(x) across p and q should take fewer rewrite steps ({} >= {})",
        cached_stats.rewrite_steps,
        uncached_stats.rewrite_steps
    );
    // At least one hit is `slow(nat)` itself, recognised by whichever of p/q
    // is checked second; a normalised condition side's literal `true` right-
    // hand side is also, incidentally, looked up and cached the same way, so
    // this is `>=` rather than an exact count.
    assert!(
        cached_stats.condition_cache_hits >= 1,
        "slow(x) should be recognised as already cached by whichever of p/q runs second, got {} hits",
        cached_stats.condition_cache_hits
    );

    let mut uncached_sabre = SabreRewriter::with_condition_cache_capacity(&spec, None);
    let (result, uncached_stats) = uncached_sabre.rewrite_with_statistics(&input);
    assert_eq!(result, expected, "SabreRewriter: uncached result mismatch");

    let mut cached_sabre = SabreRewriter::new(&spec);
    let (result, cached_stats) = cached_sabre.rewrite_with_statistics(&input);
    assert_eq!(result, expected, "SabreRewriter: default-capacity result mismatch");

    assert!(
        cached_stats.rewrite_steps < uncached_stats.rewrite_steps,
        "SabreRewriter: sharing slow(x) across p and q should take fewer rewrite steps ({} >= {})",
        cached_stats.rewrite_steps,
        uncached_stats.rewrite_steps
    );
}

#[test]
fn test_condition_cache_correct_under_heavy_eviction() {
    // A cache too small to hold even one of the (many) distinct subterms
    // `slow` recurses through must still produce the right answer: eviction
    // can only cost the speedup, never correctness.
    let spec = spec();
    let input = term(&format!("check({})", nat_literal(N)), &[]);
    let expected = term("ok", &[]);

    let mut inner = InnermostRewriter::with_condition_cache_capacity(&spec, Some(1));
    let (result, _) = inner.rewrite_with_statistics(&input);
    assert_eq!(
        result, expected,
        "a 1-entry cache must still be correct, just ineffective"
    );

    let mut sabre = SabreRewriter::with_condition_cache_capacity(&spec, Some(1));
    let (result, _) = sabre.rewrite_with_statistics(&input);
    assert_eq!(
        result, expected,
        "a 1-entry cache must still be correct, just ineffective"
    );
}

#[test]
fn test_condition_cache_miss_then_hit() {
    let mut cache = ConditionCache::new(Some(0));
    let key = term("even(s(s(d0)))", &[]);
    let value = term("true", &[]);

    assert_eq!(cache.get(&key), None);
    cache.insert(key.clone(), value.clone());
    assert_eq!(cache.get(&key), Some(value));
    assert_eq!(cache.get(&term("even(s(d0))", &[])), None);
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_disabled_condition_cache_stores_nothing() {
    let mut cache = ConditionCache::new(None);
    let key = term("even(s(s(d0)))", &[]);
    cache.insert(key.clone(), term("true", &[]));

    assert_eq!(cache.get(&key), None);
    assert!(cache.is_empty());
}

#[test]
fn test_condition_cache_evicts_the_least_recently_used_entry() {
    let mut cache = ConditionCache::new(Some(2));
    let (a, b, c) = (term("a", &[]), term("b", &[]), term("c", &[]));
    let (value_a, value_b, value_c) = (term("va", &[]), term("vb", &[]), term("vc", &[]));

    cache.insert(a.clone(), value_a.clone());
    cache.insert(b.clone(), value_b);

    // Looking up `a` makes `b` the least recently used entry, so inserting `c` evicts `b`.
    assert_eq!(cache.get(&a), Some(value_a.clone()));
    cache.insert(c.clone(), value_c.clone());

    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get(&b), None);
    assert_eq!(cache.get(&a), Some(value_a));
    assert_eq!(cache.get(&c), Some(value_c));
}
