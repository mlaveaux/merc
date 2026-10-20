//! Tests of `linearise` on small specifications, one feature at a time.

use merc_data::DataExpression;
use merc_linearisation::LinearisationError;
use merc_linearisation::LinearisationOptions;
use merc_lps::LinearProcessSpecification;
use merc_lps::read_lps;
use merc_lps::write_lps;
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

/// Type checks and linearises `text`, asserting that both succeed and that the result survives a
/// round trip through the binary `.lps` format.
#[track_caller]
fn linearise_ok(text: &str) -> LinearProcessSpecification {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped)
        .and_then(|mut spec| spec.lower())
        .expect("the specification should type check");
    let lps = match merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default())
    {
        Ok(lps) => lps,
        Err(error) => panic!("expected linearisation to succeed:\n{text}\nerror: {error}"),
    };

    let mut bytes = Vec::new();
    write_lps(&lps, &mut bytes).expect("the LPS can be written");
    let read = read_lps(bytes.as_slice()).expect("the written LPS can be read");
    assert_eq!(read, lps, "the LPS changed in a round trip through the .lps format");
    lps
}

#[track_caller]
fn linearise_err(text: &str) -> LinearisationError {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped)
        .and_then(|mut spec| spec.lower())
        .expect("the specification should type check");
    match merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default()) {
        Err(error) => error,
        Ok(lps) => panic!("expected linearisation to be rejected:\n{text}\n{lps}"),
    }
}

fn names(lps: &LinearProcessSpecification) -> Vec<String> {
    let mut result: Vec<String> = lps
        .process()
        .action_summands()
        .iter()
        .map(|summand| summand.multi_action().to_string())
        .collect();
    result.sort();
    result
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_terminating_action_is_followed_by_terminate() {
    let lps = linearise_ok("act a; init a;");
    assert_eq!(names(&lps), ["Terminate", "a"]);
    assert!(
        lps.action_labels()
            .iter()
            .any(|label| label.name().value() == "Terminate")
    );
}

/// `a . delta` performs `a` exactly once, so the LPS needs a control state for it.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_action_followed_by_deadlock_happens_once() {
    let lps = linearise_ok("act a; init a . delta;");
    assert_eq!(names(&lps), ["a"]);
    let summand = &lps.process().action_summands()[0];
    assert_ne!(
        summand.next_state(),
        lps.initial_state(),
        "`a` can repeat forever:\n{lps}"
    );
}

/// `P(n) = a` performs `a` once; afterwards the process has stopped.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_terminating_action_happens_once() {
    let lps = linearise_ok("act a; proc P(n: Nat) = a; init P(0);");
    for summand in lps.process().action_summands() {
        assert_ne!(summand.next_state(), lps.initial_state(), "{lps}");
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_tau_is_the_empty_multi_action() {
    let lps = linearise_ok("init tau . delta;");
    assert_eq!(names(&lps), ["tau"]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_single_recursive_equation_needs_no_control_state() {
    let lps = linearise_ok("act a, b; proc P = a . P + b . P; init P;");
    assert_eq!(names(&lps), ["a", "b"]);
    assert!(lps.process().parameters().is_empty(), "{lps}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_self_recursion_with_data() {
    let lps = linearise_ok("act a: Nat; proc P(n: Nat) = a(n) . P(n + 1); init P(0);");
    assert_eq!(lps.process().parameters().len(), 1);
    let next = &lps.process().action_summands()[0].next_state()[0];
    assert_ne!(
        *next,
        DataExpression::from(lps.process().parameters()[0].clone()),
        "{lps}"
    );
    assert!(merc_data::is_closed(&lps.initial_state()[0]), "{lps}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mutual_recursion() {
    let lps = linearise_ok("act a, b; proc P(x: Bool) = a . Q(!x); Q(y: Bool) = b . P(y); init P(true);");
    assert_eq!(names(&lps), ["a", "b"]);
    // A control parameter and the two data parameters.
    assert_eq!(lps.process().parameters().len(), 3, "{lps}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_call_in_non_tail_position() {
    let lps = linearise_ok("act a, b, c; proc P = a . Q . c . P; Q = b + a . Q; init P;");
    assert_eq!(names(&lps), ["a", "a", "b", "c"]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sum_binds_a_summation_variable() {
    let lps = linearise_ok("act a: Bool; proc P = sum b: Bool . a(b) . P; init P;");
    assert_eq!(lps.process().action_summands()[0].summation_variables().len(), 1);
}

/// A sum variable named like a parameter of another equation must not capture it.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sum_variable_does_not_capture() {
    let lps = linearise_ok(
        "act a: Bool; b; proc P(x: Bool) = (sum x: Bool . a(x)) . Q(x); Q(y: Bool) = b . P(!y); init P(true);",
    );
    for summand in lps.process().action_summands() {
        for sum in summand.summation_variables() {
            assert!(
                lps.process()
                    .parameters()
                    .iter()
                    .all(|parameter| parameter.name() != sum.name()),
                "{lps}"
            );
        }
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_hide_rename_and_block() {
    assert_eq!(
        names(&linearise_ok("act a, b; init hide({a}, a . b . delta);")),
        ["b", "tau"]
    );
    assert_eq!(
        names(&linearise_ok("act a, b; init rename({a -> b}, a . delta);")),
        ["b"]
    );
    assert_eq!(
        names(&linearise_ok("act a, b; init block({a}, a . delta + b . delta);")),
        ["b"]
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_allow_keeps_tau_and_matches_multi_actions() {
    assert_eq!(
        names(&linearise_ok("act a, b; init allow({b}, tau . a . delta);")),
        ["tau"]
    );
    assert_eq!(
        names(&linearise_ok("act a, b; init allow({a|b}, a || b);")),
        ["Terminate", "a|b"]
    );
}

/// Restrictions are pushed through `comm`, `hide` and `rename` by the alphabet of their operand;
/// the multi-actions that survive must be those the operators produce.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_restrictions_through_operators() {
    assert_eq!(
        names(&linearise_ok(
            "act a, b, c; init block({a, b}, comm({a|b -> c}, a || b));"
        )),
        ["Terminate", "c"]
    );
    assert_eq!(
        names(&linearise_ok(
            "act a, b, c; init allow({c}, hide({a}, a|c . delta || b . delta));"
        )),
        ["c"]
    );
    assert_eq!(
        names(&linearise_ok(
            "act a, b, c; init allow({c, c|b}, rename({a -> c}, a . delta || b . delta));"
        )),
        ["b|c", "c"]
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_parallel_composition_interleaves_and_synchronises() {
    let lps = linearise_ok("act a, b; init a || b;");
    assert_eq!(names(&lps), ["Terminate", "a", "a|b", "b"]);
}

/// The communication of `s(x)` and `r(y)` requires `x == y`; sum elimination then removes one of
/// the two summation variables.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_communication_with_data() {
    let lps = linearise_ok(
        "act s, r, c: Bool;
         proc S = sum x: Bool . s(x) . S;
              R = sum y: Bool . r(y) . R;
         init allow({c}, comm({s|r -> c}, S || R));",
    );
    assert_eq!(lps.process().action_summands().len(), 1, "{lps}");
    let summand = &lps.process().action_summands()[0];
    assert_eq!(summand.multi_action().to_string().split('(').next(), Some("c"));
    assert_eq!(summand.summation_variables().len(), 1, "{lps}");
}

/// Processes may be overloaded on their parameter sorts; `init P(1)` must select `P(n: Nat)`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_overloaded_process_selects_by_sort() {
    let lps = linearise_ok("act a, b; proc P(n: Nat) = a . P(n); P(x: Bool) = b . P(x); init P(1);");
    assert_eq!(names(&lps), ["a"]);
    assert_eq!(lps.process().parameters()[0].name(), "n", "{lps}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_no_init_is_rejected() {
    assert!(matches!(linearise_err("act a;"), LinearisationError::MissingInit));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_unguarded_recursion_is_rejected() {
    assert!(matches!(
        linearise_err("act a; proc P = a + P; init P;"),
        LinearisationError::UnguardedRecursion { .. }
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_terminate_is_reserved() {
    assert!(matches!(
        linearise_err("act Terminate; init Terminate;"),
        LinearisationError::ReservedActionName { .. }
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_parallel_inside_sequence_is_rejected() {
    assert!(matches!(
        linearise_err("act a; proc P = a . (P || P); init P;"),
        LinearisationError::NotParallelPcrl { .. }
    ));
}
