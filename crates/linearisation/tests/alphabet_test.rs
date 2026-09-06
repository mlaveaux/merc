//! Tests for the alphabet-based axiom elimination pass (`alphabet::simplify`,
//! run automatically by `translate::linearise_process` — see
//! `docs/linearisation-implementation-plan.md` and the `alphabet` module's
//! own documentation). Most of what the pass does is externally
//! unobservable by design (every operator it simplifies away already has
//! correct standalone semantics in `translate`, so the *result* is
//! identical whether or not the wrapper survives) — these tests target the
//! two places where it *is* observable: `comm` becoming linearisable when
//! it turns out to be a no-op (`translate_test.rs` covers this directly),
//! and this module's performance safeguard against the alphabet of a wide
//! parallel composition.

use std::fmt::Write as _;
use std::time::Duration;
use std::time::Instant;

use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

/// A wide `||` composition is exactly the shape `alphabet::alphabet_of`'s
/// `Parallel` case can blow up on: each additional operand's alphabet gets
/// cross-multiplied against everything combined so far, so the naive
/// (uncapped) computation is exponential in the number of operands. This
/// specification wraps 40 single-action processes — 2^40 possible merged
/// multi-actions if computed exactly — in an outer `hide`, forcing
/// `simplify` to compute (an approximation of) the whole composition's
/// alphabet before `translate` ever gets to reject the `||` itself.
///
/// This is a regression test for `alphabet::MAX_PARALLEL_ALPHABET`: without
/// it, this call does not return in any practical amount of time.
#[test]
fn test_alphabet_of_a_wide_parallel_composition_stays_bounded() {
    const COMPONENTS: usize = 40;

    let mut text = String::new();
    for i in 0..COMPONENTS {
        writeln!(text, "act a{i};").unwrap();
        writeln!(text, "proc P{i} = a{i};").unwrap();
    }
    write!(text, "init hide({{a0}}, P0").unwrap();
    for i in 1..COMPONENTS {
        write!(text, " || P{i}").unwrap();
    }
    write!(text, ");").unwrap();

    let untyped = UntypedProcessSpecification::parse(&text).expect("generated specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped).expect("generated specification should type check");

    let start = Instant::now();
    let result = merc_linearisation::linearise(&spec);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "alphabet computation over a {COMPONENTS}-way parallel composition took {elapsed:?}; \
         the MAX_PARALLEL_ALPHABET cap should keep this fast regardless of width"
    );
    // `||` itself is still unsupported (Milestone 3) -- only the *alphabet
    // computation* `simplify` runs first needs to stay cheap, not the
    // parallel composition itself.
    assert!(
        result.is_err(),
        "'||' is not supported yet, so this must still be rejected"
    );
}
