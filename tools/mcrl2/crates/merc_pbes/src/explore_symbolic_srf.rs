use log::debug;
use mcrl2::SrfPbes;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::Value;

use merc_symbolic::ExplorationStrategy;
use merc_symbolic::ReachabilityOptions;
use merc_symbolic::SymbolicLPS;
use merc_symbolic::SymbolicLps;
use merc_symbolic::SymbolicLpsOptions;
use merc_symbolic::reachability_with_options;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::Player;
use merc_vpg::Priority;
use merc_vpg::SymbolicParityGame;

use crate::explore_srf::PbesSrfLps;

/// A PBES explored symbolically into the ingredients of a parity game.
pub struct SymbolicPbes {
    /// The reachable states (BES equations).
    pub vertices: LDDFunction,

    /// The encoded initial state.
    pub initial_vertex: LDDFunction,

    /// Reachable states without an enabled summand.
    pub sinks: LDDFunction,

    /// The symbolic parity game built from `vertices`, `initial_vertex`'s owner/priority partition
    /// and the transition groups learned during exploration.
    pub game: SymbolicParityGame,
}

/// Explores a PBES in SRF normal form into a symbolic parity game, using LDD-based reachability.
///
/// State vectors have layout `[equation_index, param_0, …, param_{n-1}]`. The summand machinery
/// (equation-index gating via `prepare`, condition enumeration, read/write positions) is reused
/// from the explicit [`PbesSrfLps`] through the generic [`SymbolicLps`] adapter, shared with LPS
/// symbolic exploration.
///
/// The `encoding` decides how the equations are distributed over the transition groups and in
/// which order their parameters are stored, mirroring the `--groups` and `--reorder` options of
/// mCRL2's `pbessolvesymbolic`, and `cached` its `--cached` option: every group then remembers the
/// parameter values it has already learned successors for, instead of re-enumerating them.
/// `strategy` selects how the transition groups are applied, as for LPS symbolic exploration.
///
/// Deadlock detection is always requested internally regardless of `strategy`, since the resulting
/// sinks are needed to build the game; there is no `--detect-deadlocks` flag here, unlike
/// `merc-sym`/`merc-lps`.
///
/// Builds the game from the same `SymbolicContext` reachability ran with; the
/// value → equation-index mapping in `context.columns()` is only valid for that one context and
/// must not be recombined with states obtained from a different one.
pub fn explore_pbes_symbolic_game(
    storage: &LDDManagerRef,
    srf_pbes: SrfPbes,
    encoding: &SymbolicLpsOptions,
    strategy: ExplorationStrategy,
    cached: bool,
    compute_strategy: bool,
    timing: &Timing,
) -> Result<SymbolicPbes, MercError> {
    debug_assert!(
        srf_pbes.is_unified(),
        "explore_pbes_symbolic requires a PBES whose equations share one parameter vector; \
         call `SrfPbes::unify_parameters` on it first"
    );
    let lps = PbesSrfLps::new(srf_pbes)?;
    let mut symbolic = SymbolicLps::with_options(storage, lps, encoding)?;

    debug!("{symbolic:?}");

    // `with_options` wraps the LPS in a `PermutedLps` to realise `--reorder`, so the equation
    // index (position 0 of the unpermuted state vector) is not necessarily at level 0.
    let level = symbolic
        .lps()
        .order()
        .iter()
        .position(|&position| position == 0)
        .expect("the equation index (position 0) must appear somewhere in the permutation");

    let options = ReachabilityOptions {
        strategy,
        detect_deadlocks: true,
        cached,
    };
    let mut context = symbolic.create_context();
    let result = reachability_with_options(storage, &mut symbolic, &mut context, &options, timing)?;
    let sinks = result
        .deadlocks
        .expect("detect_deadlocks was requested, so deadlocks must be Some");

    let info = symbolic.lps().inner().equation_info();
    let blocks: Vec<(Value, Player, Priority)> = context.columns()[level]
        .iter()
        .map(|(value, &equation)| (*value as Value, info[equation].player, info[equation].priority))
        .collect();

    let game = SymbolicParityGame::from_block_index(
        storage,
        symbolic.transition_groups(),
        result.states.clone(),
        level,
        &blocks,
        compute_strategy,
    )?;

    Ok(SymbolicPbes {
        vertices: result.states,
        initial_vertex: symbolic.initial_state().clone(),
        sinks,
        game,
    })
}

/// Explores a PBES in SRF normal form using symbolic LDD-based reachability, returning only the
/// reachable states.
///
/// A thin wrapper around [`explore_pbes_symbolic_game`] for callers that only need the state count
/// (`--explore-symbolic`), not the game.
pub fn explore_pbes_symbolic(
    storage: &LDDManagerRef,
    srf_pbes: SrfPbes,
    encoding: &SymbolicLpsOptions,
    strategy: ExplorationStrategy,
    cached: bool,
    timing: &Timing,
) -> Result<LDDFunction, MercError> {
    Ok(explore_pbes_symbolic_game(storage, srf_pbes, encoding, strategy, cached, false, timing)?.vertices)
}
