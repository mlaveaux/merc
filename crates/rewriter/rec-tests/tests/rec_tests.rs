use ahash::HashMap;
use ahash::HashMapExt;
use merc_utilities::test_logger;
use test_case::test_case;

use merc_aterm::ATerm;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::is_data_application;
use merc_data::to_untyped_data_expression;
use merc_rec_tests::load_rec_from_strings;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;

/// Loads the rewrite specification and the terms to evaluate from the given REC files.
fn load_spec(rec_files: &[&str]) -> (RewriteSpecification, Vec<DataExpression>) {
    let (syntax_spec, syntax_terms) = load_rec_from_strings(rec_files).unwrap();
    let spec = syntax_spec.to_rewrite_spec();
    let terms = syntax_terms
        .iter()
        .map(|t| to_untyped_data_expression(t.clone(), None))
        .collect();
    (spec, terms)
}

/// Parses the expected results, one term per line, asserting that the number of
/// lines matches the number of terms so a drifted snapshot fails clearly.
fn parse_expected(terms: &[DataExpression], expected_result: &str) -> Vec<DataExpression> {
    // The snapshot files end in a trailing newline; ignore the resulting empty line.
    let lines: Vec<&str> = expected_result
        .strip_suffix('\n')
        .unwrap_or(expected_result)
        .split('\n')
        .collect();
    assert_eq!(
        lines.len(),
        terms.len(),
        "the snapshot has {} lines but the specification evaluates {} terms",
        lines.len(),
        terms.len()
    );

    lines
        .iter()
        .map(|line| to_untyped_data_expression(ATerm::from_string(line).unwrap(), None))
        .collect()
}

/// Rewrites every term with `rewriter` and compares against the expected results.
fn check_rewriter(
    rewriter: &mut impl RewriteEngine,
    terms: &[DataExpression],
    expected: &[DataExpression],
    engine: &str,
) {
    for (term, expected_result) in terms.iter().zip(expected) {
        let result = rewriter.rewrite(term);
        assert_eq!(
            &result, expected_result,
            "The {engine} rewrite result doesn't match the expected result"
        );
    }
}

/// Abstracts the arguments of `term` into fresh variables, mapped to their own normal forms.
///
/// Rewriting the result under that substitution must reach the same normal form as rewriting
/// `term` itself, since the arguments are normalised either way. Returns `None` for terms
/// without arguments, which have nothing to abstract.
fn abstract_arguments(
    rewriter: &mut impl RewriteEngine,
    term: &DataExpression,
) -> Option<(DataExpression, HashMap<DataVariable, DataExpression>)> {
    if !is_data_application(term) {
        return None;
    }

    let mut sigma = HashMap::new();
    let mut arguments = vec![];
    for (index, argument) in term.data_arguments().enumerate() {
        // Variables and function symbols are distinct terms, so these names cannot clash with
        // the function symbols of the specification.
        let name = format!("x{index}");
        let variable = DataVariable::new(name.as_str());

        sigma.insert(variable.clone(), rewriter.rewrite(&argument.protect()));
        arguments.push(DataExpression::from(variable));
    }

    let symbol = term.data_function_symbol().protect();
    Some((DataApplication::with_args(&symbol, &arguments).into(), sigma))
}

/// Rewrites every term with `rewriter` under a substitution that supplies its arguments, and
/// compares against the expected results.
fn check_rewriter_with_substitution(
    rewriter: &mut impl RewriteEngine,
    terms: &[DataExpression],
    expected: &[DataExpression],
    engine: &str,
) {
    for (term, expected_result) in terms.iter().zip(expected) {
        let Some((abstracted, sigma)) = abstract_arguments(rewriter, term) else {
            continue;
        };

        let result = rewriter.rewrite_with(&abstracted, &sigma);
        assert_eq!(
            &result, expected_result,
            "The {engine} rewrite result under a substitution doesn't match the expected result"
        );
    }
}

/// Showcases that the set-automaton rewriter rewrites lazily: for the `lazyif`
/// specification it only evaluates the selected branch of an if-then-else, so it
/// performs the same number of rewrite steps that a just-in-time (jitty)
/// strategy would, whereas the innermost rewriter eagerly normalises the
/// discarded branch as well.
#[cfg_attr(miri, ignore)]
#[test]
fn test_sabre_matches_jitty_step_count_on_if_heavy() {
    test_logger();

    let (spec, terms) = load_spec(&[include_str!("lazyif.rec")]);

    let mut inner = InnermostRewriter::new(&spec);
    let mut sabre = SabreRewriter::new(&spec);

    for (i, term) in terms.iter().enumerate() {
        let (inner_result, inner_stats) = inner.rewrite_with_statistics(term);
        let (sabre_result, sabre_stats) = sabre.rewrite_with_statistics(term);

        // Both engines must reach the same normal form.
        assert_eq!(
            inner_result, sabre_result,
            "the innermost and set-automaton rewriters disagree on term {i}"
        );

        // The set-automaton rewriter never rewrites the discarded fib(..)
        // branches, so it never takes more steps than innermost (2 versus 173,
        // 2 versus 173 and 4 versus 274 for the if-then-else terms) and takes
        // the same number of steps when the result is genuinely computed (57
        // for the plain fib term).
        assert!(
            sabre_stats.rewrite_steps <= inner_stats.rewrite_steps,
            "the set-automaton rewriter took {} steps versus {} for innermost on term {i}",
            sabre_stats.rewrite_steps,
            inner_stats.rewrite_steps
        );
    }
}

/// A local function to share the rec_test functionality.
///
/// When `check_steps` holds also asserts that the set-automaton rewriter never
/// applies more rewrite steps than the innermost rewriter, since it only
/// rewrites branches that contribute to the normal form. This does not hold for
/// heavily duplicating specifications, where rewriting a duplicating rule early
/// copies unfinished work that innermost would have normalised once.
///
/// The steps are counted without the condition cache: innermost builds its
/// condition sides from normal forms, so they coincide, and hit the cache, more
/// often than those of the set-automaton rewriter, which would make the
/// comparison measure the cache rather than the strategy.
fn rec_test(rec_files: Vec<&str>, expected_result: &str, check_steps: bool) {
    test_logger();

    let (spec, terms) = load_spec(&rec_files);
    let expected = parse_expected(&terms, expected_result);

    let mut inner = InnermostRewriter::with_condition_cache_capacity(&spec, None);
    let mut sabre = SabreRewriter::with_condition_cache_capacity(&spec, None);

    for (i, (term, expected_result)) in terms.iter().zip(&expected).enumerate() {
        let (inner_result, inner_stats) = inner.rewrite_with_statistics(term);
        assert_eq!(
            &inner_result, expected_result,
            "The inner rewrite result doesn't match the expected result"
        );

        let (sabre_result, sabre_stats) = sabre.rewrite_with_statistics(term);
        assert_eq!(
            &sabre_result, expected_result,
            "The sabre rewrite result doesn't match the expected result"
        );

        assert!(
            !check_steps || sabre_stats.rewrite_steps <= inner_stats.rewrite_steps,
            "the set-automaton rewriter took {} steps versus {} for innermost on term {i}",
            sabre_stats.rewrite_steps,
            inner_stats.rewrite_steps
        );
    }
}

#[cfg_attr(miri, ignore)]
#[test_case(vec![include_str!("../../../../examples/REC/rec/calls.rec")], include_str!("snapshot/result_calls.txt") ; "calls")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check1.rec")], include_str!("snapshot/result_check1.txt") ; "check1")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check2.rec")], include_str!("snapshot/result_check2.txt") ; "check2")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/confluence.rec")], include_str!("snapshot/result_confluence.txt") ; "confluence")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/fibonacci05.rec"), include_str!("../../../../examples/REC/rec/fibonacci.rec")], include_str!("snapshot/result_fibonacci05.txt") ; "fibonacci05")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/hanoi4.rec"), include_str!("../../../../examples/REC/rec/hanoi.rec")], include_str!("snapshot/result_hanoi4.txt") ; "hanoi4")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/logic3.rec")], include_str!("snapshot/result_logic3.txt") ; "logic3")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/merge.rec")], include_str!("snapshot/result_merge.txt") ; "merge")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/mergesort10.rec"), include_str!("../../../../examples/REC/rec/mergesort.rec")], include_str!("snapshot/result_mergesort10.txt") ; "mergesort10")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/quicksort10.rec"), include_str!("../../../../examples/REC/rec/quicksort.rec")], include_str!("snapshot/result_quicksort10.txt") ; "quicksort10")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/revelt.rec")], include_str!("snapshot/result_revelt.txt") ; "revelt")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/searchinconditions.rec")], include_str!("snapshot/result_searchinconditions.txt") ; "searchinconditions")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/sieve20.rec"), include_str!("../../../../examples/REC/rec/sieve.rec")], include_str!("snapshot/result_sieve20.txt") ; "sieve20")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/sieve100.rec"), include_str!("../../../../examples/REC/rec/sieve.rec")], include_str!("snapshot/result_sieve100.txt") ; "sieve100")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/soundnessofparallelengines.rec")], include_str!("snapshot/result_soundnessofparallelengines.txt") ; "soundnessofparallelengines")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/tricky.rec")], include_str!("snapshot/result_tricky.txt") ; "tricky")]
fn test_rec_specification(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, true);
}

// These specifications are heavily duplicating (for instance tak(X, Y, Z) copies
// its arguments into several recursive calls), in which case the innermost
// strategy is the more economical one and the step-count invariant does not
// hold.
#[cfg_attr(miri, ignore)]
#[test_case(vec![include_str!("../../../../examples/REC/rec/tak18.rec"), include_str!("../../../../examples/REC/rec/tak.rec")], include_str!("snapshot/result_tak18.txt") ; "tak18")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/tautologyhard.rec")], include_str!("snapshot/result_tautologyhard.txt") ; "tautologyhard")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/bubblesort10.rec"), include_str!("../../../../examples/REC/rec/bubblesort.rec")], include_str!("snapshot/result_bubblesort10.txt") ; "bubblesort10")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/bubblesort20.rec"), include_str!("../../../../examples/REC/rec/bubblesort.rec")], include_str!("snapshot/result_bubblesort20.txt") ; "bubblesort20")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/missionaries2.rec"), include_str!("../../../../examples/REC/rec/missionaries.rec")], include_str!("snapshot/result_missionaries2.txt") ; "missionaries2")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/missionaries3.rec"), include_str!("../../../../examples/REC/rec/missionaries.rec")], include_str!("snapshot/result_missionaries3.txt") ; "missionaries3")]
fn test_rec_specification_duplicating(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, false);
}

// On these specifications the set-automaton rewriter takes more rewrite steps
// than the innermost rewriter, so only the normal forms are compared. The automaton construction merges a new match goal
// into an existing partition only when their obligation positions overlap,
// which bounds the size of the automaton but gives up part of the laziness that
// keeps the step count within that of innermost elsewhere.
#[cfg_attr(miri, ignore)]
#[test_case(vec![include_str!("../../../../examples/REC/rec/benchexpr10.rec"), include_str!("../../../../examples/REC/rec/asfsdfbenchmark.rec")], include_str!("snapshot/result_benchexpr10.txt") ; "benchexpr10")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/benchsym10.rec"), include_str!("../../../../examples/REC/rec/asfsdfbenchmark.rec")], include_str!("snapshot/result_benchsym10.txt") ; "benchsym10")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/factorial5.rec"), include_str!("../../../../examples/REC/rec/factorial.rec")], include_str!("snapshot/result_factorial5.txt") ; "factorial5")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/garbagecollection.rec")], include_str!("snapshot/result_garbagecollection.txt") ; "garbagecollection")]
fn test_rec_specification_more_steps_than_innermost(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, false);
}

#[cfg_attr(miri, ignore)]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check1.rec")], include_str!("snapshot/result_check1.txt") ; "check1")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check2.rec")], include_str!("snapshot/result_check2.txt") ; "check2")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/logic3.rec")], include_str!("snapshot/result_logic3.txt") ; "logic3")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/searchinconditions.rec")], include_str!("snapshot/result_searchinconditions.txt") ; "searchinconditions")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/tautologyhard.rec")], include_str!("snapshot/result_tautologyhard.txt") ; "tautologyhard")]
fn test_rec_specification_naive(rec_files: Vec<&str>, expected_result: &str) {
    test_logger();

    let (spec, terms) = load_spec(&rec_files);
    let expected = parse_expected(&terms, expected_result);

    check_rewriter(&mut NaiveRewriter::new(&spec), &terms, &expected, "naive");
}

/// Pins the substitution-aware path of every engine to its plain one on specifications with
/// conditional rules, where the substituted subterms also end up in the condition sides. The
/// innermost engine consumes the substitution while decomposing the term, whereas the other two
/// eagerly substitute first.
#[cfg_attr(miri, ignore)]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check1.rec")], include_str!("snapshot/result_check1.txt") ; "check1")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/check2.rec")], include_str!("snapshot/result_check2.txt") ; "check2")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/logic3.rec")], include_str!("snapshot/result_logic3.txt") ; "logic3")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/searchinconditions.rec")], include_str!("snapshot/result_searchinconditions.txt") ; "searchinconditions")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/tautologyhard.rec")], include_str!("snapshot/result_tautologyhard.txt") ; "tautologyhard")]
fn test_rec_specification_substitution(rec_files: Vec<&str>, expected_result: &str) {
    test_logger();

    let (spec, terms) = load_spec(&rec_files);
    let expected = parse_expected(&terms, expected_result);

    check_rewriter_with_substitution(&mut InnermostRewriter::new(&spec), &terms, &expected, "innermost");
    check_rewriter_with_substitution(&mut SabreRewriter::new(&spec), &terms, &expected, "sabre");
    check_rewriter_with_substitution(&mut NaiveRewriter::new(&spec), &terms, &expected, "naive");
}

// These tests are too slow without optimisations.
#[cfg_attr(miri, ignore)]
#[cfg(not(debug_assertions))]
#[test_case(vec![include_str!("../../../../examples/REC/rec/bubblesort100.rec"), include_str!("../../../../examples/REC/rec/bubblesort.rec")], include_str!("snapshot/result_bubblesort100.txt") ; "bubblesort100")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/empty.rec")], include_str!("snapshot/result_empty.txt") ; "empty")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/evalexpr.rec")], include_str!("snapshot/result_evalexpr.txt") ; "evalexpr")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/evaltree.rec")], include_str!("snapshot/result_evaltree.txt") ; "evaltree")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/hanoi8.rec"), include_str!("../../../../examples/REC/rec/hanoi.rec")], include_str!("snapshot/result_hanoi8.txt") ; "hanoi8")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/natlist.rec")], include_str!("snapshot/result_natlist.txt") ; "natlist")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/order.rec")], include_str!("snapshot/result_order.txt") ; "order")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/permutations6.rec"), include_str!("../../../../examples/REC/rec/permutations.rec")], include_str!("snapshot/result_permutations6.txt") ; "permutations6")]
fn test_rec_specification_release(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, true);
}

// The larger instances of the specifications above (benchexpr, benchsym and
// factorial), plus revnat, on which the set-automaton rewriter also takes more
// rewrite steps than the innermost rewriter.
#[cfg_attr(miri, ignore)]
#[cfg(not(debug_assertions))]
#[test_case(vec![include_str!("../../../../examples/REC/rec/benchexpr20.rec"), include_str!("../../../../examples/REC/rec/asfsdfbenchmark.rec")], include_str!("snapshot/result_benchexpr20.txt") ; "benchexpr20")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/benchsym20.rec"), include_str!("../../../../examples/REC/rec/asfsdfbenchmark.rec")], include_str!("snapshot/result_benchsym20.txt") ; "benchsym20")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/factorial6.rec"), include_str!("../../../../examples/REC/rec/factorial.rec")], include_str!("snapshot/result_factorial6.txt") ; "factorial6")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/revnat100.rec"), include_str!("../../../../examples/REC/rec/revnat.rec")], include_str!("snapshot/result_revnat100.txt") ; "revnat100")]
fn test_rec_specification_release_more_steps_than_innermost(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, false);
}

// The mutually recursive conditions of the odd/even rules make the set-automaton
// rewriter re-evaluate a condition per recursive call (2^N - 1 steps versus the
// 21 of innermost), so the step-count invariant does not hold here either.
#[cfg_attr(miri, ignore)]
#[cfg(not(debug_assertions))]
#[test_case(vec![include_str!("../../../../examples/REC/rec/oddeven.rec")], include_str!("snapshot/result_oddeven.txt") ; "oddeven")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/dart.rec")], include_str!("snapshot/result_dart.txt") ; "dart")]
fn test_rec_specification_release_duplicating(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, false);
}

// These specifications recurse too deeply for the default thread stack, and rely
// on the `RUST_MIN_STACK` set in `.config/nextest.toml`.
#[cfg_attr(miri, ignore)]
#[cfg(all(unix, not(debug_assertions)))]
#[ignore]
#[test_case(vec![include_str!("../../../../examples/REC/rec/sieve1000.rec"), include_str!("../../../../examples/REC/rec/sieve.rec")], include_str!("snapshot/result_sieve1000.txt") ; "sieve1000")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/fibonacci18.rec"), include_str!("../../../../examples/REC/rec/fibonacci.rec")], include_str!("snapshot/result_fibonacci18.txt") ; "fibonacci18")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/fibonacci19.rec"), include_str!("../../../../examples/REC/rec/fibonacci.rec")], include_str!("snapshot/result_fibonacci19.txt") ; "fibonacci19")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/fibonacci20.rec"), include_str!("../../../../examples/REC/rec/fibonacci.rec")], include_str!("snapshot/result_fibonacci20.txt") ; "fibonacci20")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/fibonacci21.rec"), include_str!("../../../../examples/REC/rec/fibonacci.rec")], include_str!("snapshot/result_fibonacci21.txt") ; "fibonacci21")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/hanoi12.rec"), include_str!("../../../../examples/REC/rec/hanoi.rec")], include_str!("snapshot/result_hanoi12.txt") ; "hanoi12")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/permutations7.rec"), include_str!("../../../../examples/REC/rec/permutations.rec")], include_str!("snapshot/result_permutations7.txt") ; "permutations7")]
fn test_rec_specification_largestack(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, true);
}

// The deep-recursion specifications on which the set-automaton rewriter also
// takes more rewrite steps than the innermost rewriter.
#[cfg_attr(miri, ignore)]
#[cfg(all(unix, not(debug_assertions)))]
#[ignore]
#[test_case(vec![include_str!("../../../../examples/REC/rec/revnat1000.rec"), include_str!("../../../../examples/REC/rec/revnat.rec")], include_str!("snapshot/result_revnat1000.txt") ; "revnat1000")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/closure.rec")], include_str!("snapshot/result_closure.txt") ; "closure")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/factorial7.rec"), include_str!("../../../../examples/REC/rec/factorial.rec")], include_str!("snapshot/result_factorial7.txt") ; "factorial7")]
#[test_case(vec![include_str!("../../../../examples/REC/rec/factorial8.rec"), include_str!("../../../../examples/REC/rec/factorial.rec")], include_str!("snapshot/result_factorial8.txt") ; "factorial8")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/factorial9.rec"), include_str!("../../../../examples/REC/rec/factorial.rec")], include_str!("snapshot/result_factorial9.txt") ; "factorial9")]
fn test_rec_specification_largestack_more_steps_than_innermost(rec_files: Vec<&str>, expected_result: &str) {
    rec_test(rec_files, expected_result, false);
}

// // These REC tests have META data that is not supported by the current implementation.
// #[test_case(vec![include_str!("../../../../examples/REC/rec/add8.rec")], include_str!("snapshot/result_add8.txt") ; "add8")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/add16.rec")], include_str!("snapshot/result_add16.txt") ; "add16")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/add32.rec")], include_str!("snapshot/result_add32.txt") ; "add32")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/mul8.rec")], include_str!("snapshot/result_mul8.txt") ; "mul8")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/mul16.rec")], include_str!("snapshot/result_mul16.txt") ; "mul16")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/mul32.rec")], include_str!("snapshot/result_mul32.txt") ; "mul32")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/omul8.rec")], include_str!("snapshot/result_omul8.txt") ; "omul8")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/omul32.rec")], include_str!("snapshot/result_omul32.txt") ; "omul32")]
// #[test_case(vec![include_str!("../../../../examples/REC/rec/intnat.rec")], include_str!("snapshot/result_intnat.txt") ; "intnat")]
