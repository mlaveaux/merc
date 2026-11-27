//! A struct constructor that shares its name with a symbol of another sort makes generated
//! struct equations ambiguous: `==` is polymorphic over any single sort, so the generated
//! equation has two self-consistent readings. These specifications are rejected; mCRL2 accepts
//! them through a heuristic that is not sound in general.

use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::InferenceError;
use merc_typecheck::WellTypedError;

/// Asserts that `text` is rejected as ambiguous in the equation printed as `expression`.
#[track_caller]
fn assert_ambiguous(text: &str, expression: &str) {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
        Err(WellTypedError::Inference(InferenceError::AmbiguousExpression { expression: actual, .. })) => {
            assert_eq!(actual, expression)
        }
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("expected {text} to be rejected"),
    }
}

/// The nullary constructor `a` of `A` and the projection `a` of another struct.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_constant_and_unrelated_projection_sharing_a_name() {
    assert_ambiguous(
        "sort A = struct a?is_a; sort APos = struct ca(a: A)?is_ca | cpos(p: Pos)?is_cpos; map f: A -> Bool;",
        "==(a, a) = true",
    );
}

/// The nullary constructor `a` of `A` and the unary constructor `a` of `B`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_constant_and_unrelated_struct_function_sharing_a_name() {
    assert_ambiguous("sort A = struct a?is_a; B = struct a(x: Nat)?is_b;", "==(a, a) = true");
}

/// Two unary constructors `c` that differ only in their codomain.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_two_structs_sharing_same_arity_constructor_name() {
    assert_ambiguous(
        "sort C = struct c(v: Nat)?is_c; D = struct c(w: Nat)?is_d;",
        "==(c(@x0_0), c(@y0_0)) = ==(@x0_0, @y0_0)",
    );
}

/// The same collision written as a user equation is rejected in the same way.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_user_written_equivalent_is_rejected() {
    assert_ambiguous(
        "sort A = struct a?is_a; B = struct a(x: Nat)?is_b; map t: Bool; eqn t = (a == a);",
        "t = ==(a, a)",
    );
}
