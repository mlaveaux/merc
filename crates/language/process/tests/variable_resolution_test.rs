//! Resolution of the data variables of a process specification: every occurrence refers to its
//! own binder.

use merc_process::ProcessSpecification;
use merc_syntax::DataExprKind;
use merc_syntax::ProcessExprKind;
use merc_syntax::UntypedProcessSpecification;

/// Type checks `text`, which resolves its variables.
fn check(text: &str) -> ProcessSpecification {
    let spec = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    ProcessSpecification::from_untyped(spec).expect("the specification should type check")
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_action_argument_resolves_to_process_parameter() {
    let text = "act a: Nat; proc P(n: Nat) = a(n); init P(1);";
    let spec = check(text);

    let declared = spec.process_declarations()[0].params[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let ProcessExprKind::Action(_, args) = &spec.process_declarations()[0].body.node else {
        panic!("expected an Action body");
    };
    assert!(matches!(
        &args[0].node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_sum_bound_variable_resolves_to_its_own_binder() {
    let text = "act a: Nat; proc P = sum x: Nat . a(x); init P;";
    let spec = check(text);

    let ProcessExprKind::Sum { variables, operand } = &spec.process_declarations()[0].body.node else {
        panic!("expected a Sum body");
    };
    let declared = variables[0].var_id.expect("the binder was assigned a VarId");
    let ProcessExprKind::Action(_, args) = &operand.node else {
        panic!("expected an Action operand");
    };
    assert!(matches!(
        &args[0].node,
        DataExprKind::Resolved(name, var_id) if name == "x" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_dist_weight_sees_its_own_bound_variable() {
    let text = "act a; proc P = dist x: Pos[1/x] . a; init P;";
    let spec = check(text);

    let ProcessExprKind::Dist {
        variables,
        expr: weight,
        ..
    } = &spec.process_declarations()[0].body.node
    else {
        panic!("expected a Dist body");
    };
    let declared = variables[0].var_id.expect("the binder was assigned a VarId");
    let DataExprKind::Binary { rhs, .. } = &weight.node else {
        panic!("expected a Binary (division) weight, got {:?}", weight.node);
    };
    assert!(matches!(
        &rhs.node,
        DataExprKind::Resolved(name, var_id) if name == "x" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_assignment_value_resolves_but_key_does_not() {
    let text = "proc P(n: Nat) = P(n = n); init P(0);";
    let spec = check(text);

    let declared = spec.process_declarations()[0].params[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let ProcessExprKind::Id(_, assignments) = &spec.process_declarations()[0].body.node else {
        panic!("expected an Id (instantiation) body");
    };
    // The key `n` is a plain `String` field (`AssignmentData::identifier`), never touched by
    // this pass; only the value `n` (the expression) is a `DataExpr` and gets resolved.
    assert_eq!(assignments[0].identifier, "n");
    assert!(matches!(
        &assignments[0].expr.node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_nested_data_binder_inside_process_action_argument_resolves() {
    let text = "act a: Bool; proc P(n: Nat) = a(exists x: Nat . x == n); init P(0);";
    let spec = check(text);

    let n_declared = spec.process_declarations()[0].params[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let ProcessExprKind::Action(_, args) = &spec.process_declarations()[0].body.node else {
        panic!("expected an Action body");
    };
    let DataExprKind::Quantifier { variables, body, .. } = &args[0].node else {
        panic!("expected a Quantifier argument");
    };
    let x_declared = variables[0].var_id.expect("the binder was assigned a VarId");
    let DataExprKind::Binary { lhs, rhs, .. } = &body.node else {
        panic!("expected a Binary (==) body");
    };
    assert!(matches!(
        &lhs.node,
        DataExprKind::Resolved(name, var_id) if name == "x" && **var_id == x_declared
    ));
    assert!(matches!(
        &rhs.node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == n_declared
    ));
    // The process parameter and the nested quantifier binder are distinct binders and must
    // never share an id, even though each is the "first" binder of its own construct.
    assert_ne!(n_declared, x_declared);
}
