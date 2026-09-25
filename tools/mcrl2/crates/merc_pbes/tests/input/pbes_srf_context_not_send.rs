//! Must NOT compile: `PbesSrfContext` is not `Send` (its `LearnSuccessorsContext` field is
//! documented as thread-affine — must be created and destroyed on the same thread), so it must
//! be rejected by an explicit `T: Send` bound. Regression guard for the `unsafe impl Send for
//! PbesSrfContext` this repo used to have (removed in `crates/merc_pbes/src/explore_srf.rs` —
//! see the phase-1b review report).

use mcrl2::Pbes;
use mcrl2::SrfPbes;
use merc_explore::LPS;
use merc_pbes::PbesSrfLps;

const PBES_TEXT: &str = "\
map f: Nat -> Nat;
var x: Nat;
eqn f(x) = x;

pbes nu X(n: Nat) = val(n == 0) || X(f(n));
init X(f(1));
";

fn assert_send<T: Send>(_: T) {}

fn main() {
    let pbes = Pbes::from_text(PBES_TEXT).unwrap();
    let mut srf = SrfPbes::from(&pbes).unwrap();
    srf.unify_parameters(false, true).unwrap();
    let lps = PbesSrfLps::new(srf).unwrap();

    let context = lps.create_context();
    assert_send(context);
}
