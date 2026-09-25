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
//! This is exercised through the public `DataSpecification`/`ATerm` API rather
//! than by constructing terms directly, since every `DataExpression` in this
//! crate is ultimately backed by the C++ mCRL2 term pool.

use mcrl2::DataApplicationRef;
use mcrl2::DataSpecification;
use mcrl2::is_application;

/// Parses a tiny data specification with one equation `f(x) = x` and returns
/// the `f(x)` application term that appears as the left-hand side of the
/// user-defined equation `eqn f(x) = x;`.
fn find_application_in_spec() -> mcrl2::ATerm {
    let spec = DataSpecification::from_string("map f: Nat -> Nat;\nvar x: Nat;\neqn f(x) = x;\n");

    for eq in spec.user_defined_equations().iter() {
        let arity = eq.get_head_symbol().arity();
        for i in 0..arity {
            let arg = eq.arg(i);
            if is_application(&arg) {
                return arg.protect();
            }
        }
    }

    panic!("expected the data_equation term for `f(x) = x` to contain a data application `f(x)`");
}

/// `data_function_symbol()` on a `DataApplication` correctly returns the
/// applied head symbol (`f`), establishing that `arg(0)` really is the
/// function symbol term and not a sort expression, as a baseline before
/// exercising the `sort()` bug below.
#[test]
fn data_application_head_symbol_is_the_function_not_a_sort() {
    let term = find_application_in_spec();
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
/// make this test's precondition ("calling sort() on a real application")
/// stop panicking, so this test would need to be replaced by one that asserts
/// the sort is e.g. `Nat` — it does not currently, because the bug is live.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not satisfy is_sort_expression")]
fn data_application_sort_panics_on_type_confusion_in_debug() {
    let term = find_application_in_spec();
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
    let term = find_application_in_spec();
    let appl = DataApplicationRef::from(term.copy());
    let sort = appl.sort();

    // A correct `sort()` for `f(x)` (`f: Nat -> Nat`) would report the result
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
