//! Resolution of the data variables of a PRES: every occurrence refers to its own binder.

use merc_pbes::resolve_variables;
use merc_pres::PresSystem;
use merc_syntax::DataExprKind;
use merc_syntax::PresExprKind;
use merc_syntax::UntypedPres;

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_pres_prop_var_inst_argument_resolves_to_equation_parameter() {
    let text = "pres mu X(n: Nat) = val(n) || X(n); init X(0);";
    let mut pres = UntypedPres::parse(text).unwrap();
    resolve_variables::<PresSystem>(&mut pres);

    let declared = pres.equations[0].variable.parameters[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let PresExprKind::Binary { rhs, .. } = &pres.equations[0].formula.node else {
        panic!("expected a Binary (||) formula");
    };
    let PresExprKind::PropVarInst(inst) = &rhs.node else {
        panic!("expected a PropVarInst");
    };
    assert!(matches!(
        &inst.arguments[0].node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_pres_bound_variable_resolves_to_its_own_binder() {
    let text = "pres mu X = sum n: Nat . val(n); init X;";
    let mut pres = UntypedPres::parse(text).unwrap();
    resolve_variables::<PresSystem>(&mut pres);

    let PresExprKind::Bound { variables, expr, .. } = &pres.equations[0].formula.node else {
        panic!("expected a Bound formula");
    };
    let declared = variables[0].var_id.expect("the binder was assigned a VarId");
    let PresExprKind::DataValExpr(data_expr) = &expr.node else {
        panic!("expected a DataValExpr body");
    };
    assert!(matches!(
        &data_expr.node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_pres_constant_multiply_resolves_its_constant() {
    let text = "pres mu X(n: Nat) = val(n) * X; init X;";
    let mut pres = UntypedPres::parse(text).unwrap();
    resolve_variables::<PresSystem>(&mut pres);

    let declared = pres.equations[0].variable.parameters[0]
        .var_id
        .expect("the parameter was assigned a VarId");
    let PresExprKind::LeftConstantMultiply { constant, .. } = &pres.equations[0].formula.node else {
        panic!("expected a LeftConstantMultiply formula");
    };
    assert!(matches!(
        &constant.node,
        DataExprKind::Resolved(name, var_id) if name == "n" && **var_id == declared
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_pres_init_only_sees_globals_not_any_equations_own_parameters() {
    let text = "glob g: Nat; pres nu X(n: Nat) = val(n); init X(g);";
    let mut pres = UntypedPres::parse(text).unwrap();
    resolve_variables::<PresSystem>(&mut pres);

    let declared = pres.global_variables[0]
        .var_id
        .expect("the global was assigned a VarId");
    assert!(matches!(
        &pres.init.arguments[0].node,
        DataExprKind::Resolved(name, var_id) if name == "g" && **var_id == declared
    ));
}
