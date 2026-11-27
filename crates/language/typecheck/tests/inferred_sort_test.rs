//! The sorts inferred for every subexpression of an equation, as shown by
//! [DataSpecification::to_typed_string], and the errors of equations that have no typing.

use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::InferenceError;
use merc_typecheck::WellTypedError;

/// The single equation of `text`, with every subexpression annotated with its inferred sort.
#[track_caller]
fn typed_equation(text: &str) -> String {
    let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap())
        .unwrap_or_else(|err| panic!("expected {text} to typecheck, got {err}"));
    let typed = spec.to_typed_string().to_string();
    let (_, equations) = typed.split_once("eqn\n").expect("the specification has an eqn section");
    let lines: Vec<_> = equations.lines().collect();
    assert_eq!(lines.len(), 1, "expected a single equation in {typed}");
    lines[0].trim().to_string()
}

#[track_caller]
fn inference_error(text: &str) -> InferenceError {
    match DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()) {
        Err(WellTypedError::Inference(error)) => error,
        Err(other) => panic!("expected an inference error for {text}, got {other}"),
        Ok(_) => panic!("expected {text} to be rejected"),
    }
}

/// The variable keeps its declared sort and the function its declared signature.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_infers_basic_equation() {
    assert_eq!(
        typed_equation("map f: Nat -> Bool; var n: Nat; eqn f(n) = true;"),
        "f(n: Nat): (Nat -> Bool) = true: Bool;"
    );
}

/// Literals keep their minimal sort `Pos`, and `+` resolves to its `Pos` overload.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_numeric_literals_stay_positive() {
    assert_eq!(
        typed_equation("map p: Pos; eqn p = 1 + 2;"),
        "p: Pos = +(1: Pos, 2: Pos): (Pos # Pos -> Pos);"
    );
}

/// The literal argument keeps its minimal sort; lowering inserts the upcast to `Int`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_literal_argument_keeps_minimal_sort_under_upcast() {
    assert_eq!(
        typed_equation("map f: Int -> Bool; b: Bool; eqn b = f(1);"),
        "b: Bool = f(1: Pos): (Int -> Bool);"
    );
}

/// The polymorphic `==` is instantiated at the user sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equality_scheme_on_user_sort() {
    assert_eq!(
        typed_equation("sort D; cons d: D; map b: Bool; eqn b = d == d;"),
        "b: Bool = ==(d: D, d: D): (D # D -> Bool);"
    );
}

/// The branches of `if` stay `Pos`; the equation joins them to `Nat`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_if_scheme_infers_element_sort() {
    assert_eq!(
        typed_equation("map n: Nat; eqn n = if(true, 1, 2);"),
        "n: Nat = if(true: Bool, 1: Pos, 2: Pos): (Bool # Pos # Pos -> Pos);"
    );
}

/// The element sort of an empty list that nothing constrains defaults to `Bool`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_free_element_sort_defaults_to_bool() {
    assert_eq!(
        typed_equation("map b: Bool; eqn b = [] == [];"),
        "b: Bool = ==([]: List(Bool), []: List(Bool)): (List(Bool) # List(Bool) -> Bool);"
    );
}

/// The literal stays `Pos` and is upcast into the join with the `Nat` left-hand side.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_sides_join_through_upcast() {
    assert_eq!(typed_equation("map f: Nat; eqn f = 1;"), "f: Nat = 1: Pos;");
}

/// A lambda has the function sort from its bound variable's sort to its body's sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_lambda_infers_function_sort() {
    assert_eq!(
        typed_equation("map f: Nat -> Bool; eqn f = lambda n: Nat. true;"),
        "f: (Nat -> Bool) = (lambda n: Nat . true: Bool): (Nat -> Bool);"
    );
}

/// A quantifier is `Bool`, and its variable has the declared sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_infers_bool_sort() {
    assert_eq!(
        typed_equation("map b: Bool; eqn b = forall n: Nat. n >= 0;"),
        "b: Bool = (forall n: Nat . >=(n: Nat, 0: Nat): (Nat # Nat -> Bool)): Bool;"
    );
}

/// A `whr` variable takes the sort of its assignment, not a declared sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_binds_variable_to_assignment_sort() {
    assert_eq!(
        typed_equation("map g: Nat; eqn g = (x + 1) whr x = 2 end;"),
        "g: Nat = +(x: Pos, 1: Pos): (Pos # Pos -> Pos) whr x = 2: Pos end: Pos;"
    );
}

/// Every assignment is typed in the outer scope, so `y`'s `x` is the declared `Nat` variable even though the same `whr` rebinds `x` to a `Bool`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_assignments_do_not_see_each_other() {
    assert_eq!(
        typed_equation("map f: Nat -> Bool; var x: Nat; eqn f(x) = true whr x = false, y = x end;"),
        "f(x: Nat): (Nat -> Bool) = true: Bool whr x = false: Bool, y = x: Nat end: Bool;"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_literal_takes_finite_set_sort() {
    assert_eq!(
        typed_equation("map s: FSet(Pos); eqn s = {1, 2};"),
        "s: FSet(Pos) = { 1: Pos, 2: Pos }: FSet(Pos);"
    );
}

/// The enumeration stays `FSet(Nat)` under the `Set(Nat)` join and the literals keep `Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_literal_widens_to_set_at_use() {
    assert_eq!(
        typed_equation("map s: Set(Nat); eqn s = {1, 2};"),
        "s: Set(Nat) = { 1: Pos, 2: Pos }: FSet(Nat);"
    );
}

/// The element sort is the join `Int`; the literal itself stays `Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_elements_join_to_common_supersort() {
    assert_eq!(
        typed_equation("map s: Int -> FSet(Int); var n: Int; eqn s(n) = {1, n};"),
        "s(n: Int): (Int -> FSet(Int)) = { 1: Pos, n: Int }: FSet(Int);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_empty_set_takes_element_sort_from_context() {
    assert_eq!(
        typed_equation("map s: Set(Nat); eqn s = {};"),
        "s: Set(Nat) = {}: FSet(Nat);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_free_empty_set_defaults_to_bool() {
    assert_eq!(
        typed_equation("map b: Bool; eqn b = {} == {};"),
        "b: Bool = ==({}: FSet(Bool), {}: FSet(Bool)): (FSet(Bool) # FSet(Bool) -> Bool);"
    );
}

/// The element is `Nat`, the count keeps its minimal sort `Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_literal_counts_are_natural() {
    assert_eq!(
        typed_equation("map b: FBag(Nat); eqn b = {0: 2};"),
        "b: FBag(Nat) = { 0: Nat: 2: Pos }: FBag(Nat);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_empty_bag_takes_element_sort_from_context() {
    assert_eq!(
        typed_equation("map b: Bag(Pos); eqn b = {:};"),
        "b: Bag(Pos) = {:}: FBag(Pos);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_comprehension_from_boolean_body() {
    assert_eq!(
        typed_equation("map s: Set(Nat); eqn s = { n: Nat | n < 3 };"),
        "s: Set(Nat) = { n: Nat | <(n: Nat, 3: Pos): (Nat # Nat -> Bool) }: Set(Nat);"
    );
}

/// A numeric body makes a bag; the body itself keeps its sort `Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_comprehension_from_numeric_body() {
    assert_eq!(
        typed_equation("map b: Pos -> Bag(Nat); var m: Pos; eqn b(m) = { n: Nat | m };"),
        "b(m: Pos): (Pos -> Bag(Nat)) = { n: Nat | m: Pos }: Bag(Nat);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_comprehension_from_positive_body() {
    assert_eq!(
        typed_equation("map b: Bag(Pos); eqn b = { p: Pos | 2 };"),
        "b: Bag(Pos) = { p: Pos | 2: Pos }: Bag(Pos);"
    );
}

/// The bound `n: Nat` shadows the `Bool` mapping `n` inside the comprehension only.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comprehension_variable_shadows_declarations() {
    assert_eq!(
        typed_equation("map n: Bool; s: Set(Nat); b: Bool; eqn b = ({ n: Nat | n < 3 } == s) && n;"),
        "b: Bool = &&(==({ n: Nat | <(n: Nat, 3: Pos): (Nat # Nat -> Bool) }: Set(Nat), s: Set(Nat)): (Set(Nat) # Set(Nat) -> Bool), n: Bool): (Bool # Bool -> Bool);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comprehension_over_alias() {
    assert_eq!(
        typed_equation("sort A = Nat; map s: Set(A); eqn s = { a: A | a < 3 };"),
        "s: Set(Nat) = { a: Nat | <(a: Nat, 3: Pos): (Nat # Nat -> Bool) }: Set(Nat);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comprehension_over_user_sort() {
    assert_eq!(
        typed_equation("sort D = struct d1 | d2; map s: Set(D); eqn s = { x: D | x == d1 };"),
        "s: Set(D) = { x: D | ==(x: D, d1: D): (D # D -> Bool) }: Set(D);"
    );
}

/// No container sort is declared, so `in` exists only through the polymorphic template.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_polymorphic_membership_over_undeclared_container_sort() {
    assert_eq!(
        typed_equation("sort D = struct a | b | c; map f: D -> Bool; var x: D; eqn f(x) = x in { a, b };"),
        "f(x: D): (D -> Bool) = in(x: D, { a: D, b: D }: FSet(D)): (D # FSet(D) -> Bool);"
    );
}

/// `head` and `|>` come from the list template; the elements stay `Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_polymorphic_list_operations() {
    assert_eq!(
        typed_equation("map n: Nat; eqn n = head([1, 2]);"),
        "n: Nat = head(|>(1: Pos, |>(2: Pos, []: List(Pos)): (Pos # List(Pos) -> List(Pos))): (Pos # List(Pos) -> List(Pos))): (List(Pos) -> Pos);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_update_is_polymorphic() {
    assert_eq!(
        typed_equation("map f: Nat -> Bool; g: Nat -> Bool; eqn g = f[1 -> true];"),
        "g: (Nat -> Bool) = @func_update(f: (Nat -> Bool), 1: Pos, true: Bool): (Nat -> Bool # Nat # Bool -> Nat -> Bool);"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_undeclared_name() {
    let text = "map b: Bool; eqn b = undeclared;";
    match inference_error(text) {
        InferenceError::UndeclaredName { name, span } => {
            assert_eq!(name, "undeclared");
            assert_eq!(&text[span.start..span.end], "undeclared");
        }
        other => panic!("expected UndeclaredName, got {other}"),
    }
}

/// Asserts that `text` is rejected because its equation, which prints as `expression`, has no
/// typing; the whole equation including its `;`, which ends `text`, is blamed.
#[track_caller]
fn assert_no_typing(text: &str, expression: &str) {
    match inference_error(text) {
        InferenceError::NoTyping {
            expression: actual,
            sort,
            span,
        } => {
            assert_eq!(actual, expression);
            assert_eq!(span.start, text.find("eqn ").unwrap() + "eqn ".len());
            assert_eq!(span.end, text.len());
            // A whole-equation check has no externally supplied expected sort.
            assert_eq!(sort, None);
        }
        other => panic!("expected NoTyping, got {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equations_without_typing() {
    assert_no_typing("map f: Bool; eqn f = 1;", "f = 1");
    assert_no_typing("map s: FSet(Nat); eqn s = {1, true};", "s = { 1, true }");
    // A bag count must be a natural number.
    assert_no_typing("map b: FBag(Nat); r: Real; eqn b = {0: r};", "b = { 0: r }");
    // A comprehension body must be `Bool` or a number.
    assert_no_typing(
        "map r: Real; s: Set(Nat); eqn s = { n: Nat | r };",
        "s = { n: Nat | r }",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_non_boolean_condition() {
    let text = "map f: Nat -> Bool; var n: Nat; eqn n -> f(n) = true;";
    match inference_error(text) {
        InferenceError::ConditionNotBool { condition, span } => {
            assert_eq!(condition, "n");
            // The span points at the condition, not at the variable declaration.
            assert_eq!(span.start, text.rfind("n ->").unwrap());
            assert_eq!(span.end, span.start + 1);
        }
        other => panic!("expected ConditionNotBool, got {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_requires_boolean_body() {
    let text = "map b: Bool; eqn b = forall n: Nat. n;";
    match inference_error(text) {
        InferenceError::QuantifierNotBool { body, span } => {
            assert_eq!(body, "n");
            // The span points at the body, not at the bound variable just before it.
            assert_eq!(span.start, text.len() - 2);
            assert_eq!(span.end, text.len() - 1);
        }
        other => panic!("expected QuantifierNotBool, got {other}"),
    }
}

/// `f(n)` types as both `Bool` (a set) and `Nat` (a bag), and `==` accepts either pair.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comprehension_readings_can_be_ambiguous() {
    let text = "map f: Nat -> Bool; f: Nat -> Nat; b: Bool; eqn b = { n: Nat | f(n) } == { n: Nat | f(n) };";
    match inference_error(text) {
        InferenceError::AmbiguousExpression { expression, span } => {
            assert_eq!(expression, "b = ==({ n: Nat | f(n) }, { n: Nat | f(n) })");
            assert_eq!(&text[span.start..span.end], &text[text.find("b = {").unwrap()..]);
        }
        other => panic!("expected AmbiguousExpression, got {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_product_binder_sort_is_rejected() {
    let text = "map s: Set(Nat); eqn s = { x: Nat # Nat | true };";
    match inference_error(text) {
        InferenceError::InvalidBinderSort { sort, span, .. } => {
            // `Display` parenthesizes the product; the span is the binder's own identifier.
            assert_eq!(sort, "(Nat # Nat)");
            assert_eq!(span.start, text.find("x:").unwrap());
            assert_eq!(span.end, span.start + 1);
        }
        other => panic!("expected InvalidBinderSort, got {other}"),
    }
}

/// The resolved sorts of the mappings of `text`, as `name: sort`.
fn mapping_sorts(text: &str) -> Vec<String> {
    let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse(text).unwrap()).unwrap();
    let typed = spec.to_typed_string().to_string();
    let (_, mappings) = typed.split_once("map\n").expect("the specification has a map section");
    mappings
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .map(|line| line.trim().trim_end_matches(';').to_string())
        .collect()
}

/// Declared sorts resolve through aliases to their normal form; a struct is its own nominal sort,
/// also behind an alias chain; function sorts keep their (flattened) domain.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_declared_sorts_are_resolved() {
    assert_eq!(mapping_sorts("map f: Nat;"), ["f: Nat"]);
    assert_eq!(mapping_sorts("sort D = Nat; map f: List(D);"), ["f: List(Nat)"]);
    assert_eq!(mapping_sorts("map f: Nat # Bool -> Real;"), ["f: (Nat # Bool -> Real)"]);
    assert_eq!(
        mapping_sorts("map f: (Nat -> Bool) -> Bool; g: Nat -> Bool;"),
        ["f: ((Nat -> Bool) -> Bool)", "g: (Nat -> Bool)"]
    );
    assert_eq!(mapping_sorts("sort D = struct a | b; map f: D;"), ["f: D"]);
    assert_eq!(
        mapping_sorts("sort A = B; B = struct c; map f: A; g: B;"),
        ["f: B", "g: B"]
    );
}

/// An equation variable has its declared sort, also through an alias onto a struct.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_variable_has_its_declared_sort() {
    assert_eq!(
        typed_equation("sort D; A = B; B = struct s; cons c: A; map g: A -> Bool; var x: A; eqn g(x) = x == c;"),
        "g(x: B): (B -> Bool) = ==(x: B, c: B): (B # B -> Bool);"
    );
}

/// The operators of the basic sorts come from their system-defined signatures, overloads
/// included.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_basic_sort_operators_are_resolved() {
    assert_eq!(
        typed_equation("map b: Bool; eqn b = true && false;"),
        "b: Bool = &&(true: Bool, false: Bool): (Bool # Bool -> Bool);"
    );
    assert_eq!(
        typed_equation("map n: Nat; eqn n = max(1, 0);"),
        "n: Nat = max(1: Pos, 0: Nat): (Pos # Nat -> Pos);"
    );
    assert_eq!(
        typed_equation("map n: Nat; eqn n = max(0, 0);"),
        "n: Nat = max(0: Nat, 0: Nat): (Nat # Nat -> Nat);"
    );
}
