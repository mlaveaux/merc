//! Must NOT compile: `ExplicitContext` is not `Send` (its `LearnSuccessorsContext` field is
//! documented as thread-affine), so it must be rejected by an explicit `T: Send` bound.
//! Regression guard for the `unsafe impl Send for ExplicitContext` this repo used to have
//! (removed in `crates/merc_lps/src/explore_explicit.rs` — see the phase-1b review report).

use mcrl2::read_lps_text;
use merc_explore::LPS;
use merc_lps::ExplicitLinearProcessSpecification;

const LPS_TEXT: &str = "\
act a;
proc P(n: Nat) = (n < 3) -> a . P(n + 1);
init P(0);
";

fn assert_send<T: Send>(_: T) {}

fn main() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("explicit_context_not_send_{}.mcrl2", std::process::id()));
    std::fs::write(&path, LPS_TEXT).unwrap();
    let lps = read_lps_text(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let explicit = ExplicitLinearProcessSpecification::new(lps).unwrap();

    let context = explicit.create_context();
    assert_send(context);
}
