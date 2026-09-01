#![forbid(unsafe_code)]

use std::fmt;

use ahash::HashMap;
use ahash::HashMapExt;
use itertools::Itertools;
use log::debug;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;

use crate::Rule;
use crate::utilities::DataPosition;
use crate::utilities::TermStack;
use crate::utilities::TermStackBuilder;
use crate::utilities::create_var_map;

/// A subterm that occurs more than once across the combined left- and
/// right-hand sides of a rule's conditions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CachedTerm {
    /// The pattern this entry was derived from, kept for [fmt::Display].
    pattern: DataExpression,
    /// How to construct the subterm from a match. May itself reference
    /// earlier (necessarily smaller) entries of the same cache through
    /// [TermStack::from_term_with_cache].
    term_stack: TermStack,
}

/// One condition of a [ConditionCache], with both sides built so that any
/// occurrence of a [CachedTerm] is filled from the cache instead of being
/// reconstructed.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CachedCondition {
    lhs: TermStack,
    rhs: TermStack,
    equality: bool,
}

/// A per-rule cache of subterms shared between two or more of its
/// conditions, built once by [build_condition_cache] and read on every
/// [check_conditions_with_cache] call; the two functions are the only entry
/// points, and this type's fields are private.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConditionCache {
    /// Ordered so a later entry only ever references earlier ones.
    terms: Vec<CachedTerm>,
    conditions: Vec<CachedCondition>,
}

impl fmt::Display for ConditionCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.terms.iter().map(|t| &t.pattern).format(", "))
    }
}

/// Builds a [ConditionCache] for `rule` from its conditions' unsubstituted
/// patterns. Returns `None` when no subterm occurs more than once across
/// them — the common case, since most rules have at most one condition — so
/// callers can fall back to checking each condition independently with no
/// overhead.
///
/// For example, given the conditions `even(n) == true` and `f(even(n)) ==
/// false`, the returned cache normalises `even(n)` once and both conditions
/// reuse that result.
pub fn build_condition_cache(rule: &Rule) -> Option<ConditionCache> {
    if rule.conditions.is_empty() {
        return None;
    }

    let var_map = create_var_map(&rule.lhs);

    // Pass 1: count how often every non-trivial subterm occurs across all
    // condition sides combined.
    let mut counts: HashMap<DataExpression, usize> = HashMap::new();
    for c in &rule.conditions {
        count_subterms(c.lhs.copy(), &mut counts);
        count_subterms(c.rhs.copy(), &mut counts);
    }

    // Pass 2: collect the subterms that recur, bottom-up so a subterm
    // nested inside a larger one is always assigned an index before it.
    let mut cache_index: HashMap<DataExpression, usize> = HashMap::new();
    let mut terms: Vec<CachedTerm> = Vec::new();
    for c in &rule.conditions {
        collect_cached_terms(c.lhs.copy(), &counts, &var_map, &mut cache_index, &mut terms);
        collect_cached_terms(c.rhs.copy(), &counts, &var_map, &mut cache_index, &mut terms);
    }

    if terms.is_empty() {
        return None;
    }

    // Pass 3: build every condition's own term stacks, referencing a cached
    // entry wherever it occurs, including a condition side that is itself
    // exactly one of them.
    let conditions = rule
        .conditions
        .iter()
        .map(|c| CachedCondition {
            lhs: TermStack::from_term_with_cache(&c.lhs.copy(), &var_map, &cache_index),
            rhs: TermStack::from_term_with_cache(&c.rhs.copy(), &var_map, &cache_index),
            equality: c.equality,
        })
        .collect();

    let cache = ConditionCache { terms, conditions };
    debug!(
        "condition cache for rule {rule}: {} shared subterm(s): {cache}",
        cache.terms.len()
    );
    Some(cache)
}

/// Checks every condition of `cache` against `matched`: first evaluates
/// `cache`'s subterms and normalises each with `normalise` exactly once, in
/// the order they were collected in, then evaluates and (where needed)
/// normalises every condition's two sides, reusing those results wherever a
/// side references one.
///
/// `builder` is reused, cleared and refilled, by every term-stack evaluation
/// this performs; `normalise` receives it too since normalising a term
/// usually requires building further terms of its own.
pub fn check_conditions_with_cache<'a, 'b, T: Term<'a, 'b>>(
    cache: &ConditionCache,
    matched: &'b T,
    builder: &mut TermStackBuilder,
    normalise: &mut impl FnMut(&DataExpression, &mut TermStackBuilder) -> DataExpression,
) -> bool {
    let mut normal_forms: Vec<DataExpression> = Vec::with_capacity(cache.terms.len());
    for entry in &cache.terms {
        let raw = entry.term_stack.evaluate_with_cache(matched, &normal_forms, builder);
        normal_forms.push(normalise(&raw, builder));
    }

    cache.conditions.iter().all(|c| {
        let lhs = evaluate_normal(&c.lhs, matched, &normal_forms, builder, normalise);
        let rhs = evaluate_normal(&c.rhs, matched, &normal_forms, builder, normalise);

        (lhs == rhs) == c.equality
    })
}

/// Evaluates `term_stack` against `matched` and normalises the result,
/// unless `term_stack` is a single hole (a variable or a cached reference),
/// which is already a normal form and is returned as evaluated.
fn evaluate_normal<'a, 'b, T: Term<'a, 'b>>(
    term_stack: &TermStack,
    matched: &'b T,
    cache: &[DataExpression],
    builder: &mut TermStackBuilder,
    normalise: &mut impl FnMut(&DataExpression, &mut TermStackBuilder) -> DataExpression,
) -> DataExpression {
    let term = term_stack.evaluate_with_cache(matched, cache, builder);
    if term_stack.stack_size == 1 && term_stack.variables.len() + term_stack.cached.len() == 1 {
        term
    } else {
        normalise(&term, builder)
    }
}

/// Counts every non-trivial subterm of `term`, including `term` itself.
/// Variables and machine numbers are excluded: reading either is already
/// O(1), so caching them has no benefit.
fn count_subterms(term: DataExpressionRef<'_>, counts: &mut HashMap<DataExpression, usize>) {
    if is_data_variable(&term) || is_data_machine_number(&term) {
        return;
    }

    *counts.entry(term.protect()).or_insert(0) += 1;
    for arg in term.data_arguments() {
        count_subterms(arg, counts);
    }
}

/// Walks `term` bottom-up, assigning a cache index to every subterm counted
/// at least twice by [count_subterms] that has not been assigned one yet.
fn collect_cached_terms(
    term: DataExpressionRef<'_>,
    counts: &HashMap<DataExpression, usize>,
    var_map: &HashMap<DataVariable, DataPosition>,
    cache_index: &mut HashMap<DataExpression, usize>,
    terms: &mut Vec<CachedTerm>,
) {
    if is_data_variable(&term) || is_data_machine_number(&term) {
        return;
    }

    for arg in term.data_arguments() {
        collect_cached_terms(arg, counts, var_map, cache_index, terms);
    }

    let pattern = term.protect();
    if cache_index.contains_key(&pattern) {
        return;
    }

    if counts.get(&pattern).copied().unwrap_or(0) >= 2 {
        // Children that are themselves cached were indexed above, so this
        // term's own construction references them instead of duplicating
        // their construction.
        let term_stack = TermStack::from_term_with_cache(&term, var_map, cache_index);
        let index = terms.len();
        cache_index.insert(pattern.clone(), index);
        terms.push(CachedTerm { pattern, term_stack });
    }
}

#[cfg(test)]
mod tests {
    use ahash::AHashSet;
    use merc_aterm::ATerm;
    use merc_data::DataExpression;
    use merc_data::to_untyped_data_expression;
    use test_log::test;

    use crate::Condition;
    use crate::Rule;
    use crate::utilities::TermStackBuilder;

    use super::build_condition_cache;
    use super::check_conditions_with_cache;

    /// Builds a rule `lhs -> conditions -> lhs = rhs` where `variables`
    /// names the free variables shared between `lhs` and every condition.
    fn create_conditional_rule(lhs: &str, conditions: &[(&str, &str, bool)], rhs: &str, variables: &[&str]) -> Rule {
        let vars: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
        let parse =
            |s: &str| -> DataExpression { to_untyped_data_expression(ATerm::from_string(s).unwrap(), Some(&vars)) };

        let conditions = conditions
            .iter()
            .map(|(cl, cr, equality)| Condition::new(parse(cl), parse(cr), *equality))
            .collect();

        Rule::with_condition(conditions, parse(lhs), parse(rhs))
    }

    #[test]
    fn test_no_cache_without_conditions() {
        let rule = create_conditional_rule("f(x)", &[], "x", &["x"]);
        assert!(build_condition_cache(&rule).is_none());
    }

    #[test]
    fn test_no_cache_single_condition() {
        // A single, non-repeating condition has nothing to cache.
        let rule = create_conditional_rule("f(x)", &[("g(x)", "true", true)], "x", &["x"]);
        assert!(build_condition_cache(&rule).is_none());
    }

    #[test]
    fn test_no_cache_for_a_bare_variable() {
        // x is a bare left-hand side variable: reading it back out of a
        // match is already O(1), so it is never worth caching even though it
        // recurs across every condition side.
        let rule = create_conditional_rule("f(x)", &[("g(x)", "true", true), ("h(x)", "true2", true)], "x", &["x"]);
        assert!(build_condition_cache(&rule).is_none());
    }

    #[test]
    fn test_cache_across_conditions() {
        // not(p(x)) == true, f(g(p(x)), asd(e, f, p(x))) == z: p(x) recurs
        // across both conditions.
        let rule = create_conditional_rule(
            "h(x, e, f)",
            &[("not(p(x))", "true", true), ("f(g(p(x)), asd(e, f, p(x)))", "z", true)],
            "x",
            &["x", "e", "f"],
        );

        let cache = build_condition_cache(&rule).expect("p(x) should be cached");
        assert_eq!(
            cache.terms.len(),
            1,
            "only p(x) recurs; not(p(x)), g(p(x)) etc. each occur once"
        );
        assert_eq!(cache.terms[0].pattern.to_string(), "p(x)");
    }

    #[test]
    fn test_cache_within_one_condition() {
        // f(p(c), p(c)) == true: p(c) occurs twice within the same side.
        let rule = create_conditional_rule("h(c)", &[("f(p(c), p(c))", "true", true)], "c", &["c"]);

        let cache = build_condition_cache(&rule).expect("p(c) should be cached");
        assert_eq!(cache.terms.len(), 1);
        assert_eq!(cache.terms[0].pattern.to_string(), "p(c)");
    }

    #[test]
    fn test_nested_cache_orders_dependencies_first() {
        // g(p(c)) recurs, and p(c) itself also recurs nested inside it: p(c)
        // must be assigned an index before g(p(c)) is.
        let rule = create_conditional_rule(
            "h(c)",
            &[("g(p(c))", "true", true), ("f(g(p(c)), p(c))", "z", true)],
            "c",
            &["c"],
        );

        let cache = build_condition_cache(&rule).expect("g(p(c)) and p(c) should both be cached");
        assert_eq!(cache.terms.len(), 2);
        assert_eq!(
            cache.terms[0].pattern.to_string(),
            "p(c)",
            "p(c) must be indexed before g(p(c))"
        );
        assert_eq!(cache.terms[1].pattern.to_string(), "g(p(c))");
    }

    #[test]
    fn test_evaluation_matches_direct_normalisation() {
        // even(k) shared between two conditions, checked through the cache
        // against a concrete matched term.
        let rule = create_conditional_rule(
            "h(k)",
            &[("not(even(k))", "false", true), ("f(even(k))", "true", true)],
            "k",
            &["k"],
        );
        let cache = build_condition_cache(&rule).expect("even(k) should be cached");

        let matched = DataExpression::from_string("h(a)").unwrap();
        let mut builder = TermStackBuilder::new();

        // A stub normaliser that leaves every term unchanged and just counts
        // how many times it was called, to check the cached subterm is
        // normalised once rather than once per occurrence.
        let mut calls = 0;
        let mut normalise = |t: &DataExpression, _builder: &mut TermStackBuilder| {
            calls += 1;
            t.clone()
        };

        // not(even(a)) != false, so the first condition already fails and
        // the second is never even evaluated (order independence does not
        // mean every condition runs, only that whichever do give a
        // consistent answer).
        assert!(!check_conditions_with_cache(
            &cache,
            &matched,
            &mut builder,
            &mut normalise
        ));

        // One normalisation for the cached `even(k)`, plus one each for
        // `not(even(k))` and the literal `false` side of the first
        // condition; the second condition is never reached.
        assert_eq!(calls, 3);
    }

    #[test]
    fn test_evaluation_reuses_cached_normal_form() {
        // even(k) == true, and f(even(k)) == true2: the first condition's
        // left-hand side *is* the cached subterm, so checking it consumes
        // the precomputed normal form directly; the second condition
        // reconstructs f(even(k)) and must reuse the same normal form for
        // its `even(k)` argument rather than renormalising it.
        let rule = create_conditional_rule(
            "h(k)",
            &[("even(k)", "true", true), ("f(even(k))", "true2", true)],
            "k",
            &["k"],
        );
        let cache = build_condition_cache(&rule).expect("even(k) should be cached");

        let matched = DataExpression::from_string("h(a)").unwrap();
        let mut builder = TermStackBuilder::new();

        // A stub normaliser that resolves any `even(...)` term to `true`,
        // counting how many times it does so, and otherwise leaves the term
        // unchanged.
        let true_term = DataExpression::from_string("true").unwrap();
        let mut even_normalisations = 0;
        let mut normalise = |t: &DataExpression, _builder: &mut TermStackBuilder| {
            if t.to_string().starts_with("even(") {
                even_normalisations += 1;
                return true_term.clone();
            }
            t.clone()
        };

        // even(a) -> true, so the first condition (even(k) == true) holds.
        // f(even(a)) -> f(true) (the stub does not evaluate `f`), which is
        // not `true2`, so the second condition fails and the overall check
        // reports failure — but only after reusing, not recomputing, the
        // normal form of `even(k)`.
        assert!(!check_conditions_with_cache(
            &cache,
            &matched,
            &mut builder,
            &mut normalise
        ));
        assert_eq!(
            even_normalisations, 1,
            "even(k) must be normalised exactly once and reused by the second condition"
        );
    }
}
