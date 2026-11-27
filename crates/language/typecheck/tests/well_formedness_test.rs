//! Declarations that are rejected before inference: constructors for basic or function sorts,
//! constructor/mapping conflicts, duplicate variables and misplaced product sorts.

use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::WellTypedError;

fn typed(text: &str) -> DataSpecification {
    DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap())
        .unwrap_or_else(|err| panic!("expected {text} to typecheck, got {err}"))
}

fn typecheck_err(text: &str) -> WellTypedError {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
        Err(err) => err,
        Ok(_) => panic!("expected {text} to be rejected"),
    }
}

/// Asserts that `text` is rejected because constructor `constructor` targets the basic sort
/// written as `sort`.
#[track_caller]
fn assert_constructor_for_basic_sort(text: &str, constructor: &str, sort: &str) {
    match typecheck_err(text) {
        WellTypedError::ConstructorForBasicSort {
            constructor: actual,
            sort: actual_sort,
            ..
        } => assert_eq!((actual.as_str(), actual_sort.as_str()), (constructor, sort)),
        other => panic!("unexpected error {other:?}"),
    }
}

/// Asserts that `text` is rejected because constructor `constructor` targets the function sort
/// written as `sort`.
#[track_caller]
fn assert_constructor_for_function_sort(text: &str, constructor: &str, sort: &str) {
    match typecheck_err(text) {
        WellTypedError::ConstructorForFunctionSort {
            constructor: actual,
            sort: actual_sort,
            ..
        } => assert_eq!((actual.as_str(), actual_sort.as_str()), (constructor, sort)),
        other => panic!("unexpected error {other:?}"),
    }
}

/// The error names the target sort as written, not its expansion.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_for_basic_sort_is_rejected() {
    assert_constructor_for_basic_sort("sort D;\ncons f: D -> Nat;", "f", "Nat");
    assert_constructor_for_basic_sort("sort D = Nat; cons c: D;", "c", "D");
    // `c: A` with `A = Nat -> Bool` is a constructor for `Bool`; `A` is the closest the user came
    // to writing the target.
    assert_constructor_for_basic_sort("sort A = Nat -> Bool; cons c: A;", "c", "A");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_for_function_sort_is_rejected() {
    assert_constructor_for_function_sort("cons c: Bool -> (Nat -> Bool);", "c", "(Nat -> Bool)");
    assert_constructor_for_function_sort("sort A = Nat -> Bool; cons c: Bool -> A;", "c", "A");
}

/// An aliased function sort that ranges over a declared sort makes a valid unary constructor.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_whose_function_alias_targets_a_declared_sort() {
    let spec = typed("sort D; A = Nat -> D; cons c: A; d: D; map g: D -> Bool; eqn g(c(0)) = true;");
    assert!(
        spec.to_typed_string()
            .to_string()
            .contains("g(c(0: Nat): (Nat -> D)): (D -> Bool) = true: Bool;")
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_and_mapping_conflict() {
    match typecheck_err("sort D; cons c: D; map c: D;") {
        WellTypedError::ConstructorAndMappingConflict { constructor, map, .. } => {
            assert_eq!((constructor.as_str(), map.as_str()), ("c", "c"));
        }
        other => panic!("unexpected error {other:?}"),
    }
    // `A` and `(Nat -> Bool)` denote the same sort, so the two `c` are the same symbol even
    // though their written sorts differ.
    match typecheck_err("sort A = Nat -> Bool; sort D; cons c: A -> D; d: D; map c: (Nat -> Bool) -> D;") {
        WellTypedError::ConstructorAndMappingConflict { constructor, map, .. } => {
            assert_eq!((constructor.as_str(), map.as_str()), ("c", "c"));
        }
        other => panic!("unexpected error {other:?}"),
    }
}

/// A constructor and a mapping that share a name but not a sort are distinct overloads.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_and_mapping_overload_is_allowed() {
    let spec = typed("sort D; cons c: Bool -> D; d: D; map c: Nat -> D; g: D -> Bool; eqn g(c(true)) = g(c(0));");
    assert!(
        spec.to_typed_string()
            .to_string()
            .contains("g(c(true: Bool): (Bool -> D)): (D -> Bool) = g(c(0: Nat): (Nat -> D)): (D -> Bool);")
    );
}

/// Mappings with the same name are overloads, resolved by use; a repeated declaration is one
/// symbol, so its use is not ambiguous.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mapping_overloads() {
    let spec = typed("map f: Nat; f: Bool -> Bool; b: Bool; eqn b = f(f == 0);");
    assert!(
        spec.to_typed_string()
            .to_string()
            .contains("b: Bool = f(==(f: Nat, 0: Nat): (Nat # Nat -> Bool)): (Bool -> Bool);")
    );
    let spec = typed("map f: Nat; f: Nat; eqn f = 1;");
    assert!(spec.to_typed_string().to_string().contains("f: Nat = 1: Pos;"));
}

/// mCRL2 rejects both orders ("The variable n occurs multiple times"); resolving by name
/// would otherwise let the later declaration win.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_equation_variable_is_rejected() {
    for text in [
        "map f: Nat -> Bool; var n: Bool; n: Nat; eqn f(n) = true;",
        "map f: Nat -> Bool; var n: Nat; n: Bool; eqn f(n) = true;",
    ] {
        match typecheck_err(text) {
            WellTypedError::DuplicateEquationVariable { variable, .. } => assert_eq!(variable, "n"),
            other => panic!("unexpected error {other:?}"),
        }
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_product_sort_outside_function_domain_is_rejected() {
    for (text, product) in [
        ("map f: Pos -> (Pos # Pos);", "Pos # Pos"),
        ("map f: List(Pos # Pos);", "Pos # Pos"),
        ("sort D = Bool # Bool;", "Bool # Bool"),
        ("map f: Nat; var x: Nat # Nat; eqn f = 0;", "Nat # Nat"),
        ("map f: ((Pos # Pos) -> Bool) -> (Nat # Nat);", "Nat # Nat"),
    ] {
        match typecheck_err(text) {
            WellTypedError::ProductSortOutsideFunctionDomain { sort, span } => {
                assert_eq!(sort, format!("({product})"), "{text}");
                assert_eq!(span.start, text.rfind(product).unwrap(), "{text}");
            }
            other => panic!("unexpected error {other:?} for {text}"),
        }
    }
}

/// Parenthesised products inside a domain are still domain separators, also in a nested
/// higher-order function sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_product_sort_in_function_domain_is_allowed() {
    typed("map f: (Pos # Pos) # Pos -> Bool;");
    typed("map f: ((Pos # Pos) -> Bool) -> Bool;");
}
