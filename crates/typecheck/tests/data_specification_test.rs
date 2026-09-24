//! Data-specification type-checking tests.

use std::collections::HashSet;

use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::InferenceError;
use merc_typecheck::WellTypedError;
use merc_utilities::random_test;
use rand::Rng;
use rand::RngExt;

/// Type checks `text`, asserting it is accepted (`expect_ok`) or rejected.
#[track_caller]
fn check(text: &str, expect_ok: bool) {
    let spec = UntypedDataSpecification::parse(text).expect("the specification should parse");
    let result = DataSpecification::from_untyped(spec);
    assert_eq!(
        result.is_ok(),
        expect_ok,
        "unexpected type-check result for:\n{text}\nerror: {:?}",
        result.err()
    );
}

/// Type checks `text`, returning the error for the caller to match on the
/// specific variant.
#[track_caller]
fn check_err(text: &str) -> WellTypedError {
    let spec = UntypedDataSpecification::parse(text).expect("the specification should parse");
    match DataSpecification::from_untyped(spec) {
        Err(err) => err,
        Ok(_) => panic!("expected the specification to be rejected:\n{text}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_with_reused_projection() {
    // A recursive structured sort whose projection `p` is reused across
    // constructors is well-formed.
    check("sort S = struct c(p: Bool) | d(p: Bool, q: S);", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_sort_conflicting() {
    check(
        "sort S = struct c;
         S = Nat;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_and_mapping_same_symbol() {
    // The same symbol `f: S` cannot be declared as both a constructor and a
    // mapping.
    check(
        "sort S;
         cons f: S;
         map  f: S;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_overloaded_by_signature() {
    // `f` as a constant of `S` and as a function `S -> T` is allowed.
    check(
        "sort S;
              T;
         cons f: S;
              f: S -> T;",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_nested_inline_struct() {
    check("sort S = struct t(struct e(Nat));", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_cyclic_aliases_direct() {
    check(
        "sort S = U;
         U = S;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_cyclic_aliases_indirect() {
    check(
        "sort S = U;
         U = T;
         T = S;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_alias() {
    check(
        "sort Array = Nat -> Nat;
         map  update: Nat # Nat # Array -> Array;
         var  i,n: Nat;
              f: Array;
         eqn  update(i, n, f)  =  lambda j: Nat. if(i == j, n, f(j));",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_function_sort() {
    check(
        "sort G;
         F = F -> G;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_function_sort_reverse() {
    check(
        "sort G;
         F = G -> F;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bare_self_alias_rejected() {
    // Row A1 = A1: the shortest possible cycle.
    match check_err("sort A1 = A1;") {
        WellTypedError::AliasCycle { sorts, .. } if sorts.contains(&"A1".to_string()) => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bare_fset_fbag_self_alias_rejected() {
    // Plain cycles through structured sorts are allowed.
    match check_err("sort A12 = FSet(A12);") {
        WellTypedError::AliasCycle { sorts, .. } if sorts.contains(&"A12".to_string()) => {}
        other => panic!("unexpected error {other}"),
    }
    match check_err("sort A13 = FBag(A13);") {
        WellTypedError::AliasCycle { sorts, .. } if sorts.contains(&"A13".to_string()) => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bare_set_self_alias_rejected() {
    // A bare struct alias cycle, since Set is a function sort, is rejected as a
    // cycle through a function sort.
    match check_err("sort A3 = Set(A3);") {
        WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. } if sort == "A3" => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bare_bag_self_alias_rejected() {
    match check_err("sort A4 = Bag(A4);") {
        WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. } if sort == "A4" => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_alias_loop_via_list_of_struct() {
    // `B`'s only reference to itself goes through both `List` and a struct
    // constructor, so it is accepted.
    check("sort B = List(struct f(B)); map g: B; eqn g = [];", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_alias_loop_via_list_of_struct_with_extra_constant() {
    check("sort B = List(struct f(B) | c); map g: B; eqn g = [];", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_constructor_named_like_containing_alias() {
    // A struct constructor sharing its name with the alias it belongs to.
    check("sort B; A11 = struct A11 | B;", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_wrapping_fset_and_fbag_self_recursive() {
    // Struct-boxed recursion through the *finite* containers `FSet`/`FBag`,
    // distinct from the already-tested `Set`/function-sort cases (those hit
    // `RecursiveAliasThroughFunctionSort`; these do not, since FSet/FBag do
    // not set the flag).
    check("sort A14 = struct f(FSet(A14)) | c;", true);
    check("sort A15 = struct f(FSet(A15)) | g(FBag(A15)) | c;", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_without_base_case_is_empty() {
    // A single self-recursive constructor with no base case has no finite
    // element.
    match check_err("sort D = struct f(D);") {
        WellTypedError::EmptySort { sort, .. } if sort == "D" => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sort_name_reused_as_map_and_variable() {
    // `S` is a sort, a mapping and an equation variable at once; the variable
    // shadows the mapping inside the equation, so `S(S)` applies the
    // non-function variable and the equation is rejected.
    check(
        "sort S;
         map  S: S -> Bool;
         var  S: S;
         eqn  S(S)  =  S == S;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_via_function_codomain() {
    // Struct recursion in a function sort's *codomain*.
    match check_err("sort G = struct f(Nat -> G);") {
        WellTypedError::RecursiveAliasThroughFunctionSort { sort, .. } if sort == "G" => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_recursive_struct_list_indirect() {
    // Struct recursion through a List alias one level removed.
    check(
        "sort LP = List(P);
         P = struct b(x: LP);",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_variables_in_var_block() {
    check(
        "sort S;
         map g: Bool;
         var x: Nat;
             x: S;
         eqn g = (x == x + 1);",
        false,
    );
    check(
        "sort S;
         map g: Bool;
         var x: S;
             x: Nat;
         eqn g = (x == x + 1);",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_normalize_sorts_across_equations() {
    // Struct aliases used by mappings and equations together.
    check(
        "sort Bit = struct e0 | e1;
              AbsBit = struct arbitrary;
         map  inv: Bit -> Bit;
              h: Bit -> AbsBit;
              abseq: AbsBit # AbsBit -> Set(Bool);
              absinv: AbsBit -> Set(AbsBit);
         eqn  inv(e0) = e1;
              inv(e1) = e0;",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_constant_different_sort_rejected_cons_cons() {
    check("sort S; T; cons f: S; f: T;", false);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_duplicate_constant_different_sort_rejected_cons_map() {
    check("sort S; T; cons f: S; map f: T;", false);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_cross_struct_duplicate_constant_name_rejected() {
    check(
        "sort front_doorstate = struct open | closed;
         rear_doorstate  = struct open | closed;",
        false,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_user_declaration_shadowing_system_conversion_rejected() {
    check("map Nat2Pos: Nat -> Pos;", false);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_many_aliases_to_nat_and_struct() {
    check(
        "sort A_t = Nat; B_t = Nat; C_t = Nat; D_t = Nat; E_t = Nat; F_t = Nat; G_t = Nat;
         H_t = Nat; I_t = Nat; J_t = Nat; K_t = Nat; L_t = Nat; M_t = Nat; N_t = Nat; O_t = Nat;
         S_t = struct s(a: A_t, b: B_t, c: C_t, d: D_t, e: E_t, f: F_t, g: G_t, h: H_t,
                        i: I_t, j: J_t, k: K_t, l: L_t, m: M_t, n: N_t, o: O_t);",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_concat_of_element_and_list_is_rejected() {
    match check_err("map place: List(Nat) -> List(Nat); var l: List(Nat); eqn place(l) = head(l) ++ tail(l);") {
        WellTypedError::Inference(InferenceError::NoTyping { .. }) => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_concat_of_two_lists_is_accepted() {
    check(
        "map place: List(Nat) -> List(Nat); var l: List(Nat); eqn place(l) = l ++ tail(l);",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mapping_named_like_a_system_function_prefix_is_accepted() {
    check("sort S = struct c; map succ_: S -> S; eqn succ_(c) = c;", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_two_parameter_mapping_over_distinct_abstract_sorts_is_accepted() {
    check(
        "sort S, T; map count_: S # T -> Nat; var x: S; y: T; eqn count_(x, y) = 0;",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_one_parameter_mapping_over_an_abstract_sort_is_accepted() {
    check("sort S; map count_: S -> Nat; var x: S; eqn count_(x) = 0;", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_redeclaring_the_system_count_function_is_rejected() {
    match check_err("map count: Pos # Bag(Pos) -> Nat; map f: Nat -> Pos; map g: Nat; eqn g = count(3, {3:4});") {
        WellTypedError::SystemFunctionRedeclared { name, .. } => assert_eq!(name, "count"),
        other => panic!("unexpected error {other}"),
    }
}

/// `@` is reserved for Appendix B's own generated content (`@c0`, `@cPair`, `@zero_`, …),
/// regardless of whether this particular name happens to already exist there.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_at_prefixed_mapping_is_rejected_even_without_a_name_collision() {
    match check_err("map @my_helper: Nat; map f: Nat; eqn f = @my_helper;") {
        WellTypedError::SystemFunctionRedeclared { name, .. } => assert_eq!(name, "@my_helper"),
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_at_prefixed_constructor_is_rejected() {
    match check_err("sort D; cons @weird: D;") {
        WellTypedError::SystemFunctionRedeclared { name, .. } => assert_eq!(name, "@weird"),
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constructor_for_the_built_in_bool_sort_is_rejected() {
    match check_err("cons maybe: Bool;") {
        WellTypedError::ConstructorForBasicSort { constructor, sort, .. } => {
            assert_eq!(constructor, "maybe");
            assert_eq!(sort, "Bool");
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_projection_applied_inside_a_lambda_argument_is_accepted() {
    check(
        "sort S = struct c(proj: Int);
         map  f: (S -> Bool) # S -> S;
         var  pre: S -> Bool;
              s: S;
         eqn  f(pre, s) = s;
         map  g: S;
         eqn  g = f(lambda x: S. proj(x) < 0, c(0));",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_projection_named_x_applied_inside_a_lambda_argument_is_accepted() {
    check(
        "sort S = struct c(x: Int);
         map  f: (S -> Bool) # S -> S;
         var  pre: S -> Bool;
              s: S;
         eqn  f(pre, s) = s;
         map  g: S;
         eqn  g = f(lambda i: S. x(i) < 0, c(0));",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constant_mapping_passed_to_another_mapping_is_accepted() {
    check(
        "map const: Pos; f: Nat -> Pos;
         eqn  const = 10;
         map  g: Pos;
         eqn  g = f(const);",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_pos_literal_upcasts_to_real_in_an_addition() {
    check(
        "sort T = Real;
         map  x: List(T) -> List(T);
         var  l: List(T);
              r: T;
         eqn  x(r |> l) = (r + 0) |> l;",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_unapplied_function_as_a_set_comprehension_body_is_rejected() {
    match check_err("map b: Bool # Pos -> Nat; map s: Set(Nat); eqn s = { n: Nat | b };") {
        WellTypedError::Inference(InferenceError::NoTyping { .. }) => {}
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_unapplied_function_as_an_equation_condition_is_rejected() {
    match check_err("map b: Bool # Pos -> Nat; map n: Nat; eqn b -> n = 0;") {
        WellTypedError::Inference(InferenceError::ConditionNotBool { condition, .. }) => {
            assert_eq!(condition, "b");
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constant_mapping_upcasts_from_pos_to_nat() {
    check("map const: Pos; eqn const = 10; map g: Nat; eqn g = const;", true);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constant_mapping_round_trips_through_pos_nat_conversions() {
    check(
        "map const: Pos; eqn const = 10; map g: Nat; eqn g = Nat2Pos(Pos2Nat(const));",
        true,
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_constant_mapping_converted_from_pos_to_nat() {
    check(
        "map const: Pos; eqn const = 10; map g: Nat; eqn g = Pos2Nat(const);",
        true,
    );
}

/// Picks either one of the already-declared sorts or a built-in sort.
fn random_leaf(rng: &mut impl Rng, earlier: &[String]) -> String {
    const BASICS: [&str; 5] = ["Bool", "Nat", "Pos", "Int", "Real"];
    if !earlier.is_empty() && rng.random_bool(0.6) {
        earlier[rng.random_range(0..earlier.len())].clone()
    } else {
        BASICS[rng.random_range(0..BASICS.len())].to_string()
    }
}

/// Builds a random (non-structured) sort expression over `earlier` sorts and the
/// built-in sorts, up to `depth` container/function nestings.
fn random_sort(rng: &mut impl Rng, earlier: &[String], depth: u32) -> String {
    if depth == 0 || rng.random_bool(0.4) {
        return random_leaf(rng, earlier);
    }
    match rng.random_range(0..3u32) {
        0 => {
            const CONTAINERS: [&str; 5] = ["List", "Set", "Bag", "FSet", "FBag"];
            let container = CONTAINERS[rng.random_range(0..CONTAINERS.len())];
            format!("{container}({})", random_sort(rng, earlier, depth - 1))
        }
        1 => format!(
            "({} -> {})",
            random_leaf(rng, earlier),
            random_sort(rng, earlier, depth - 1)
        ),
        _ => random_leaf(rng, earlier),
    }
}

/// Collects the names of every resolved (nominal) sort in `sort`.
fn collect_resolved_names(sort: &SortExpression, out: &mut Vec<String>) {
    match &sort.node {
        SortExpressionKind::Resolved(name, _) => out.push(name.clone()),
        SortExpressionKind::Complex(_, subsort) => collect_resolved_names(subsort, out),
        SortExpressionKind::Function { domain, range } => {
            collect_resolved_names(domain, out);
            collect_resolved_names(range, out);
        }
        SortExpressionKind::FlattenedFunction { domain, range } => {
            for sort in domain {
                collect_resolved_names(sort, out);
            }
            collect_resolved_names(range, out);
        }
        SortExpressionKind::Product { lhs, rhs } => {
            collect_resolved_names(lhs, out);
            collect_resolved_names(rhs, out);
        }
        SortExpressionKind::Struct { inner } => {
            for constructor in inner {
                for (_, sort) in &constructor.args {
                    collect_resolved_names(sort, out);
                }
            }
        }
        SortExpressionKind::Simple(_)
        | SortExpressionKind::Reference(_)
        | SortExpressionKind::TypeVar(_)
        | SortExpressionKind::ResolvedTypeVar(_) => {}
    }
}

/// Random acyclic alias graphs must type check, and normalization must fully
/// expand every non-structured alias — no normalized declaration may still
/// refer to one. Because every alias only refers to earlier sorts the graph is a
/// DAG, so there are no cycles and the specification is always well-typed.
#[test]
#[cfg_attr(miri, ignore)]
fn test_random_acyclic_aliases_are_normalized() {
    random_test(100, |rng| {
        let count = rng.random_range(2..8usize);
        let names: Vec<String> = (0..count).map(|i| format!("D{i}")).collect();
        let mut non_struct_aliases: HashSet<String> = HashSet::new();

        let mut sorts = String::from("sort ");
        for (i, name) in names.iter().enumerate() {
            let earlier = &names[0..i];
            match rng.random_range(0..3u32) {
                // Abstract sort.
                0 => sorts.push_str(&format!("{name};\n")),
                // Non-structured alias over earlier sorts.
                1 => {
                    sorts.push_str(&format!("{name} = {};\n", random_sort(rng, earlier, 3)));
                    non_struct_aliases.insert(name.clone());
                }
                // Structured-sort alias (a named representative, kept by normalization).
                _ => {
                    let argument = random_leaf(rng, earlier);
                    sorts.push_str(&format!("{name} = struct c{i}a({argument}) | c{i}b;\n"));
                }
            }
        }

        // Force every sort into a mapping so its normalized form is inspectable.
        let mut maps = String::from("map ");
        for (i, name) in names.iter().enumerate() {
            maps.push_str(&format!("g{i}: {name};\n"));
        }
        let text = format!("{sorts}{maps}");

        let spec = UntypedDataSpecification::parse(&text).unwrap_or_else(|e| panic!("should parse:\n{text}\n{e:?}"));
        let checked =
            DataSpecification::from_untyped(spec).unwrap_or_else(|e| panic!("should type check:\n{text}\n{e:?}"));

        for map in &checked.data_specification().map_declarations {
            let mut resolved = Vec::new();
            collect_resolved_names(&map.sort, &mut resolved);
            for name in resolved {
                assert!(
                    !non_struct_aliases.contains(&name),
                    "normalized sort of {} still refers to non-struct alias {name}:\n{text}",
                    map.identifier
                );
            }
        }
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_container_scheme_usable_alongside_numeric_operator_of_same_name() {
    // `used_by` includes the `Set` container template because `S` aliases `Set(Nat)`, so the
    // `+` disjunction must offer both the numeric overloads and the `Set` union scheme.
    check(
        "sort S = Set(Nat);
         map f: S # S -> S;
         var s, t: S;
         eqn f(s, t) = s + t;",
        true,
    );
}
