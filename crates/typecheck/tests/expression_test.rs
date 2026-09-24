//! Type checking and lowering of a *standalone* data expression.

use merc_syntax::DataExpr;
use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::InferenceError;
use merc_typecheck::NumberEncoding;

/// Type checks `spec_text`, then type checks and lowers `expr_text` against it.
#[track_caller]
fn lower(spec_text: &str, expr_text: &str) -> String {
    lower_with(spec_text, expr_text, NumberEncoding::Binary)
}

#[track_caller]
fn lower_with(spec_text: &str, expr_text: &str, encoding: NumberEncoding) -> String {
    let untyped = UntypedDataSpecification::parse(spec_text).expect("the specification should parse");
    let mut spec = DataSpecification::from_untyped_with(untyped, encoding, &mut SourceMap::new())
        .expect("the specification should type check");
    let expr = DataExpr::parse(expr_text).expect("the expression should parse");

    spec.typecheck_expression(&expr)
        .unwrap_or_else(|err| panic!("'{expr_text}' should type check: {err}"))
        .to_string()
}

/// Type checks `spec_text`, then returns the error `expr_text` is rejected with.
#[track_caller]
fn lower_err(spec_text: &str, expr_text: &str) -> InferenceError {
    let untyped = UntypedDataSpecification::parse(spec_text).expect("the specification should parse");
    let mut spec = DataSpecification::from_untyped(untyped).expect("the specification should type check");
    let expr = DataExpr::parse(expr_text).expect("the expression should parse");

    match spec.typecheck_expression(&expr) {
        Err(err) => err,
        Ok(term) => panic!("expected '{expr_text}' to be rejected, but it lowered to '{term}'"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_user_constant_lowers() {
    assert_eq!(lower("sort D; cons d: D;", "d"), "d");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_user_application_lowers() {
    assert_eq!(lower("sort D; cons d: D; map f: D -> D;", "f(d)"), "f(d)");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_nested_user_application_lowers() {
    assert_eq!(lower("sort D; cons d: D; map f: D -> D;", "f(f(f(d)))"), "f(f(f(d)))");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_constructor_lowers() {
    // The constructors of a structured sort are declared by desugaring, not by
    // the user text, so this exercises resolution against the desugared spec.
    assert_eq!(
        lower("sort D = struct c(n: Nat) | e;", "c(3)"),
        "c(@cNat(@cDub(true, @c1)))"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_projection_lowers() {
    assert_eq!(lower("sort D = struct c(n: Nat) | e;", "n(e)"), "n(e)");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_operator_node_is_lowered_to_an_application() {
    // `1 + 1` is a `Binary` node.
    assert_eq!(lower("map f: Bool;", "1 + 1"), "+(@c1, @c1)");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_argument_coercion_is_inserted() {
    // `g`'s parameter is `Nat` but `1` infers to `Pos`, so lowering inserts the
    // `@cNat` widening.
    assert_eq!(lower("map g: Nat -> Bool;", "g(1)"), "g(@cNat(@c1))");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_boolean_literal_lowers() {
    assert_eq!(lower("map f: Bool;", "true"), "true");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_literal_lowers_to_a_cons_chain() {
    assert_eq!(lower("map f: Bool;", "[1, 2]"), "|>(@c1, |>(@cDub(false, @c1), []))");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_literal_lowers() {
    assert_eq!(lower("map f: Bool;", "{1}"), "@fset_insert(@c1, {})");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_machine_word_encoding_is_used_for_literals() {
    // The expression is lowered with the specification's own encoding, so the
    // term it produces is compatible with the rules lowered alongside it.
    assert_eq!(
        lower_with("map f: Bool;", "1 + 1", NumberEncoding::MachineWord),
        "+(@most_significant_digit(1), @most_significant_digit(1))"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bound_variables_are_in_scope() {
    // A standalone expression declares no equation variables, but a binder
    // still introduces its own — `x` here resolves to the lambda's parameter.
    let term = lower("map f: Bool;", "lambda x: Nat. x == x");
    assert!(term.contains("Lambda"), "expected a lambda binder in: {term}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_lowers() {
    let term = lower("map f: Bool;", "forall x: Nat. x == x");
    assert!(term.contains("Forall"), "expected a forall binder in: {term}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_clause_lowers() {
    let term = lower("map g: Nat -> Bool;", "g(y) whr y = 1 end");
    assert!(term.contains("Whr"), "expected a where clause in: {term}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_free_identifier_is_undeclared() {
    // There is no enclosing `var` block, so a name that is not a declared
    // constructor or mapping cannot be a variable either.
    assert!(
        matches!(lower_err("map f: Bool;", "x"), InferenceError::UndeclaredName { .. }),
        "a free identifier must be reported as undeclared"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ill_sorted_application_is_rejected() {
    let err = lower_err("sort D; cons d: D; map g: Nat -> Bool;", "g(d)");
    assert!(
        matches!(err, InferenceError::NoTyping { .. }),
        "expected no valid sort assignment, got: {err:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_applying_a_non_function_is_rejected() {
    let err = lower_err("sort D; cons d: D;", "d(d)");
    assert!(
        matches!(
            err,
            InferenceError::NotAFunction { .. } | InferenceError::NoTyping { .. }
        ),
        "expected a non-function application error, got: {err:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_error_renders_a_source_snippet() {
    let err = lower_err("map f: Bool;", "x");
    let mut sources = SourceMap::new();
    sources.add_text("<expression>", "x");
    let rendered = err.render(&sources);
    assert!(rendered.contains("-->"), "expected a caret snippet in: {rendered}");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_lowering_the_specification_still_works_afterwards() {
    // Inference interns sorts into the shared context, so type checking an
    // expression must leave the specification itself lowerable.
    let untyped = UntypedDataSpecification::parse("map g: Nat -> Bool; eqn g(0) = true;").unwrap();
    let mut spec = DataSpecification::from_untyped(untyped).unwrap();

    let before = spec.lower_data_specification().equations().len();
    let expr = DataExpr::parse("g(1)").unwrap();
    spec.typecheck_expression(&expr).expect("g(1) type checks");
    let after = spec.lower_data_specification().equations().len();

    assert_eq!(before, after, "type checking an expression must not add equations");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_the_same_expression_can_be_checked_twice() {
    let untyped = UntypedDataSpecification::parse("map g: Nat -> Bool;").unwrap();
    let mut spec = DataSpecification::from_untyped(untyped).unwrap();
    let expr = DataExpr::parse("g(1)").unwrap();

    let first = spec.typecheck_expression(&expr).unwrap().to_string();
    let second = spec.typecheck_expression(&expr).unwrap().to_string();
    assert_eq!(first, second);
}
