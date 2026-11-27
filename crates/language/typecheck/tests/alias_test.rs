//! Sort aliases: cycles, recursion through structured sorts, normalisation of declared sorts, and
//! the non-emptiness of sorts.

use merc_syntax::Sort;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::UntypedDataSpecification;

use merc_typecheck::DataSpecification;
use merc_typecheck::WellTypedError;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_trivial_alias_cycle() {
    match DataSpecification::from_untyped(
        UntypedDataSpecification::parse(
            "sort S = T;
            T = U;
            U = S;",
        )
        .unwrap(),
    ) {
        Err(WellTypedError::AliasCycle { sorts, .. })
            if sorts == vec!["S".to_string(), "T".to_string(), "U".to_string()] => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_alias_self_loop_through_container() {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse("sort S = List(S);").unwrap()) {
        Err(WellTypedError::AliasCycle { sorts, .. }) if sorts == vec!["S".to_string()] => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_alias_cycle_through_function_sort() {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse("sort S = List(S -> Bool);").unwrap()) {
        Err(WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. }) if sort == "S" => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_is_allowed() {
    DataSpecification::from_untyped(
        UntypedDataSpecification::parse("sort Tree = struct leaf | node(Tree, Tree);").unwrap(),
    )
    .expect("recursion through a structured sort is well-defined");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_through_list_is_allowed() {
    DataSpecification::from_untyped(
        UntypedDataSpecification::parse("sort Forest = struct node(List(Forest));").unwrap(),
    )
    .expect("recursion through a List container in a structured sort is allowed");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_through_function_sort() {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse("sort S = struct f(S -> Bool);").unwrap()) {
        Err(WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. }) if sort == "S" => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_through_set() {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse("sort S = struct f(Set(S));").unwrap()) {
        Err(WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. }) if sort == "S" => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_through_function_into_list_is_allowed() {
    DataSpecification::from_untyped(UntypedDataSpecification::parse("sort S = struct f(Bool -> List(S));").unwrap())
        .expect("a List container resets the function-sort observation, as in mCRL2");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_through_function_into_set() {
    match DataSpecification::from_untyped(
        UntypedDataSpecification::parse("sort S = struct f(Bool -> Set(S));").unwrap(),
    ) {
        Err(WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. }) if sort == "S" => {}
        Err(other) => panic!("Unexpected error {:?}", other),
        _ => panic!("Expected from_untyped to fail"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mutually_recursive_structs_are_allowed() {
    DataSpecification::from_untyped(
        UntypedDataSpecification::parse(
            "sort A = struct f(B);
        B = struct g(A) | h;",
        )
        .unwrap(),
    )
    .expect("mutual recursion through structured sorts is well-defined");
}
/// Type checks `text` and returns the normalised sort of the map `name`.
fn map_sort(text: &str, name: &str) -> SortExpression {
    let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
    spec.data_specification()
        .map_declarations
        .iter()
        .find(|map| map.identifier.node == name)
        .unwrap_or_else(|| panic!("map {name} should be declared"))
        .sort
        .clone()
}

/// Aliases of basic and container sorts are expanded, also through a chain of aliases.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_alias_is_expanded() {
    assert_eq!(
        map_sort("sort D = Nat; map f: D;", "f").node,
        SortExpressionKind::Simple(Sort::Nat)
    );
    assert_eq!(
        map_sort("sort D = Nat; E = D; map f: E;", "f").node,
        SortExpressionKind::Simple(Sort::Nat)
    );
    assert_eq!(map_sort("sort D = Nat; map f: List(D);", "f").to_string(), "List(Nat)");
}

/// A structured sort is its own representative, so `f: D` stays `D` rather than being replaced by
/// the struct; an alias chain onto a struct ends at the same representative.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_structured_alias_keeps_its_name() {
    let sort = map_sort("sort D = struct a | b; map f: D;", "f");
    assert!(
        matches!(&sort.node, SortExpressionKind::Resolved(name, _) if name == "D"),
        "{sort:?}"
    );

    let text = "sort A = B; B = struct c; map f: A; g: B;";
    let a = map_sort(text, "f");
    assert_eq!(a, map_sort(text, "g"));
    assert!(
        matches!(&a.node, SortExpressionKind::Resolved(name, _) if name == "B"),
        "{a:?}"
    );
}

/// `D` recurses into itself through an inline `struct`; normalisation keeps the back-reference
/// named rather than unfolding it forever.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_alias_through_inline_struct_terminates() {
    assert_eq!(
        map_sort("sort D = List(struct f(D)); map g: D;", "g").to_string(),
        "List(@struct0)"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sort_with_only_recursive_constructors_is_empty() {
    let text = "sort D;\ncons f: D -> D;";
    match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
        Err(WellTypedError::EmptySort { sort, span }) => {
            assert_eq!(sort, "D");
            assert_eq!(span.start, text.find('D').unwrap());
        }
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("expected {text} to be rejected"),
    }
}

/// A constant constructor makes its sort non-empty, and an abstract argument sort is assumed to
/// be non-empty.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_non_empty_sorts() {
    for text in [
        "sort S;\ncons c: S;",
        "sort D;\n     E;\ncons c: D -> E;",
        "sort D;\nmap f: D -> D;",
    ] {
        DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap())
            .unwrap_or_else(|err| panic!("expected {text} to typecheck, got {err}"));
    }
}
