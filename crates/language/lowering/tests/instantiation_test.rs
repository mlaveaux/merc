//! The template instantiations that `lower_data_specification` adds: a container,
//! function-update, or comparison instantiation is only generated there.

use merc_data::Mcrl2DataSpecification;
use merc_lowering::lower_data_specification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

fn lowered(text: &str) -> Mcrl2DataSpecification {
    lower_data_specification(&DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap())
}

/// Whether the lowered specification of `text` declares the function-update operators.
fn has_function_update(text: &str) -> bool {
    lowered(text)
        .mappings()
        .iter()
        .any(|map| map.name().value().contains("func_update"))
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_single_argument_function_gets_update_operators() {
    assert!(has_function_update("map f: Nat -> Bool;"));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_multi_argument_function_gets_update_operators() {
    // `Nat # Bool -> Nat` has a product domain; `standard_sort` generalizes
    // the Appendix-B template to it instead of deferring it.
    assert!(has_function_update("map f: Nat # Bool -> Nat;"));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_over_containers_terminates() {
    // Regression: re-scanning generated function-update specs for further
    // function sorts diverged, because `@is_not_an_update: (S -> T) -> Bool`
    // is itself a single-argument function, growing the sort without bound.
    assert!(has_function_update("map f: List(Nat) -> List(Nat);"));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_multi_argument_function_over_containers_terminates() {
    // The same regression as above, but seeded from a multi-argument
    // function so the fixpoint also terminates when it re-scans a
    // generated multi-argument `@func_update`/`@is_not_an_update`/
    // `@if_always_else` specification.
    assert!(has_function_update("map f: List(Nat) # Bool -> List(Nat);"));
}

/// Whether the lowered specification of `text` has an equation for `if` on the sort of the
/// mapping `q` of `sort_text`, with a variable of that sort. Sorts are compared as lowered terms,
/// since a container sort prints as its element sort in some places.
fn has_comparison_equations_for(text: &str, sort_text: &str) -> bool {
    let sort_spec = lowered(sort_text);
    let expected_sort = sort_spec
        .mappings()
        .iter()
        .find(|mapping| mapping.name().value() == "q")
        .expect("sort_text declares q")
        .sort()
        .protect();

    lowered(text).equations().iter().any(|eqn| {
        eqn.variables()
            .into_iter()
            .any(|var| var.sort().protect() == expected_sort)
            && eqn.lhs().to_string().starts_with("if(")
    })
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_container_sort_gets_comparison_equations() {
    // Closes a real gap: `if(b, xs, ys)` for `List(Nat)` used to
    // type-check (the polymorphic scheme accepts any sort) but had no
    // equation to rewrite it with, since `list.mcrl2` defines its own
    // structural `==`/`<` but never a generic `if`.
    assert!(has_comparison_equations_for("map f: List(Nat);", "map q: List(Nat);"));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_sort_gets_comparison_equations() {
    // A `struct` gets its own componentwise `==`/`<`/`<=` from
    // `structured_sort_equations`, but never `if` — that still has to
    // come from the generic scheme.
    assert!(has_comparison_equations_for(
        "sort D = struct c1 | c2; map f: D;",
        "sort D = struct c1 | c2; map q: D;"
    ));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_unused_basic_sort_has_no_comparison_equations() {
    // The comparison-operator instantiation is lazy, like a container's:
    // a basic sort that never occurs in the specification gets no
    // comparison equations, even though its own sort/arithmetic
    // declarations are still unconditionally present.
    assert!(!has_comparison_equations_for("map f: Bool;", "map q: Real;"));
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sort_inferred_only_from_a_literal_gets_comparison_equations() {
    // The inference-driven pass (`extend_system_with_inferred_sorts`)
    // must catch a sort that is never spelled out anywhere in the
    // specification's own text — the comparison-operator counterpart of
    // the enumeration-literal container gap.
    assert!(has_comparison_equations_for(
        "map f: Bool; eqn f = (1 == 1);",
        "map q: Pos;"
    ));
}

/// The container constructors of the lowered specification of `text`, with their sorts.
fn container_constructors(text: &str) -> Vec<String> {
    let lowered = lowered(text);
    let mut symbols: Vec<String> = lowered
        .constructors()
        .iter()
        .chain(lowered.mappings())
        .filter(|symbol| ["@fset_insert", "@fbag_cinsert", "@set", "@bag", "|>"].contains(&symbol.name().value()))
        .map(|symbol| format!("{}: {}", symbol.name().value(), symbol.sort()))
        .collect();
    symbols.sort();
    symbols.dedup();
    symbols
}

/// A container sort pulls in exactly the containers it is defined in terms of, for its element
/// sort, wherever in the specification it occurs.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_containers_are_instantiated_with_their_dependencies() {
    let fset = "@fset_insert: [Nat,FSet(Nat)] -> FSet(Nat)";
    let set = "@set: [[Nat] -> Bool,FSet(Nat)] -> Set(Nat)";
    let fbag = "@fbag_cinsert: [Nat,Nat,FBag(Nat)] -> FBag(Nat)";
    let bag = "@bag: [[Nat] -> Nat,FBag(Nat)] -> Bag(Nat)";

    assert!(container_constructors("map f: Bool;").is_empty());
    assert_eq!(container_constructors("map f: Set(Nat);"), [fset, set]);
    assert_eq!(container_constructors("map f: Bag(Nat);"), [bag, fbag, fset, set]);
    assert_eq!(
        container_constructors("map f: List(Set(Nat));"),
        [fset, set, "|>: [Set(Nat),List(Set(Nat))] -> List(Set(Nat))"]
    );
    // A template instantiated with a user sort.
    assert_eq!(
        container_constructors("sort D = struct s; map f: List(D);"),
        ["|>: [D,List(D)] -> List(D)"]
    );
    // A sort that only occurs on a quantifier.
    assert_eq!(
        container_constructors("map b: Bool; eqn b = forall l: List(Nat). l == [];"),
        ["|>: [Nat,List(Nat)] -> List(Nat)"]
    );
    // A comprehension may denote a set or a bag of its element sort.
    assert_eq!(
        container_constructors("map b: Bool; eqn b = 1 in { n: Pos | n < 3 };"),
        [
            "@bag: [[Pos] -> Nat,FBag(Pos)] -> Bag(Pos)",
            "@fbag_cinsert: [Pos,Nat,FBag(Pos)] -> FBag(Pos)",
            "@fset_insert: [Pos,FSet(Pos)] -> FSet(Pos)",
            "@set: [[Pos] -> Bool,FSet(Pos)] -> Set(Pos)",
        ]
    );
}

/// The template's binder sorts are instantiated like every other sort, so no equation refers to
/// the template variable `S`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_template_binder_sorts_are_instantiated() {
    let equations: Vec<String> = lowered("map f: Bag(Nat);")
        .equations()
        .iter()
        .map(|equation| format!("{} = {}", equation.lhs(), equation.rhs()))
        .collect();
    assert!(
        equations.iter().all(|equation| !equation.contains("SortId(S)")),
        "{equations:#?}"
    );

    let set_equality = equations
        .iter()
        .find(|equation| equation.starts_with("==(@set(f, s), @set(g, t)) = "))
        .expect("the set extensionality equation");
    assert!(
        set_equality.contains("Binder(Forall, [DataVarId(c, SortId(Nat))]"),
        "{set_equality}"
    );
}

/// The function-update operators take one index argument per domain sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_update_follows_the_arity() {
    for (text, sort, equation) in [
        (
            "map f: Nat -> Bool;",
            "[[Nat] -> Bool,Nat,Bool] -> [Nat] -> Bool",
            "@func_update(@f, @x0, @v) = @if_always_else(==(@f(@x0), @v), @f, @func_update_stable(@f, @x0, @v))",
        ),
        (
            "map f: Nat # Bool -> Nat;",
            "[[Nat,Bool] -> Nat,Nat,Bool,Nat] -> [Nat,Bool] -> Nat",
            "@func_update(@f, @x0, @x1, @v) = @if_always_else(==(@f(@x0, @x1), @v), @f, @func_update_stable(@f, @x0, @x1, @v))",
        ),
        (
            "map f: Nat # Bool # Nat -> Bool;",
            "[[Nat,Bool,Nat] -> Bool,Nat,Bool,Nat,Bool] -> [Nat,Bool,Nat] -> Bool",
            "@func_update(@f, @x0, @x1, @x2, @v) = @if_always_else(==(@f(@x0, @x1, @x2), @v), @f, @func_update_stable(@f, @x0, @x1, @x2, @v))",
        ),
    ] {
        let lowered = lowered(text);
        let sorts: Vec<String> = lowered
            .mappings()
            .iter()
            .filter(|map| map.name().value() == "@func_update")
            .map(|map| map.sort().to_string())
            .collect();
        assert_eq!(sorts, [sort], "{text}");
        assert!(
            lowered
                .equations()
                .iter()
                .any(|e| format!("{} = {}", e.lhs(), e.rhs()) == equation),
            "{text}: missing {equation}"
        );
    }
}
