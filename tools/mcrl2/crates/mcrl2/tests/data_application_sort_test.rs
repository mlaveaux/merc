//! Regression tests for `DataApplication::sort()` / `DataApplicationRef::sort()`
//! (see `crates/mcrl2/src/data_expression.rs`).
//!
//! `sort()` is documented as "Returns the sort of a data application", but it
//! used to be byte-for-byte identical to `data_function_symbol()`: it
//! reinterpreted `arg(0)` of the application (the applied function/head term,
//! e.g. `f` in `f(x)`) as a `SortExpressionRef`, instead of computing the
//! application's actual result sort. `arg(0)` is never a `sort_expression`, so
//! the reinterpretation was a straightforward runtime-tag mismatch of the kind
//! the `mcrl2_term` macro's `debug_assert!` exists to catch.
//!
//! `sort()` now computes the actual result sort: the codomain of the head
//! symbol's (arrow) sort, mirroring the `application` case of
//! `data_expression::sort()` in the vendored mCRL2 C++ (`data.cpp`).
//!
//! The application term is obtained from a `Pbes`'s initial state rather than
//! from `DataSpecification::user_defined_equations()`: the latter returns
//! terms in the index-*stripped* `OpIdNoIndex` serialisation (see
//! `lowering_conformance.rs`), which does not satisfy `is_function_symbol`
//! either, and would make a baseline sanity check panic for an unrelated
//! reason. A PBES initial state is built by the real mCRL2 parser/type-checker
//! pipeline and yields genuine indexed `data_expression` terms, exactly the
//! shape every other wrapper in `data_expression.rs` assumes.

use mcrl2::DataApplicationRef;
use mcrl2::Pbes;
use mcrl2::is_application;

/// Parses a tiny PBES whose initial state passes the data application `f(1)`
/// as an argument, and returns that application term.
fn find_application_in_initial_state() -> mcrl2::DataExpression {
    let pbes = Pbes::from_text(
        "map f: Nat -> Nat;\n\
         var x: Nat;\n\
         eqn f(x) = x;\n\
         \n\
         pbes nu X(n: Nat) = val(n == 0) || X(f(n));\n\
         init X(f(1));\n",
    )
    .expect("PBES text should parse and type-check");

    let initial = pbes.initial_state();
    for arg in initial.arguments().iter() {
        if is_application(&arg.copy()) {
            return arg.copy().protect();
        }
    }

    panic!("expected the initial state instantiation `X(f(1))` to carry the application `f(1)`");
}

/// `data_function_symbol()` on a `DataApplication` correctly returns the
/// applied head symbol (`f`), establishing that `arg(0)` really is the
/// function symbol term and not a sort expression, as a baseline before
/// exercising `sort()` below.
#[test]
fn data_application_head_symbol_is_the_function_not_a_sort() {
    let term = find_application_in_initial_state();
    let appl = DataApplicationRef::from(term.copy());
    assert_eq!(appl.data_function_symbol().name(), "f");
}

/// `DataApplicationRef::sort()` must return the application's actual result
/// sort, i.e. the codomain of `f`'s arrow sort (`Nat -> Nat`), not `f` itself
/// reinterpreted as a sort. This is the regression test for the type-confusion
/// bug: it used to fail (`sort().name()` returned `"f"`, the function's own
/// name) because `sort()` just returned `arg(0)` unchanged. It now passes in
/// both debug and release builds, since the fix does not rely on a
/// `debug_assert!` to catch the mismatch -- it computes the right term.
#[test]
fn data_application_sort_is_the_result_sort_not_the_function_symbol() {
    let term = find_application_in_initial_state();
    let appl = DataApplicationRef::from(term.copy());
    let sort = appl.sort();

    assert_eq!(
        sort.pretty_print(),
        "Nat",
        "sort() of `f(1)` (f: Nat -> Nat) should be the codomain `Nat`, not the function symbol"
    );
    assert_eq!(
        sort.name(),
        "Nat",
        "sort() must not return the applied function symbol itself"
    );
}
