//! The lowered form of user equations, through [lower_data_specification]: sorts, literals,
//! coercions, container literals and binders.

use merc_data::is_container_sort;
use merc_data::is_function_sort;
use merc_lowering::lower_data_specification;
use merc_lowering::lower_expression;
use merc_syntax::DataExpr;
use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::NumberEncoding;

fn typed(text: &str, encoding: NumberEncoding) -> DataSpecification {
    let untyped = UntypedDataSpecification::parse(text).unwrap();
    DataSpecification::from_untyped_with(untyped, encoding, &mut SourceMap::new()).unwrap()
}

/// The right-hand side of the only lowered equation of `text` whose left-hand side prints as
/// `lhs`; that equation must have no condition.
#[track_caller]
fn rhs_of(text: &str, lhs: &str) -> String {
    let lowered = lower_data_specification(&typed(text, NumberEncoding::Binary));
    let equations: Vec<_> = lowered
        .equations()
        .iter()
        .filter(|equation| equation.lhs().to_string() == lhs)
        .collect();
    assert_eq!(equations.len(), 1, "expected one equation with left-hand side {lhs}");
    assert!(equations[0].condition().is_none());
    equations[0].rhs().to_string()
}

/// The lowered sort of the only mapping named `name` in `text`.
#[track_caller]
fn mapping_sort(text: &str, name: &str) -> String {
    let lowered = lower_data_specification(&typed(text, NumberEncoding::Binary));
    let mappings: Vec<_> = lowered
        .mappings()
        .iter()
        .filter(|mapping| mapping.name().value() == name)
        .collect();
    assert_eq!(mappings.len(), 1, "expected one mapping {name}");
    mappings[0].sort().to_string()
}

/// The standalone literal `text` lowered under `encoding`, at its own inferred sort.
fn literal(text: &str, encoding: NumberEncoding) -> String {
    let mut spec = typed("map f: Bool;", encoding);
    let typed = spec.typecheck_expression(&DataExpr::parse(text).unwrap()).unwrap();
    lower_expression(&spec, &typed).to_string()
}

/// `2^64` and `2^128`, the first values needing two and three machine words.
const TWO_POW_64: &str = "18446744073709551616";
const TWO_POW_128: &str = "340282366920938463463374607431768211456";

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_declared_sorts_are_lowered() {
    let text = "sort D; map f: Nat; g: List(Nat); h: Nat -> Bool; k: D; m: Nat # Bool -> List(Nat);";
    assert_eq!(mapping_sort(text, "f"), "Nat");
    assert_eq!(mapping_sort(text, "g"), "List(Nat)");
    assert_eq!(mapping_sort(text, "h"), "[Nat] -> Bool");
    assert_eq!(mapping_sort(text, "k"), "D");
    assert_eq!(mapping_sort(text, "m"), "[Nat,Bool] -> List(Nat)");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_pos_literals() {
    let binary = NumberEncoding::Binary;
    assert_eq!(literal("1", binary), "@c1");
    assert_eq!(literal("2", binary), "@cDub(false, @c1)");
    assert_eq!(literal("3", binary), "@cDub(true, @c1)");
    assert_eq!(literal("5", binary), "@cDub(true, @cDub(false, @c1))");
    assert_eq!(
        literal("255", binary),
        format!("{}@c1{}", "@cDub(true, ".repeat(7), ")".repeat(7))
    );

    let word = NumberEncoding::MachineWord;
    assert_eq!(literal("1", word), "@most_significant_digit(1)");
    assert_eq!(literal("255", word), "@most_significant_digit(255)");
    // Two and three digits, most significant innermost.
    assert_eq!(
        literal(TWO_POW_64, word),
        "@concat_digit(@most_significant_digit(1), 0)"
    );
    assert_eq!(
        literal(TWO_POW_128, word),
        "@concat_digit(@concat_digit(@most_significant_digit(1), 0), 0)"
    );
}

/// A literal is rebuilt at the sort of the equation it occurs in, not wrapped in a coercion.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_literals_at_the_equation_sort() {
    assert_eq!(rhs_of("map p: Pos; eqn p = 1;", "p"), "@c1");
    assert_eq!(rhs_of("map n: Nat; eqn n = 0;", "n"), "@c0");
    assert_eq!(rhs_of("map n: Nat; eqn n = 2;", "n"), "@cNat(@cDub(false, @c1))");
    assert_eq!(rhs_of("map i: Int; eqn i = 0;", "i"), "@cInt(@c0)");
    assert_eq!(rhs_of("map r: Real; eqn r = 0;", "r"), "@cReal(@cInt(@c0), @c1)");
    assert_eq!(rhs_of("map r: Real; eqn r = 1;", "r"), "@cReal(@cInt(@cNat(@c1)), @c1)");
}

/// The `@cDub` function symbol carries its own function sort `Bool # Pos -> Pos`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_literal_sort_is_embedded() {
    let mut spec = typed("map f: Bool;", NumberEncoding::Binary);
    let typed = spec.typecheck_expression(&DataExpr::parse("2").unwrap()).unwrap();
    let cdub = lower_expression(&spec, &typed);
    let sort = cdub.data_function_symbol().sort();
    assert!(is_function_sort(&sort));
    assert_eq!(sort.to_string(), "[Bool,Pos] -> Pos");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_user_op_application_no_coercion() {
    assert_eq!(rhs_of("map f: Bool -> Bool; var x: Bool; eqn f(x) = x;", "f(x)"), "x");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comparison_and_if_on_declared_sort() {
    assert_eq!(
        rhs_of("sort D; cons d: D; map b: Bool; eqn b = (d == d);", "b"),
        "==(d, d)"
    );
    assert_eq!(
        rhs_of("sort D; cons d: D; map f: D; eqn f = if(true, d, d);", "f"),
        "if(true, d, d)"
    );
}

/// The narrower side of an equation is widened, by the constructors directly (`@cNat`), not by a
/// call to a conversion function such as `Pos2Nat`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_level_coercion() {
    assert_eq!(rhs_of("map n: Nat; eqn n = 1;", "n"), "@cNat(@c1)");
    assert_eq!(rhs_of("map n: Nat; eqn 1 = n;", "@cNat(@c1)"), "n");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_argument_coercion_widens_to_domain() {
    assert_eq!(rhs_of("map f: Nat -> Bool; eqn f(1) = true;", "f(@cNat(@c1))"), "true");
}

/// A finite set or bag argument is widened by the `@set`/`@bag` constructor directly, not by the
/// rewrite-system-only `@setfset`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_finite_container_argument_widens() {
    assert_eq!(
        rhs_of(
            "map e: FSet(Nat); map s: Set(Nat) -> Bool; eqn s(e) = true;",
            "s(@set(@false_, e))"
        ),
        "true"
    );
    assert_eq!(
        rhs_of(
            "map e: FBag(Nat); map s: Bag(Nat) -> Bool; eqn s(e) = true;",
            "s(@bag(@zero_, e))"
        ),
        "true"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_container_literals() {
    assert_eq!(rhs_of("map s: List(Nat); eqn s = [];", "s"), "[]");
    assert_eq!(rhs_of("map s: FSet(Nat); eqn s = {};", "s"), "{}");
    assert_eq!(rhs_of("map b: FBag(Nat); eqn b = {:};", "b"), "{:}");
    assert_eq!(
        rhs_of("map s: Nat -> FSet(Nat); var n: Nat; eqn s(n) = {n};", "s(n)"),
        "@fset_insert(n, {})"
    );
    assert_eq!(
        rhs_of(
            "map s: Nat # Nat -> FSet(Nat); var n: Nat; m: Nat; eqn s(n, m) = {n, m};",
            "s(n, m)"
        ),
        "@fset_insert(n, @fset_insert(m, {}))"
    );
    // The multiplicity `1` infers `Pos` and is widened to `Nat`, as is the element of `{1}`.
    assert_eq!(
        rhs_of("map b: Nat -> FBag(Nat); var n: Nat; eqn b(n) = {n: 1};", "b(n)"),
        "@fbag_cinsert(n, @cNat(@c1), {:})"
    );
    assert_eq!(
        rhs_of("map s: FSet(Nat); eqn s = {1};", "s"),
        "@fset_insert(@cNat(@c1), {})"
    );
}

/// The `[]` constant carries its `List(Nat)` sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_empty_list_sort_is_embedded() {
    let lowered = lower_data_specification(&typed("map s: List(Nat); eqn s = [];", NumberEncoding::Binary));
    let equation = lowered
        .equations()
        .iter()
        .find(|equation| equation.lhs().to_string() == "s")
        .unwrap();
    let sort = equation.rhs().data_sort();
    assert!(is_container_sort(&sort));
    assert_eq!(sort.to_string(), "List(Nat)");
}

/// Binders keep their bound variables with the declared sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_binders() {
    let x = "DataVarId(x, SortId(Bool))";
    assert_eq!(
        rhs_of("map f: Bool -> Bool; eqn f = lambda x: Bool. x;", "f"),
        format!("Binder(Lambda, [{x}], {x})")
    );
    assert_eq!(
        rhs_of("map b: Bool; eqn b = forall x: Bool. x;", "b"),
        format!("Binder(Forall, [{x}], {x})")
    );
    assert_eq!(
        rhs_of("map b: Bool; eqn b = exists x: Bool. x;", "b"),
        format!("Binder(Exists, [{x}], {x})")
    );
    assert_eq!(
        rhs_of("map f: Bool; var x: Bool; eqn f = x whr x = true end;", "f"),
        format!("Whr({x}, [DataVarIdInit({x}, OpIdNoIndex(true, SortId(Bool)))])")
    );
}

/// A comprehension becomes the `@set`/`@bag` constructor applied to its characteristic function,
/// not a `SetComp` binder.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_comprehensions() {
    assert_eq!(
        rhs_of("map s: Set(Nat); eqn s = { x: Nat | x == 0 };", "s"),
        "@set(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(==, SortArrow([SortId(Nat),SortId(Nat)], SortId(Bool))), DataVarId(x, SortId(Nat)), OpIdNoIndex(@c0, SortId(Nat)))), {})"
    );
    // The body counts multiplicities, so it has sort `Nat`.
    assert_eq!(
        rhs_of("map b: Bag(Nat); eqn b = { x: Nat | x + 0 };", "b"),
        "@bag(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(+, SortArrow([SortId(Nat),SortId(Nat)], SortId(Nat))), DataVarId(x, SortId(Nat)), OpIdNoIndex(@c0, SortId(Nat)))), {:})"
    );
    // A `Pos` body is widened to `Nat`.
    assert_eq!(
        rhs_of("map b: Bag(Nat); eqn b = { x: Nat | 1 };", "b"),
        "@bag(Binder(Lambda, [DataVarId(x, SortId(Nat))], DataAppl(OpIdNoIndex(@cNat, SortArrow([SortId(Pos)], SortId(Nat))), OpIdNoIndex(@c1, SortId(Pos)))), {:})"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_builtin_operators() {
    assert_eq!(
        rhs_of(
            "map n: Nat # Nat -> Nat; var a: Nat; b: Nat; eqn n(a, b) = a + b;",
            "n(a, b)"
        ),
        "+(a, b)"
    );
    assert_eq!(
        rhs_of(
            "map b: Nat # Set(Nat) -> Bool; var n: Nat; s: Set(Nat); eqn b(n, s) = n in s;",
            "b(n, s)"
        ),
        "in(n, s)"
    );
    assert_eq!(
        rhs_of(
            "map f: Nat -> Bool; map g: Nat -> Nat -> Bool; var n: Nat; eqn g(n) = f[n -> true];",
            "g(n)"
        ),
        "@func_update(f, n, true)"
    );
}
