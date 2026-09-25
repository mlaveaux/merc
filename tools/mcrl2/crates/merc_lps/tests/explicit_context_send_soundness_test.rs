//! Regression test for the `unsafe impl Send for ExplicitContext` in
//! `crates/merc_lps/src/explore_explicit.rs`.
//!
//! Same bug class as `merc_pbes`'s `pbes_srf_context_send_soundness_test.rs`
//! and `pbes_context_send_soundness_test.rs`, on the explicit-state LPS
//! explorer: `ExplicitContext` embeds an `mcrl2::LearnSuccessorsContext`,
//! documented as needing to be "created and destroyed on the same thread"
//! (`crates/mcrl2/src/lps.rs`). `LearnSuccessorsContext` is correctly
//! `!Send` (`PhantomUnsend`), but `ExplicitContext` overrides that with
//! `unsafe impl Send`.

use mcrl2::read_lps_text;
use merc_explore::LPS;
use merc_explore::Summand;
use merc_lps::ExplicitLinearProcessSpecification;

const LPS_TEXT: &str = "\
act a;
proc P(n: Nat) = (n < 3) -> a . P(n + 1);
init P(0);
";

#[test]
fn explicit_context_created_on_one_thread_is_usable_on_another() {
    // `read_lps_text` takes a *file path*, not the spec text directly.
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "explicit_context_send_soundness_test_{}.mcrl2",
        std::process::id()
    ));
    std::fs::write(&path, LPS_TEXT).expect("failed to write temp LPS spec file");
    let lps = read_lps_text(path.to_str().unwrap()).expect("LPS text should parse and linearise");
    let _ = std::fs::remove_file(&path);
    let explicit = ExplicitLinearProcessSpecification::new(lps).expect("ExplicitLinearProcessSpecification::new should succeed");

    // Created HERE, on the test's main thread.
    let mut context = explicit.create_context();

    let explicit_ref = &explicit;
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                // Everything from here on runs on a *different* OS thread than
                // the one that created `context`'s `LearnSuccessorsContext`.
                let explicit = explicit_ref;
                let state = explicit.initial_state();
                let mut fired = 0usize;
                for summand_idx in explicit.prepare(&mut context, &state) {
                    explicit.summands()[summand_idx]
                        .enumerate(&mut context, &state, |_label, _next_state| {
                            fired += 1;
                            Ok(())
                        })
                        .expect("enumerate should not itself error for this LPS");
                }
                fired
            })
            .join()
    });

    match outcome {
        Ok(fired) => {
            eprintln!(
                "cross-thread use of ExplicitContext did not crash ({fired} solutions found); \
                 the Send impl is still unsound against LearnSuccessorsContext's documented \
                 same-thread requirement."
            );
        }
        Err(_) => panic!(
            "cross-thread use of an ExplicitContext created on a different thread panicked/aborted, \
             consistent with LearnSuccessorsContext's documented thread affinity being violated \
             by `unsafe impl Send for ExplicitContext`"
        ),
    }
}
