//! Regression coverage for `extend_system_with_inferred_sorts`
//! (`crates/typecheck/src/lowering/instantiate.rs`): unlike its container-sort
//! top-up, it never re-discovers a *function* sort that only ever shows up as
//! an inferred node sort (never spelled out by any `map`/`cons` declaration or
//! function-sort binder). A `lambda`'s own result sort is exactly such a case
//! — nothing in the source text ever writes down `Pos -> Bool` — yet the
//! function-update syntax `expr[key -> value]` accepts *any* function-sorted
//! `expr`, not just a bare mapping name (`system_check.rs`'s `FunctionUpdate`
//! arm recurses into an arbitrary sub-expression with no such restriction).

use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_update_on_lambda_only_sort_gets_ground_equations() {
    // `Pos -> Bool` is never declared anywhere (no `map`/`cons` has that sort);
    // it only ever appears as the inferred sort of the two `lambda` values.
    // The specification is expected to type check (function-update syntax
    // applies to any function-sorted expression, not just a named mapping),
    // and, having type checked, the lowered specification must carry ground
    // `@func_update` equations for `Pos -> Bool` so the term the equation
    // produces is actually reducible.
    let text = "map b: Bool; \
                 eqn b = (lambda n: Pos. true)[1 -> false] == (lambda n: Pos. false);";
    let spec = UntypedDataSpecification::parse(text).expect("the specification should parse");
    let checked = DataSpecification::from_untyped(spec)
        .unwrap_or_else(|err| panic!("expected the specification to type check, got {err}"));

    let lowered = checked.lower_data_specification();
    let has_func_update_equation = lowered
        .equations()
        .iter()
        .any(|eqn| eqn.lhs().to_string().contains("func_update"));

    assert!(
        has_func_update_equation,
        "expected a ground @func_update equation for the Pos -> Bool sort used by \
         `(lambda n: Pos. true)[1 -> false]`, but none was generated — the resulting \
         @func_update(...) term in the equation's right-hand side has no rewrite rule \
         and can never reduce"
    );
}
