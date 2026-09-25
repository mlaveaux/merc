//! Regression test for the `unsafe impl Send for PbesContext` in
//! `crates/merc_pbes/src/explore_pbes.rs`.
//!
//! Same bug class as `pbes_srf_context_send_soundness_test.rs`, on the direct
//! (non-SRF) PBES explorer this time: `PbesContext` embeds an
//! `mcrl2::PbesRewriteContext`, whose own doc comment says "Not `Send`: the
//! underlying C++ rewriter is single-threaded. Clone the PBES and construct a
//! separate context per thread." (`crates/mcrl2/src/pbes.rs`).
//! `PbesRewriteContext` is correctly `!Send` (`PhantomData<*const ()>`), but
//! `PbesContext` overrides that with `unsafe impl Send`, so safe code can
//! create the context on one OS thread and drive it on another.

use mcrl2::Pbes;
use merc_explore::LPS;
use merc_explore::Summand;
use merc_pbes::PbesLps;

const PBES_TEXT: &str = "\
map f: Nat -> Nat;
var x: Nat;
eqn f(x) = x;

pbes nu X(n: Nat) = val(n == 0) || X(f(n));
init X(f(1));
";

#[test]
fn pbes_context_created_on_one_thread_is_usable_on_another() {
    let pbes = Pbes::from_text(PBES_TEXT).expect("PBES text should parse and type-check");
    let lps = PbesLps::new(pbes).expect("PbesLps::new should succeed for this PBES");

    // Created HERE, on the test's main thread: this is where
    // `PbesRewriteContext::from_data_spec` runs and where the C++
    // `enumerate_quantifiers_rewriter` is constructed.
    let mut context = lps.create_context();

    let lps_ref = &lps;
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                // Everything from here on runs on a *different* OS thread than
                // the one that created `context`'s `PbesRewriteContext`.
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
            eprintln!(
                "cross-thread use of PbesContext did not crash ({fired} solutions found); \
                 the Send impl is still unsound against PbesRewriteContext's documented \
                 single-thread requirement."
            );
        }
        Err(_) => panic!(
            "cross-thread use of a PbesContext created on a different thread panicked/aborted, \
             consistent with PbesRewriteContext's documented thread affinity being violated \
             by `unsafe impl Send for PbesContext`"
        ),
    }
}
