//! Whole-PBES-specification type-checking tests: global variables, propositional-variable
//! equations, and `init`, on top of the data-specification checking `data_specification_test.rs`
//! already covers.

use merc_syntax::UntypedPbes;
use merc_typecheck::PbesError;
use merc_typecheck::PbesSpecification;

/// Type checks `text`, asserting it is accepted.
#[track_caller]
fn check_ok(text: &str) {
    let spec = UntypedPbes::parse(text).expect("the specification should parse");
    if let Err(error) = PbesSpecification::from_untyped(spec) {
        panic!("expected the specification to type check:\n{text}\nerror: {error}");
    }
}

/// Type checks `text`, returning the error for the caller to match on the specific variant.
#[track_caller]
fn check_err(text: &str) -> PbesError {
    let spec = UntypedPbes::parse(text).expect("the specification should parse");
    match PbesSpecification::from_untyped(spec) {
        Err(error) => error,
        Ok(_) => panic!("expected the specification to be rejected:\n{text}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_true_and_false_are_accepted() {
    check_ok("pbes mu X = true; init X;");
    check_ok("pbes mu X = false; init X;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_data_val_expr_against_bool_is_accepted() {
    check_ok("pbes mu X = val(1 == 1); init X;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_multiple_equations_and_cross_reference_are_accepted() {
    check_ok("pbes mu X = Y; nu Y = X; init Y;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_self_recursive_prop_var_inst_is_accepted() {
    // Mirrors examples/pbes/a.text.pbes's `nu Y(...) = ... || Y(...) ...` shape.
    check_ok("pbes nu X(n: Nat) = val(n == 0) || X(n); init X(0);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_argument_upcasts_like_any_other_expression() {
    check_ok("pbes mu X(n: Nat) = val(n == n); init X(1);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_undeclared_propositional_variable_is_rejected() {
    let error = check_err("pbes mu X = true; init Y;");
    assert!(
        matches!(error, PbesError::UndeclaredPropositionalVariable { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_undeclared_propositional_variable_lists_declared_names_as_candidates() {
    let error = check_err("pbes mu X = true; init Y;");
    let PbesError::UndeclaredPropositionalVariable { candidates, .. } = &error else {
        panic!("expected PbesError::UndeclaredPropositionalVariable, got {error:?}");
    };
    assert_eq!(candidates, &["X".to_string()]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_undeclared_propositional_variable_inside_a_formula_is_rejected() {
    let error = check_err("pbes mu X = Y; init X;");
    assert!(
        matches!(error, PbesError::UndeclaredPropositionalVariable { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_arity_mismatch_is_rejected() {
    let error = check_err("pbes mu X(n: Nat) = true; init X(1, 2);");
    assert!(matches!(error, PbesError::ArityMismatch { .. }), "got {error:?}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_argument_sort_mismatch_is_rejected() {
    let error = check_err("pbes mu X(n: Nat) = true; init X(true);");
    assert!(matches!(error, PbesError::Inference(_)), "got {error:?}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_is_in_scope_for_its_body() {
    check_ok("pbes mu X = forall n: Nat . val(n == n); init X;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_is_out_of_scope_outside_it() {
    let error = check_err("pbes mu X = (forall n: Nat . val(n == n)) && val(n == n); init X;");
    assert!(matches!(error, PbesError::Inference(_)), "got {error:?}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_parameter_is_in_scope_for_its_own_formula() {
    check_ok("pbes mu X(n: Nat) = val(n == n); init X(1);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_global_variable_is_in_scope_in_a_formula_and_init() {
    check_ok("glob n: Nat; pbes mu X = val(n == n); init X;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_parameter_shadows_a_global_of_the_same_name() {
    // `X`'s own `n: Bool` parameter shadows the global `n: Nat`.
    check_ok("glob n: Nat; pbes mu X(n: Bool) = val(n); init X(true);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_equation_parameter_is_rejected() {
    let error = check_err("pbes mu X(n: Nat, n: Bool) = true; init X(1, true);");
    assert!(
        matches!(error, PbesError::DuplicateEquationParameter { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_global_variable_is_rejected() {
    let error = check_err("glob n, n: Nat; pbes mu X = true; init X;");
    assert!(
        matches!(error, PbesError::DuplicateGlobalVariable { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_propositional_variable_is_rejected() {
    let error = check_err("pbes mu X = true; mu X = false; init X;");
    assert!(
        matches!(error, PbesError::DuplicatePropositionalVariable { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_anonymous_struct_in_global_variable_is_rejected() {
    let error = check_err("glob g: struct x | y; pbes mu X = true; init X;");
    assert!(
        matches!(error, PbesError::AnonymousStructInDeclaration { .. }),
        "got {error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_global_variable_is_in_scope_as_a_prop_var_inst_argument() {
    // Distinct from test_global_variable_is_in_scope_in_a_formula_and_init:
    // here the global is passed as a `PropVarInst` argument (both in the
    // equation's formula and in `init`), not just compared against itself in
    // a `val(...)`.
    check_ok("glob dc: Bool; pbes nu X(b: Bool) = val(b) && X(dc); init X(dc);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_anonymous_struct_in_equation_parameter_is_rejected() {
    let error = check_err("pbes mu X(g: struct x | y) = true; init X(x);");
    assert!(
        matches!(error, PbesError::AnonymousStructInDeclaration { .. }),
        "got {error:?}"
    );
}
