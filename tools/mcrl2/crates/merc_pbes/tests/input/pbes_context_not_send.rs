//! Must NOT compile: `PbesContext` is not `Send` (its `PbesRewriteContext` field is documented
//! as single-threaded), so it must be rejected by an explicit `T: Send` bound. Regression guard
//! for the `unsafe impl Send for PbesContext` this repo used to have (removed in
//! `crates/merc_pbes/src/explore_pbes.rs` — see the phase-1b review report).

use mcrl2::Pbes;
use merc_explore::LPS;
use merc_pbes::PbesLps;

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
    let lps = PbesLps::new(pbes).unwrap();

    let context = lps.create_context();
    assert_send(context);
}
