use std::path::Path;
use std::process::Command;

use mcrl2::Pbes;
use mcrl2::SrfPbes;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_io::temp_dir;
use merc_io::traced_command;
use merc_utilities::Timing;
use merc_utilities::random_test;
use merc_vpg::PG;
use merc_vpg::ParityGameBuilder;
use merc_vpg::VertexIndex;
use merc_vpg::solve_zielonka;

use merc_pbes::CfgPbesSrfLps;
use merc_pbes::explore_srf_pbes;

/// Converts `pbes` to SRF form and unifies its parameter lists, as every
/// SRF-based explorer requires (see `PbesSrfLps::new`).
fn unified_srf(pbes: &Pbes) -> SrfPbes {
    let mut srf = SrfPbes::from(pbes).expect("Failed to convert to SRF");
    srf.unify_parameters(false, true).expect("Failed to unify parameters");
    srf
}

/// Explores `pbes` (normalised to positive normal form) both with the plain SRF
/// explorer and with the control-flow-pruning variant, and asserts the two
/// parity games have equal vertex/edge counts and agree on the winner of the
/// initial vertex. Pruning summands whose source-value condition cannot hold
/// must never change the explored parity game.
fn assert_cfg_matches_srf(pbes: &Pbes) {
    let mut normalised = pbes.clone();
    normalised.normalize();

    let reference = explore_srf_pbes(
        unified_srf(&normalised),
        ExplorationStrategy::Bfs,
        CachingStrategy::None,
        false,
        &Timing::new(),
        ParityGameBuilder::new(VertexIndex::new(0)),
    )
    .expect("SRF exploration failed");

    let cfg = explore_srf_pbes(
        unified_srf(&normalised),
        ExplorationStrategy::Bfs,
        CachingStrategy::None,
        true,
        &Timing::new(),
        ParityGameBuilder::new(VertexIndex::new(0)),
    )
    .expect("Control flow SRF exploration failed");

    assert_eq!(
        reference.num_of_vertices(),
        cfg.num_of_vertices(),
        "Vertex count mismatch between plain and control-flow-pruned SRF exploration"
    );
    assert_eq!(
        reference.num_of_edges(),
        cfg.num_of_edges(),
        "Edge count mismatch between plain and control-flow-pruned SRF exploration"
    );

    let (reference_solution, _) = solve_zielonka(&reference, false);
    let (cfg_solution, _) = solve_zielonka(&cfg, false);
    assert_eq!(
        reference_solution[0][0], cfg_solution[0][0],
        "Control-flow-pruned and plain SRF parity games disagree on the initial vertex's winner"
    );
}

/// Runs `mcrl22lps` and `lps2pbes` on an mCRL2 spec and modal formula, and
/// asserts that control-flow and plain SRF exploration agree on the resulting
/// PBES.
fn compare_cfg_with_srf(spec_relative_path: &str, formula_relative_path: &str) {
    let Ok(mcrl2_path) = std::env::var("MCRL2_PATH") else {
        println!("Skipping test: MCRL2_PATH not set");
        return;
    };

    let mcrl22lps = Path::new(&mcrl2_path).join("mcrl22lps");
    let lps2pbes = Path::new(&mcrl2_path).join("lps2pbes");

    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(spec_relative_path);
    let formula_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(formula_relative_path);
    assert!(spec_path.exists(), "Spec file not found: {}", spec_path.display());
    assert!(
        formula_path.exists(),
        "Formula file not found: {}",
        formula_path.display()
    );

    let temp = temp_dir("test_cfg_srf").unwrap();
    let lps_path = temp.path().join("spec.lps");
    let pbes_path = temp.path().join("spec.pbes");

    let status =
        traced_command(Command::new(&mcrl22lps).arg(&spec_path).arg(&lps_path)).expect("Failed to execute mcrl22lps");
    assert!(status.success(), "mcrl22lps failed with status: {status}");

    let status = traced_command(
        Command::new(&lps2pbes)
            .arg("-f")
            .arg(&formula_path)
            .arg(&lps_path)
            .arg(&pbes_path),
    )
    .expect("Failed to execute lps2pbes");
    assert!(status.success(), "lps2pbes failed with status: {status}");

    let pbes = Pbes::from_file(pbes_path.to_str().unwrap()).expect("Failed to read PBES");
    assert_cfg_matches_srf(&pbes);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_abp() {
    compare_cfg_with_srf(
        "../../../../examples/mCRL2/academic/abp/abp.mcrl2",
        "../../../../examples/mCRL2/academic/abp/nodeadlock.mcf",
    );
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_dining3() {
    compare_cfg_with_srf(
        "../../../../examples/mCRL2/academic/dining/dining3.mcrl2",
        "../../../../examples/mCRL2/academic/dining/nodeadlock.mcf",
    );
}

/// A hand-written PBES whose `s: Nat` parameter is a genuine control flow
/// parameter: every summand either constrains it via `s == c` or leaves it
/// unchanged, and every change assigns it a closed value. This exercises the
/// actual source-value pruning (not just the equation-index pruning that
/// applies regardless), unlike the propositional-only random PBESs below.
///
/// Every disjunct is a guarded call `val(cond) && Target(...)`, never a bare
/// `val(cond)`, so mCRL2's boilerplate `true`/`false` SRF sink equations are
/// never actually targeted and stay unreachable from `X` — otherwise their
/// unconditional resets of every unified parameter would (correctly) disqualify
/// `s`, since those sinks would then be part of the reachable automaton too.
const CFP_PBES: &str = "
    pbes mu X(s: Nat, b: Bool) =
           (val(s == 0) && Y(1, b)) ||
           (val(s == 1) && X(0, !b));
         mu Y(s: Nat, b: Bool) =
           val(s == 1) && X(0, b);
    init X(0, false);
";

#[test]
fn test_cfg_finds_control_flow_parameter() {
    let mut pbes = Pbes::from_text(CFP_PBES).expect("Failed to parse PBES");
    pbes.normalize();

    let lps = CfgPbesSrfLps::new(unified_srf(&pbes)).expect("Failed to build control-flow SRF view");
    assert_eq!(
        lps.control_flow_parameters().len(),
        1,
        "expected the `s` parameter to be identified as the sole control flow parameter"
    );
}

#[test]
fn test_cfg_matches_srf_with_control_flow_parameter() {
    let pbes = Pbes::from_text(CFP_PBES).expect("Failed to parse PBES");
    assert_cfg_matches_srf(&pbes);
}

/// A PBES over unordered pairs, in which `pair(2, 1) == pair(1, 2)` holds
/// although they are different normal forms, so the summand of `X` guarded by
/// `p == pair(1, 2)` can fire in location `pair(2, 1)`. Obtained with
/// `lps2pbes` from `<t(7)> mu Z. (<t(7)>true || <true>Z)` on an unordered pair
/// specification similar to `merc_lps/tests/upair.mcrl2`.
const UPAIR_PBES: &str = "
    sort UPair;
    cons pair: Nat # Nat -> UPair;
    var x, y, z, w: Nat;
    eqn pair(x, y) == pair(z, w) = x == z && y == w || x == w && y == z;

    pbes nu X1(p: UPair, m: Nat) =
           val(m == 7) && val(p == pair(3, 4)) && X(pair(2, 1), m);
         mu X(p: UPair, m: Nat) =
           val(m == 7) && val(p == pair(3, 4))
           || val(p == pair(3, 4)) && X(p, if(0 < m, Int2Nat(m - 1), 0))
           || val(p == pair(3, 4)) && X(pair(2, 1), m)
           || val(p == pair(1, 2)) && X(pair(3, 4), m);
    init X1(pair(3, 4), 7);
";

#[test]
fn test_cfg_matches_srf_with_user_defined_equality() {
    let pbes = Pbes::from_text(UPAIR_PBES).expect("Failed to parse PBES");
    assert_cfg_matches_srf(&pbes);
}

#[test]
fn test_cfg_locations_are_values() {
    let mut pbes = Pbes::from_text(UPAIR_PBES).expect("Failed to parse PBES");
    pbes.normalize();

    let lps = CfgPbesSrfLps::new(unified_srf(&pbes)).expect("Failed to build control-flow SRF view");
    assert_eq!(
        lps.control_flow_parameters().len(),
        1,
        "`==` decides the equality of the locations of `p`, so it is a control flow parameter"
    );
}

/// Without equations for `==` on `D`, `a == b` does not rewrite to `true` or
/// `false`, so the locations of `d` are unknown and it is not a control flow
/// parameter.
#[test]
fn test_cfg_undecided_locations_are_not_control_flow() {
    let mut pbes = Pbes::from_text(
        "sort D;
         map a, b: D;
         pbes mu X(d: D) = val(d == a) && Y(b);
              mu Y(d: D) = val(d == b) && X(a);
         init X(a);",
    )
    .expect("Failed to parse PBES");
    pbes.normalize();

    let lps = CfgPbesSrfLps::new(unified_srf(&pbes)).expect("Failed to build control-flow SRF view");
    assert!(lps.control_flow_parameters().is_empty());
}

/// Generates random PBESs with [`random_pbes`](merc_syntax::random_pbes) and
/// asserts that control-flow and plain SRF exploration agree on each.
#[test]
fn test_cfg_random_pbes() {
    use merc_syntax::random_pbes;

    random_test(50, |rng| {
        // Propositional only (no quantifiers, no integers), matching the other
        // `random_pbes`-based tests in this crate — stable, type-correct state
        // spaces.
        let pbes_ast = random_pbes(rng, 0, 0, 3, 2, 3, false, false);
        let pbes = Pbes::from_text(&pbes_ast.to_string()).expect("parse failed");
        assert_cfg_matches_srf(&pbes);
    });
}
