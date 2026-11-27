//! Resolution of the variables of a modal formula: every occurrence refers to its own binder.

use merc_formula::FormulaType;
use merc_formula::ModalSpecification;
use merc_syntax::ActFrmKind;
use merc_syntax::DataExprKind;
use merc_syntax::RegFrmKind;
use merc_syntax::StateFrmKind;
use merc_syntax::UntypedStateFrmSpec;

/// Type checks `text` as a boolean modal formula, which resolves its variables.
fn check(text: &str) -> ModalSpecification {
    let spec = UntypedStateFrmSpec::parse(text).expect("the formula should parse");
    ModalSpecification::from_untyped(spec, FormulaType::Bool).expect("the formula should type check")
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_fixed_point_variable_resolves_to_its_own_binder() {
    let text = "mu X(n: Nat = 0) . val(n > 0) || X(n)";
    let spec = check(text);

    let StateFrmKind::FixedPoint { variable, body, .. } = &spec.formula().node else {
        panic!("expected a FixedPoint formula");
    };
    let declared = variable.id.expect("the fixpoint variable was assigned a StateVarId");
    let StateFrmKind::Binary { rhs, .. } = &body.node else {
        panic!("expected a Binary (||) body");
    };
    assert!(matches!(
        &rhs.node,
        StateFrmKind::Resolved(name, _, id) if name.node == "X" && *id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_nested_fixed_point_variables_of_the_same_name_do_not_share_an_id() {
    // The inner, parameter-less `X` refers to the *inner* `nu X`, shadowing the outer `mu X`
    // of the same name — the two binders must never share a StateVarId.
    let text = "mu X(n: Nat = 0) . [true](nu X. X)";
    let spec = check(text);

    let StateFrmKind::FixedPoint {
        variable: outer, body, ..
    } = &spec.formula().node
    else {
        panic!("expected an outer FixedPoint formula");
    };
    let outer_declared = outer.id.expect("the outer fixpoint variable was assigned a StateVarId");
    let StateFrmKind::Modality { expr, .. } = &body.node else {
        panic!("expected a Modality body");
    };
    let StateFrmKind::FixedPoint {
        variable: inner,
        body: inner_body,
        ..
    } = &expr.node
    else {
        panic!("expected a nested FixedPoint formula");
    };
    let inner_declared = inner.id.expect("the inner fixpoint variable was assigned a StateVarId");
    assert_ne!(outer_declared, inner_declared);
    assert!(matches!(
        &inner_body.node,
        StateFrmKind::Resolved(name, _, id) if name.node == "X" && *id == inner_declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_action_formula_quantifier_resolves_to_its_own_binder_and_can_see_an_outer_state_formula_binder() {
    // Exercises `resolve_in_act_frm`'s own scoping specifically: `m` is bound by the action
    // formula's own `exists`, `n` by the *enclosing* state formula's `forall`, reached by
    // crossing through the modality's `RegFrm` -- both must resolve, to two different ids.
    let text = "act a: Nat # Nat; form forall n: Nat . [exists m: Nat . a(m, n)] true;";
    let spec = check(text);

    let StateFrmKind::Quantifier { variables, body, .. } = &spec.formula().node else {
        panic!("expected a Quantifier formula");
    };
    let n_declared = variables[0].var_id.expect("the outer binder was assigned a VarId");

    let StateFrmKind::Modality { formula: reg, .. } = &body.node else {
        panic!("expected a Modality body");
    };
    let RegFrmKind::Action(act_frm) = &reg.node else {
        panic!("expected an Action regular formula");
    };
    let ActFrmKind::Quantifier {
        variables: act_vars,
        body: act_body,
        ..
    } = &act_frm.node
    else {
        panic!("expected a Quantifier action formula");
    };
    let m_declared = act_vars[0]
        .var_id
        .expect("the action formula binder was assigned a VarId");
    assert_ne!(n_declared, m_declared);

    let ActFrmKind::MultAct(multi_action) = &act_body.node else {
        panic!("expected a MultAct body");
    };
    assert!(matches!(
        &multi_action.actions[0].args[0].node,
        DataExprKind::Resolved(name, var_id) if name == "m" && **var_id == m_declared
    ));
    assert!(matches!(
        &multi_action.actions[0].args[1].node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == n_declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_action_formula_quantifier_shadows_an_outer_state_formula_binder_of_the_same_name() {
    // `n` is bound twice: the outer `forall` (`Bool`) and the action formula's own `exists`
    // (`Nat`). Inside the action formula, `n` must resolve to the *inner* binder -- the two
    // must never share a VarId.
    let text = "act a: Nat; form forall n: Bool . [exists n: Nat . a(n)] true;";
    let spec = check(text);

    let StateFrmKind::Quantifier { variables, body, .. } = &spec.formula().node else {
        panic!("expected a Quantifier formula");
    };
    let outer_declared = variables[0].var_id.expect("the outer binder was assigned a VarId");

    let StateFrmKind::Modality { formula: reg, .. } = &body.node else {
        panic!("expected a Modality body");
    };
    let RegFrmKind::Action(act_frm) = &reg.node else {
        panic!("expected an Action regular formula");
    };
    let ActFrmKind::Quantifier {
        variables: act_vars,
        body: act_body,
        ..
    } = &act_frm.node
    else {
        panic!("expected a Quantifier action formula");
    };
    let inner_declared = act_vars[0].var_id.expect("the inner binder was assigned a VarId");
    assert_ne!(outer_declared, inner_declared);

    let ActFrmKind::MultAct(multi_action) = &act_body.node else {
        panic!("expected a MultAct body");
    };
    assert!(matches!(
        &multi_action.actions[0].args[0].node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == inner_declared
    ));
}
