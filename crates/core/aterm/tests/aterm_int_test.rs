//! Regression tests for defects in `aterm_int.rs` (`ATermInt` / `is_int_term`).

use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::Symbol;
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

#[test]
fn test_int_term_value() {
    let int_term = ATermInt::new(42);

    assert_eq!(int_term.value(), 42);
    assert!(is_int_term(&int_term));
}
