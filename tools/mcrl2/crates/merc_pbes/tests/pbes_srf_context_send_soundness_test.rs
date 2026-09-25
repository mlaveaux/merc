//! Regression test for the `unsafe impl Send for PbesSrfContext` in
//! `crates/merc_pbes/src/explore_srf.rs`.
//!
//! `PbesSrfContext` embeds an `mcrl2::LearnSuccessorsContext`, whose own doc
//! comment says: "The context owns a per-thread mCRL2 enumerator backed by
//! thread-affine `aterm`s, so it must be created and destroyed on the same
//! thread." (`crates/mcrl2/src/lps.rs`). `LearnSuccessorsContext` itself is
//! correctly `!Send` (it carries a `PhantomUnsend` marker), but
//! `PbesSrfContext` overrides that with `unsafe impl Send`, so safe code can
//! create the context on one OS thread and use it on another — exactly what
//! the inner type's own contract forbids.
//!
//! This test demonstrates the *type-level* half of that unsoundness: safe
//! code, using only the public API, can compile a program that creates a
//! `PbesSrfContext` on the main thread and moves it into a `std::thread`,
//! where `Summand::enumerate` and `LPS::prepare` are then called on it. No
//! `unsafe` appears anywhere in this test. Whether this crashes, hangs, or
//! merely explores wrongly depends on the C++ enumerator's actual thread
//! affinity, which is exactly why `PbesSrfContext`'s own field
//! (`LearnSuccessorsContext`) already documents it as forbidden — this test
//! exercises the forbidden path via the public API `unsafe impl Send` opens.

use mcrl2::Pbes;
use mcrl2::SrfPbes;
use merc_explore::LPS;
use merc_explore::Summand;
use merc_pbes::PbesSrfLps;

const PBES_TEXT: &str = "\
map f: Nat -> Nat;
var x: Nat;
eqn f(x) = x;

pbes nu X(n: Nat) = val(n == 0) || X(f(n));
init X(f(1));
";

#[test]
fn pbes_srf_context_created_on_one_thread_is_usable_on_another() {
    let pbes = Pbes::from_text(PBES_TEXT).expect("PBES text should parse and type-check");
    let mut srf = SrfPbes::from(&pbes).expect("PBES should convert to SRF form");
    srf.unify_parameters(false, true).expect("parameters should unify");

    let lps = PbesSrfLps::new(srf).expect("PbesSrfLps::new should succeed for this PBES");

    // Created HERE, on the test's main thread: this is where
    // `LearnSuccessorsContext::from_data_spec` runs and where the C++
    // `learn_successors_context`/enumerator object is constructed.
    let mut context = lps.create_context();

    // `PbesSrfContext: Send` (the unsafe impl under test) is exactly what
    // makes the following line compile: without it, `context` could not be
    // moved into the spawned thread's closure. `&lps` crossing the thread
    // boundary is unrelated and unproblematic: `PbesSrfLps: Sync` is a
    // separate, independently-checked claim (see the review report).
    let lps_ref = &lps;
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                // Everything from here on runs on a *different* OS thread than
                // the one that created `context`'s `LearnSuccessorsContext`.
                // `lps_ref: &PbesSrfLps` is captured by reference (Send
                // because `PbesSrfLps: Sync`); only `context` is moved.
                let lps = lps_ref;
                let state = lps.initial_state();
                let mut fired = 0usize;
                for summand_idx in lps.prepare(&mut context, &state) {
                    lps.summands()[summand_idx]
                        .enumerate(&mut context, &state, |_label, _next_state| {
                            fired += 1;
                            Ok(())
                        })
                        .expect("enumerate should not itself error for this PBES");
                }
                fired
            })
            .join()
    });

    match outcome {
        Ok(fired) => {
            // The call did not crash the process. This does not vindicate
            // `unsafe impl Send for PbesSrfContext`: `LearnSuccessorsContext`'s
            // own doc comment states the constraint unconditionally, not
            // "unless it happens not to crash today". A silent pass here is
            // exactly the "not `Send`, but built without anything that
            // enforces it at compile time" hazard `PhantomUnsend` exists to
            // prevent for `LearnSuccessorsContext`, and `PbesSrfContext`
            // reopens by asserting `Send` over a field that says otherwise.
            eprintln!(
                "cross-thread use of PbesSrfContext did not crash ({fired} solutions found); \
                 the Send impl is still unsound against LearnSuccessorsContext's documented \
                 same-thread requirement — see the module doc comment on this test."
            );
        }
        Err(_) => panic!(
            "cross-thread use of a PbesSrfContext created on a different thread panicked/aborted, \
             consistent with LearnSuccessorsContext's documented thread affinity being violated \
             by `unsafe impl Send for PbesSrfContext`"
        ),
    }
}
