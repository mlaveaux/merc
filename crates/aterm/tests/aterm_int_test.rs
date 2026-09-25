//! Regression tests for defects in `aterm_int.rs` (`ATermInt` / `is_int_term`).

use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::Symbol;
use merc_aterm::Term;
use merc_aterm::is_int_term;

/// `is_int_term` recognizes a term purely by comparing its head symbol against
/// the pool's reserved `<aterm_int>` marker symbol. Since the symbol pool used
/// to share symbols by name and arity with no reserved-name guard,
/// `Symbol::new("<aterm_int>", 0)` used to hand out that very same reserved
/// symbol, so a constant built from it was indistinguishable from a real
/// integer term even though it carries no annotation value. `ATermInt::value`
/// would then read past the end of the term's allocation.
///
/// `Symbol::new` now panics on any name/arity that collides with one of the
/// pool's reserved marker symbols, so forging one this way is no longer
/// possible at all.
#[test]
#[should_panic(expected = "reserved for internal use")]
fn test_user_symbol_cannot_forge_reserved_int_symbol() {
    let _ = Symbol::new("<aterm_int>", 0);
}

/// A legitimate integer term must still be recognized as one.
#[test]
fn test_real_int_term_is_still_recognized() {
    let real = ATermInt::new(42);
    assert!(is_int_term(&ATerm::from(real)));
}

/// Boundary case for the reserved-name guard: only the exact `(name, arity)` pair that matches
/// the pool's reserved `<aterm_int>` marker (arity 0) is blocked. The same *name* at a
/// *different* arity is a legitimate, distinct symbol -- `resolve_read_symbol` and `is_int_term`
/// both key off the actual protected symbol's identity, not the name string, so a term built
/// from this symbol must not be mistaken for a real integer term (which has no annotation value
/// to read via `value_unchecked`).
#[test]
fn test_boundary_reserved_name_at_different_arity_is_not_an_int_term() {
    let spoof_head = Symbol::new("<aterm_int>", 1);
    let arg = ATerm::constant(&Symbol::new("boundary_spoof_arg", 0));
    let term = ATerm::with_args(&spoof_head, &[arg]).protect();

    assert!(
        !is_int_term(&term),
        "a symbol merely named \"<aterm_int>\" at a non-reserved arity must not be treated as \
         the reserved integer marker"
    );
}

/// Boundary values for the payload `ATermInt::value_unchecked` reads via a raw pointer cast:
/// the minimum (0) and maximum (`usize::MAX`) representable values must round-trip exactly.
#[test]
fn test_boundary_aterm_int_min_and_max_value_round_trip() {
    let zero = ATermInt::new(0);
    assert_eq!(zero.value(), 0);

    let max = ATermInt::new(usize::MAX);
    assert_eq!(max.value(), usize::MAX);
}
