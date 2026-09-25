//! Regression tests for `DataApplication::sort()` / `DataApplicationRef::sort()`
//! (see `crates/mcrl2/src/data_expression.rs`).
//!
//! `sort()` is documented as "Returns the sort of a data application", but its
//! implementation is byte-for-byte identical to `data_function_symbol()`: it
//! reinterprets `arg(0)` of the application (the applied function/head term,
//! e.g. `f` in `f(x)`) as a `SortExpressionRef`, instead of computing the
//! application's actual result sort. `arg(0)` is never a `sort_expression`, so
//! the reinterpretation is a straightforward runtime-tag mismatch of the kind
//! the `mcrl2_term` macro's `debug_assert!` exists to catch.
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
use mcrl2::DataExpressionRef;
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
            return DataExpressionRef::from(arg.copy()).protect();
        }
    }

    panic!("expected the initial state instantiation `X(f(1))` to carry the application `f(1)`");
}

/// `data_function_symbol()` on a `DataApplication` correctly returns the
/// applied head symbol (`f`), establishing that `arg(0)` really is the
/// function symbol term and not a sort expression, as a baseline before
/// exercising the `sort()` bug below.
#[test]
fn data_application_head_symbol_is_the_function_not_a_sort() {
    let term = find_application_in_initial_state();
    let appl = DataApplicationRef::from(term.copy());
    assert_eq!(appl.data_function_symbol().name(), "f");
}

/// In a debug build, `DataApplicationRef::sort()` must not silently succeed:
/// its result is fed through `SortExpressionRef::from`, whose `mcrl2_term`
/// macro asserts `is_sort_expression(&term)`. Because `sort()` actually
/// returns `arg(0)` (the function symbol `f`, not a sort), that assertion is
/// false and the conversion panics. This failing debug_assert *is* the
/// evidence of the type-confusion bug: fixing `sort()` to compute the actual
/// result sort (e.g. by decomposing the function symbol's arrow sort) would
/// make this precondition stop panicking, so this test would then need to be
/// replaced by one that asserts the sort is `Nat` — it is not currently,
/// because the bug is live.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not satisfy is_sort_expression")]
fn data_application_sort_panics_on_type_confusion_in_debug() {
    let term = find_application_in_initial_state();
    let appl = DataApplicationRef::from(term.copy());
    let _ = appl.sort();
}

/// In a release build the `debug_assert!` inside `SortExpressionRef::from` is
/// compiled out, so `DataApplicationRef::sort()` returns *silently wrong*
/// data instead of panicking: the value it returns is actually the function
/// symbol `f`, not `f`'s result sort `Nat`. `SortExpression::name()` reads
/// off the same `arg(0)` leaf that `DataFunctionSymbol::name()` reads its
/// name from, so the mis-tagged `SortExpressionRef` happens to print a
/// plausible-looking (but wrong) string: the function's name instead of a
/// sort name. Run this test with `cargo test --release` to observe it; under
/// `cfg(debug_assertions)` the panic above fires first.
#[cfg(not(debug_assertions))]
#[test]
fn data_application_sort_silently_returns_function_symbol_not_result_sort_in_release() {
    let term = find_application_in_initial_state();
    let appl = DataApplicationRef::from(term.copy());
    let sort = appl.sort();

    // A correct `sort()` for `f(1)` (`f: Nat -> Nat`) would report the result
    // sort `Nat`. Instead it reports "f", proving the returned handle is
    // actually still the function symbol.
    assert_eq!(
        sort.name(),
        "f",
        "sort() should be wrong today (returns the function symbol's name); \
         if this now fails, sort() has been fixed and this assertion should \
         be replaced by `assert_eq!(sort.pretty_print(), \"Nat\")`"
    );
    assert_ne!(
        sort.pretty_print(),
        "Nat",
        "a correct implementation would print the application's actual result sort"
    );
}
