use merc_syntax::DataExprKind;
use merc_syntax::PbesExprKind;
use merc_syntax::UntypedPbes;

use merc_pbes::PbesSystem;
use merc_pbes::resolve_variables;

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_prop_var_inst_argument_resolves_to_equation_parameter() {
    let text = "pbes nu X(n: Nat) = val(n == 0) || X(n); init X(0);";
    let mut pbes = UntypedPbes::parse(text).unwrap();
    resolve_variables::<PbesSystem>(&mut pbes);

    let declared = pbes.equations[0].variable.parameters[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let PbesExprKind::Binary { rhs, .. } = &pbes.equations[0].formula.node else {
        panic!("expected a Binary (||) formula");
    };
    let PbesExprKind::PropVarInst(inst) = &rhs.node else {
        panic!("expected a PropVarInst");
    };
    assert!(matches!(
        &inst.arguments[0].node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_quantifier_bound_variable_resolves_to_its_own_binder() {
    let text = "pbes mu X = forall n: Nat . val(n == n); init X;";
    let mut pbes = UntypedPbes::parse(text).unwrap();
    resolve_variables::<PbesSystem>(&mut pbes);

    let PbesExprKind::Quantifier { variables, body, .. } = &pbes.equations[0].formula.node else {
        panic!("expected a Quantifier formula");
    };
    let declared = variables[0].var_id.expect("the binder was assigned a VarId");
    let PbesExprKind::DataValExpr(data_expr) = &body.node else {
        panic!("expected a DataValExpr body");
    };
    let DataExprKind::Binary { lhs, .. } = &data_expr.node else {
        panic!("expected a Binary (==) expression");
    };
    assert!(matches!(
        &lhs.node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_pbes_init_only_sees_globals_not_any_equations_own_parameters() {
    let text = "glob g: Nat; pbes nu X(n: Nat) = val(n == n); init X(g);";
    let mut pbes = UntypedPbes::parse(text).unwrap();
    resolve_variables::<PbesSystem>(&mut pbes);

    let declared = pbes.global_variables[0]
        .var_id
        .expect("the global was assigned a VarId");
    assert!(matches!(
        &pbes.init.arguments[0].node,
        DataExprKind::Resolved(name, var_id) if name == "g" && **var_id == declared
    ));
}
