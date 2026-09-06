//! Unit tests for `translate::linearise`'s individual transformation rules,
//! per `docs/linearisation-implementation-plan.md`'s Milestone 1 scope.

use merc_linearisation::LinearProcessSpecification;
use merc_linearisation::LinearisationError;
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

/// Type checks and linearises `text`, asserting both succeed.
#[track_caller]
fn linearise_ok(text: &str) -> LinearProcessSpecification {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped).expect("the specification should type check");
    match merc_linearisation::linearise(&spec) {
        Ok(lps) => lps,
        Err(error) => panic!("expected linearisation to succeed:\n{text}\nerror: {error}"),
    }
}

/// Type checks `text`, then linearises it, returning the error for the caller to match on the
/// specific variant.
#[track_caller]
fn linearise_err(text: &str) -> LinearisationError {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped).expect("the specification should type check");
    match merc_linearisation::linearise(&spec) {
        Err(error) => error,
        Ok(_) => panic!("expected linearisation to be rejected:\n{text}"),
    }
}

#[test]
fn test_action_is_a_single_summand() {
    let lps = linearise_ok("act a; init a;");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.deadlock_summands.len(), 0);
    assert_eq!(lps.action_summands[0].multi_action.len(), 1);
}

#[test]
fn test_tau_is_the_empty_multi_action() {
    let lps = linearise_ok("init tau;");
    assert_eq!(lps.action_summands.len(), 1);
    assert!(lps.action_summands[0].multi_action.is_empty());
}

#[test]
fn test_delta_is_a_deadlock_summand() {
    let lps = linearise_ok("init delta;");
    assert_eq!(lps.action_summands.len(), 0);
    assert_eq!(lps.deadlock_summands.len(), 1);
}

#[test]
fn test_choice_is_the_union_of_both_sides() {
    let lps = linearise_ok("act a, b; init a + b;");
    assert_eq!(lps.action_summands.len(), 2);
}

#[test]
fn test_sequence_splices_the_continuation_onto_every_terminating_summand() {
    let lps = linearise_ok("act a, b, c; init (a + b) . c;");
    assert_eq!(lps.action_summands.len(), 2);
    for summand in &lps.action_summands {
        assert_eq!(summand.multi_action.len(), 2, "each branch should have picked up 'c'");
    }
}

#[test]
fn test_sequence_after_delta_is_still_just_delta() {
    // `delta` is a zero for sequencing: nothing after it can ever run.
    let lps = linearise_ok("act a; init delta . a;");
    assert_eq!(lps.action_summands.len(), 0);
    assert_eq!(lps.deadlock_summands.len(), 1);
}

#[test]
fn test_condition_splits_into_a_guarded_then_and_a_negated_else() {
    let lps = linearise_ok("act a, b; init (true) -> a <> b;");
    assert_eq!(lps.action_summands.len(), 2);
}

#[test]
fn test_condition_without_else_deadlocks_on_the_negated_guard() {
    let lps = linearise_ok("act a; init (true) -> a;");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.deadlock_summands.len(), 1);
}

#[test]
fn test_self_recursion_becomes_a_process_parameter() {
    let lps = linearise_ok("act a; proc P(n: Nat) = a . P(n); init P(0);");
    assert_eq!(lps.process_parameters.len(), 1);
    assert_eq!(lps.initial_assignment.len(), 1);
    assert_eq!(lps.action_summands.len(), 1);
}

#[test]
fn test_sum_binds_a_variable_on_the_summand() {
    let lps = linearise_ok("act a: Nat; init sum n: Nat . a(n);");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].summation_variables.len(), 1);
}

#[test]
fn test_hide_removes_the_action_from_the_multi_action() {
    let lps = linearise_ok("act a, b; init hide({a}, a|b);");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action[0].name.node, "b");
}

#[test]
fn test_block_drops_the_whole_summand() {
    let lps = linearise_ok("act a, b; init block({a}, a) + b;");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action[0].name.node, "b");
}

#[test]
fn test_rename_relabels_the_action() {
    let lps = linearise_ok("act a, b; init rename({a -> b}, a);");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action[0].name.node, "b");
}

#[test]
fn test_no_init_is_rejected() {
    let untyped = UntypedProcessSpecification::parse("act a;").unwrap();
    let spec = ProcessSpecification::from_untyped(untyped).unwrap();
    assert!(matches!(
        merc_linearisation::linearise(&spec),
        Err(LinearisationError::MissingInit)
    ));
}

#[test]
fn test_process_call_not_in_tail_position_is_rejected() {
    let error = linearise_err("act a, b; proc P = a . P . b; init P;");
    assert!(
        matches!(error, LinearisationError::NotInTailPosition { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_mutual_recursion_is_rejected() {
    let error = linearise_err("act a, b; proc P = a . Q; proc Q = b . P; init P;");
    assert!(
        matches!(error, LinearisationError::MutualRecursionUnsupported { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_parallel_composition_is_rejected() {
    let error = linearise_err("act a, b; init a || b;");
    assert!(
        matches!(error, LinearisationError::UnsupportedConstruct { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_allow_keeps_only_summands_whose_multi_action_is_allowed() {
    let lps = linearise_ok("act a, b; init allow({a}, a + b);");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action[0].name.node, "a");
}

#[test]
fn test_allow_always_keeps_tau_regardless_of_the_allow_set() {
    let lps = linearise_ok("act a; init allow({a}, tau + a);");
    assert_eq!(
        lps.action_summands.len(),
        2,
        "tau passes through 'allow' unconditionally"
    );
}

#[test]
fn test_allow_matches_by_multi_action_not_individual_action() {
    // `{a}` allows the single action `a`, not the multi-action `a|b`.
    let lps = linearise_ok("act a, b; init allow({a}, a|b);");
    assert_eq!(lps.action_summands.len(), 0);
    assert_eq!(lps.deadlock_summands.len(), 0);
}

#[test]
fn test_comm_that_can_never_fire_is_eliminated_by_the_alphabet_axiom() {
    // `comm`'s own summand-level filtering isn't implemented (`comm` alone
    // is still `UnsupportedConstruct`), but the alphabet-axiom pass proves
    // this particular `comm` can never actually fire against `a` alone (its
    // left-hand side needs both `a` and `b` together) and drops the wrapper
    // before `translate` ever sees it.
    let lps = linearise_ok("act a, b, c; init comm({a|b -> c}, a);");
    assert_eq!(lps.action_summands.len(), 1);
    assert_eq!(lps.action_summands[0].multi_action[0].name.node, "a");
}

#[test]
fn test_comm_that_can_fire_is_still_unsupported() {
    let error = linearise_err("act a, b, c; init comm({a|b -> c}, a|b);");
    assert!(
        matches!(error, LinearisationError::UnsupportedConstruct { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_time_is_rejected() {
    let error = linearise_err("act a; init a @ 1;");
    assert!(
        matches!(error, LinearisationError::UnsupportedConstruct { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_printed_output_round_trips_through_the_parser() {
    let lps = linearise_ok("act a; proc P(n: Nat) = (n > 0) -> a . P(Int2Nat(n - 1)) <> delta; init P(3);");
    let printed = lps.to_string();
    UntypedProcessSpecification::parse(&printed)
        .unwrap_or_else(|error| panic!("printed output should reparse:\n{printed}\nerror: {error}"));
}
