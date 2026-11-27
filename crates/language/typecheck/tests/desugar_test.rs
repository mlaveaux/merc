//! The declarations and equations that struct desugaring adds to a specification.

use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

fn typed(text: &str) -> DataSpecification {
    DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap()
}

/// The constructors and mappings of the type-checked specification of `text`, as `name: sort`.
fn constructors_and_mappings(text: &str) -> (Vec<String>, Vec<String>) {
    let checked = typed(text);
    let spec = checked.data_specification();
    let constructors = spec
        .constructor_declarations
        .iter()
        .map(|declaration| format!("{}: {}", declaration.identifier.node, declaration.sort))
        .collect();
    let mappings = spec
        .map_declarations
        .iter()
        .map(|declaration| format!("{}: {}", declaration.identifier.node, declaration.sort))
        .collect();
    (constructors, mappings)
}

/// The system-defined equations of `text` that a specification without user declarations does
/// not have.
fn added_system_equations(text: &str) -> Vec<String> {
    let equations = |spec: &DataSpecification| -> Vec<String> {
        spec.system_defined_specification()
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|equation| equation.to_string())
            .collect()
    };
    let basic = equations(&typed("map z: Bool;"));
    equations(&typed(text))
        .into_iter()
        .filter(|equation| !basic.contains(equation))
        .collect()
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_desugars_to_constructors() {
    let (constructors, mappings) = constructors_and_mappings("sort D = struct c1(p1: Bool)?is_c1 | c2;");
    assert_eq!(constructors, ["c1: (Bool -> D)", "c2: D"]);
    assert_eq!(mappings, ["is_c1: (D -> Bool)", "p1: (D -> Bool)"]);
}

/// `p` is shared by `c` and `d` with the same sort, so only one mapping is generated for it.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_reused_projection_is_generated_once() {
    let (constructors, mappings) = constructors_and_mappings("sort S = struct c(p: Bool) | d(p: Bool, q: S);");
    assert_eq!(constructors, ["c: (Bool -> S)", "d: (Bool # S -> S)"]);
    assert_eq!(mappings, ["p: (S -> Bool)", "q: (S -> S)"]);
}

/// The recogniser and projection equations, and the comparison equations, of every constructor.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_equations_are_in_system_spec() {
    let equations = added_system_equations("sort D = struct c1(pr1: Nat, pr2: Bool)?is_c1 | c2?is_c2 | c3(Nat);");
    assert_eq!(
        equations[..8],
        [
            "is_c1(c1(@x0_0, @x0_1)) = true",
            "is_c1(c2) = false",
            "is_c1(c3(@x2_0)) = false",
            "is_c2(c2) = true",
            "is_c2(c1(@x0_0, @x0_1)) = false",
            "is_c2(c3(@x2_0)) = false",
            "pr1(c1(@x0_0, @x0_1)) = @x0_0",
            "pr2(c1(@x0_0, @x0_1)) = @x0_1",
        ]
    );
    for equation in [
        "==(c1(@x0_0, @x0_1), c1(@y0_0, @y0_1)) = &&(==(@x0_0, @y0_0), ==(@x0_1, @y0_1))",
        "==(c2, c3(@y2_0)) = false",
        "<(c1(@x0_0, @x0_1), c1(@y0_0, @y0_1)) = ||(<(@x0_0, @y0_0), &&(==(@x0_0, @y0_0), <(@x0_1, @y0_1)))",
        "<=(c3(@x2_0), c3(@y2_0)) = <=(@x2_0, @y2_0)",
    ] {
        assert!(equations.iter().any(|actual| actual == equation), "missing {equation}");
    }
    // `c3` has no recogniser.
    assert!(equations.iter().all(|equation| !equation.contains("is_c3")));
}

/// A struct of constants only gets the comparison equations, which need no variables.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_of_constants_gets_comparison_equations() {
    let equations = added_system_equations("sort E = struct red | green;");
    assert_eq!(
        equations[..4],
        [
            "==(red, red) = true",
            "==(red, green) = false",
            "==(green, green) = true",
            "==(green, red) = false",
        ]
    );
    assert!(equations.contains(&"<(red, green) = true".to_string()));
    assert!(equations.contains(&"<=(green, red) = false".to_string()));
}

/// The desugared constructors, recognisers and projections resolve like declared ones.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_desugared_symbols_can_be_used() {
    let spec = typed(
        "sort D = struct c1(pr1: Nat)?is_c1; map f: Set(D); b: Bool; n: Nat; eqn b = is_c1(c1(1)); n = pr1(c1(1));",
    );
    let typed = spec.to_typed_string().to_string();
    assert!(
        typed.contains("b: Bool = is_c1(c1(1: Pos): (Nat -> D)): (D -> Bool);"),
        "{typed}"
    );
    assert!(
        typed.contains("n: Nat = pr1(c1(1: Pos): (Nat -> D)): (D -> Nat);"),
        "{typed}"
    );
}

/// An anonymous struct in a mapping declaration becomes a fresh abstract sort without
/// constructors: mCRL2 only generates constructors for structs in sort declarations. Identical
/// ones share that sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_anonymous_struct_in_mapping_is_abstract() {
    let (constructors, mappings) = constructors_and_mappings("map f: struct c | d;");
    assert!(constructors.is_empty(), "{constructors:?}");
    assert_eq!(mappings, ["f: @struct0"]);

    let (constructors, mappings) = constructors_and_mappings("map f: struct c;\n    g: struct c;");
    assert!(constructors.is_empty(), "{constructors:?}");
    assert_eq!(mappings, ["f: @struct0", "g: @struct0"]);
}

/// The struct nested inside `t`'s argument is hoisted and desugared, so its constructor `e` is
/// declared too.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_nested_anonymous_struct_is_desugared() {
    let (constructors, _) = constructors_and_mappings("sort S = struct t(struct e(Nat));");
    assert_eq!(constructors, ["t: (@struct0 -> S)", "e: (Nat -> @struct0)"]);
}

/// An anonymous struct that matches a named struct is that sort, so no second declaration is
/// generated.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_anonymous_struct_reuses_named_alias() {
    let (constructors, mappings) = constructors_and_mappings("sort D = struct c;\nmap f: struct c;");
    assert_eq!(constructors, ["c: D"]);
    assert_eq!(mappings, ["f: D"]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_structs_are_non_empty() {
    // A base constructor, or abstract argument sorts that are assumed non-empty.
    typed("sort Tree = struct leaf | node(Tree, Tree);");
    typed("sort A;\n     B;\nsort S = struct c(A) | d(B);");
}
