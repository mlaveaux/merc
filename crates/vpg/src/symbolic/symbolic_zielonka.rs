use std::ops::ControlFlow;

use log::debug;
use log::info;
use log::trace;

use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;

use merc_io::TimeProgress;
use merc_symbolic::SatCount;
use merc_symbolic::merge;
use merc_utilities::MercError;

use crate::Player;
use crate::Repeat;
use crate::SymbolicParityGame;
use crate::symbolic::AttractorProgress;
use crate::symbolic::symbolic_parity_game::num_vertices;

/// Progress reported by `zielonka`'s recursion.
pub type RecursionProgress = TimeProgress<(usize, SatCount)>;

/// The winning sets of both players in a symbolic parity game, indexed by [`Player::to_index`].
///
/// `strategy[Player::to_index()]` is `None` unless the [`SymbolicParityGame`] this solution was
/// computed for was built with `compute_strategy: true`.
pub struct SymbolicSolution {
    pub winning: [LDDFunction; 2],
    pub strategy: [Option<LDDFunction>; 2],
}

impl SymbolicSolution {
    /// Returns the winner of `vertex`, or `None` if `vertex` was not part of the solved game.
    pub fn winner(&self, vertex: &LDDFunction) -> Option<Player> {
        if includes(&self.winning[Player::Even.to_index()], vertex).expect("LDD operation should not run out of memory")
        {
            Some(Player::Even)
        } else if includes(&self.winning[Player::Odd.to_index()], vertex)
            .expect("LDD operation should not run out of memory")
        {
            Some(Player::Odd)
        } else {
            None
        }
    }
}

/// Returns whether `vertex` is a subset of `winning` (mCRL2's Sylvan-native `includes`, which
/// oxidd has no equivalent for).
pub(crate) fn includes(winning: &LDDFunction, vertex: &LDDFunction) -> Result<bool, MercError> {
    Ok(vertex.minus(winning)?.is_empty())
}

/// An extended parity game, which includes the initial vertex and the deadlock
/// vertices (sinks).
pub struct ExtendedParityGame {
    /// The symbolic parity game to solve.
    pub game: SymbolicParityGame,
    /// The initial vertex whose winner is sought.
    pub initial_vertex: LDDFunction,
    /// The deadlock vertices to resolve.
    pub sinks: LDDFunction,
}

impl ExtendedParityGame {
    pub fn new(game: SymbolicParityGame, initial_vertex: LDDFunction, sinks: LDDFunction) -> Self {
        Self {
            game,
            initial_vertex,
            sinks,
        }
    }
}

/// Computes the total graph of `epg.game` with respect to `epg.sinks` and the
/// current winning sets, and returns early with the accumulated
/// [`SymbolicSolution`] if `epg.initial_vertex` is already resolved.
pub(crate) fn total_graph_with_early_exit(
    epg: &ExtendedParityGame,
    incomplete: &LDDFunction,
    winning: &mut [LDDFunction; 2],
    strategy: &mut [Option<LDDFunction>; 2],
    attractor_progress: &AttractorProgress,
) -> Result<ControlFlow<SymbolicSolution, LDDFunction>, MercError> {
    let total = epg.game.compute_total_graph(
        epg.game.vertices(),
        &epg.sinks,
        winning,
        strategy,
        Some(incomplete),
        attractor_progress,
    )?;

    if includes(&winning[0], &epg.initial_vertex)? || includes(&winning[1], &epg.initial_vertex)? {
        return Ok(ControlFlow::Break(SymbolicSolution {
            winning: winning.clone(),
            strategy: strategy.clone(),
        }));
    }

    Ok(ControlFlow::Continue(total))
}

/// Solves `epg.game` restricted to `epg.game.vertices()`, with `epg.sinks` as the deadlocks to
/// resolve, and returns the winner of `epg.initial_vertex` together with the winning partition.
///
/// When `allow_early_termination` is true, the solver will terminate as soon as
/// `epg.initial_vertex` is resolved.
pub fn solve_symbolic_zielonka(
    epg: &ExtendedParityGame,
    allow_early_termination: bool,
) -> Result<(Player, SymbolicSolution), MercError> {
    let game = &epg.game;
    let empty = game.manager().with_manager_shared(LDDFunction::empty_set)?;
    let mut winning = [empty.clone(), empty];
    let mut strategy = empty_strategy_pair(game)?;
    let attractor_progress = new_attractor_progress();

    let total = game.compute_total_graph(
        game.vertices(),
        &epg.sinks,
        &mut winning,
        &mut strategy,
        None,
        &attractor_progress,
    )?;

    let already_resolved = allow_early_termination
        && (includes(&winning[0], &epg.initial_vertex)? || includes(&winning[1], &epg.initial_vertex)?);

    if !already_resolved {
        let recursion_progress = new_recursion_progress();
        let solution = zielonka(game, &total, &recursion_progress, &attractor_progress)?;
        winning[0] = winning[0].union(&solution.winning[0])?;
        winning[1] = winning[1].union(&solution.winning[1])?;
        union_strategy_pair_in_place(&mut strategy, solution.strategy)?;
    }

    if includes(&winning[0], &epg.initial_vertex)? {
        Ok((Player::Even, SymbolicSolution { winning, strategy }))
    } else if includes(&winning[1], &epg.initial_vertex)? {
        Ok((Player::Odd, SymbolicSolution { winning, strategy }))
    } else {
        Err("solve_symbolic_zielonka: initial vertex was not resolved by the solver".into())
    }
}

/// Builds the throttled progress tracker [`SymbolicParityGame::attractor`] reports through:
/// iteration number and attractor size so far, logged at most once every 5 seconds.
fn new_attractor_progress() -> AttractorProgress {
    TimeProgress::new(
        |(iteration, size)| {
            info!("attractor: iteration {iteration}, {size} vertices so far");
        },
        5,
    )
}

/// Builds the throttled progress tracker [`zielonka`]'s recursion reports through: depth and
/// `|V|` at that call, logged at most once every 5 seconds.
fn new_recursion_progress() -> RecursionProgress {
    TimeProgress::new(
        |(depth, size)| {
            info!("zielonka: recursion depth {depth}, {size} vertices remaining");
        },
        5,
    )
}

/// `[None, None]` when `game` was not built with `compute_strategy`, otherwise `[Some(∅),
/// Some(∅)]` — the accumulator [`SymbolicParityGame::compute_total_graph`] grows in place.
pub(crate) fn empty_strategy_pair(game: &SymbolicParityGame) -> Result<[Option<LDDFunction>; 2], MercError> {
    if game.compute_strategy() {
        let empty = game.manager().with_manager_shared(LDDFunction::empty_set)?;
        Ok([Some(empty.clone()), Some(empty)])
    } else {
        Ok([None, None])
    }
}

/// Unions `addition` (a `zielonka` result's strategy) into `strategy` in place, player by player;
/// a no-op on whichever players are not being tracked (i.e. `compute_strategy` is not set).
fn union_strategy_pair_in_place(
    strategy: &mut [Option<LDDFunction>; 2],
    addition: [Option<LDDFunction>; 2],
) -> Result<(), MercError> {
    for (existing, addition) in strategy.iter_mut().zip(addition) {
        if let (Some(existing), Some(addition)) = (existing.as_mut(), addition) {
            *existing = existing.union(&addition)?;
        }
    }
    Ok(())
}

/// The recursive Zielonka solver, restricted to the vertex set `v`. Entry point for
/// [`zielonka_rec`], which does the actual recursion; starts it at `depth: 0`.
///
/// `v` must be deadlock-free (every vertex has an outgoing edge staying within `v`) — callers
/// route it through [`SymbolicParityGame::compute_total_graph`] first to guarantee this. On
/// return, `winning[0]` and `winning[1]` partition `v` exactly.
pub(crate) fn zielonka(
    game: &SymbolicParityGame,
    v: &LDDFunction,
    recursion_progress: &RecursionProgress,
    attractor_progress: &AttractorProgress,
) -> Result<SymbolicSolution, MercError> {
    recursion_progress.reset();
    zielonka_rec(game, v, 0, recursion_progress, attractor_progress)
}

/// The recursive worker behind [`zielonka`]; see there for the entry point.
fn zielonka_rec(
    game: &SymbolicParityGame,
    v: &LDDFunction,
    depth: usize,
    recursion_progress: &RecursionProgress,
    attractor_progress: &AttractorProgress,
) -> Result<SymbolicSolution, MercError> {
    let indent = Repeat::new(" ", depth);

    if v.is_empty() {
        let empty = empty_set(game)?;
        let strategy = empty_strategy_pair(game)?;
        return Ok(SymbolicSolution {
            winning: [empty.clone(), empty],
            strategy,
        });
    }

    recursion_progress.print((depth, num_vertices(v)));

    let vplayer = game.players(v)?;
    let (priority, u) = game
        .max_priority(v)?
        .expect("v is non-empty, so it must contain at least one priority");
    let alpha = Player::from_priority(priority);
    let not_alpha = alpha.opponent();

    debug!(
        "{indent}zielonka: |V| = {}, priority = {priority}, player = {alpha}",
        num_vertices(v)
    );
    trace!(
        "{indent}U (highest priority vertices) has {} elements",
        num_vertices(&u)
    );

    let (a, a_strategy) = game.attractor(alpha, &u, v, &vplayer, None, None, attractor_progress)?;
    trace!("{indent}A (attractor of U) has {} elements", num_vertices(&a));

    let v_minus_a = v.minus(&a)?;
    let solution_v_minus_a = zielonka_rec(game, &v_minus_a, depth + 1, recursion_progress, attractor_progress)?;

    let solution = if solution_v_minus_a.winning[not_alpha.to_index()].is_empty() {
        let mut winning = [empty_set(game)?, empty_set(game)?];
        winning[alpha.to_index()] = a.union(&solution_v_minus_a.winning[alpha.to_index()])?;

        let mut strategy = empty_strategy_pair(game)?;
        if game.compute_strategy() {
            // `a_strategy` has no move for U itself (the attractor's target, not something pulled
            // in). Seed it with `merge(U ∩ Vplayer[alpha], v)`, later cut down to real edges by
            // `apply_strategy`.
            let u_alpha = u.intersect(&vplayer[alpha.to_index()])?;
            let extra = merge(game.manager(), &u_alpha, v)?;
            let combined = a_strategy
                .expect("compute_strategy is set")
                .union(
                    solution_v_minus_a.strategy[alpha.to_index()]
                        .as_ref()
                        .expect("compute_strategy is set"),
                )?
                .union(&extra)?;

            strategy[alpha.to_index()] = Some(combined);
        }

        SymbolicSolution { winning, strategy }
    } else {
        let (b, b_strategy) = game.attractor(
            not_alpha,
            &solution_v_minus_a.winning[not_alpha.to_index()],
            v,
            &vplayer,
            None,
            None,
            attractor_progress,
        )?;
        trace!(
            "{indent}B (attractor of the opponent's win in V \\ A) has {} elements",
            num_vertices(&b)
        );

        let v_minus_b = v.minus(&b)?;
        let mut solution = zielonka_rec(game, &v_minus_b, depth + 1, recursion_progress, attractor_progress)?;
        solution.winning[not_alpha.to_index()] = solution.winning[not_alpha.to_index()].union(&b)?;

        if game.compute_strategy() {
            let i = not_alpha.to_index();
            let combined = solution_v_minus_a.strategy[i]
                .as_ref()
                .expect("compute_strategy is set")
                .union(b_strategy.as_ref().expect("compute_strategy is set"))?
                .union(solution.strategy[i].as_ref().expect("compute_strategy is set"))?;
            solution.strategy[i] = Some(combined);
        }

        solution
    };

    #[cfg(debug_assertions)]
    {
        let partition = solution.winning[0].union(&solution.winning[1])?;
        debug_assert!(partition == *v, "zielonka: winning sets must partition V");
    }

    Ok(solution)
}

/// Returns an empty set in the same manager as `game`.
pub(crate) fn empty_set(game: &SymbolicParityGame) -> Result<LDDFunction, MercError> {
    Ok(game.manager().with_manager_shared(LDDFunction::empty_set)?)
}

#[cfg(test)]
mod tests {
    use merc_symbolic::LDD_CACHE_CAPACITY;
    use merc_symbolic::LDD_NODE_CAPACITY;
    use rand::RngExt;

    use oxidd::ManagerRef;
    use oxidd::ldd::LDDFunction;

    use merc_io::DumpFiles;
    use merc_utilities::random_test;

    use crate::PG;
    use crate::Player;
    use crate::encode_parity_game;
    use crate::random_parity_game;
    use crate::solve_zielonka;
    use crate::write_pg;

    use super::ExtendedParityGame;
    use super::solve_symbolic_zielonka;
    use crate::symbolic::verify_symbolic::make_parity_game_total;

    /// Cross-checks [`solve_symbolic_zielonka`] against the explicit [`solve_zielonka`] on random
    /// total games.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_symbolic_zielonka_solver() {
        random_test(100, |rng| {
            let files = DumpFiles::new("test_random_symbolic_zielonka_solver");
            let game = random_parity_game(rng, true, 20, 5, 3);
            files.dump("input.pg", |writer| write_pg(writer, &game)).unwrap();

            let (expected, _) = solve_zielonka(&game, false);

            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
            let radix = rng.random_range(2..=5);
            let num_groups = rng.random_range(1..=3);
            let (symbolic, all_vertices, cubes) =
                encode_parity_game(&manager, &game, rng, radix, num_groups, false).unwrap();

            let empty_sinks = symbolic.sinks(&all_vertices, &all_vertices).unwrap();
            assert!(empty_sinks.is_empty(), "a total game must have no sinks");

            let initial = manager
                .with_manager_shared(|m| LDDFunction::singleton(m, &cubes[0]))
                .unwrap();
            let epg = ExtendedParityGame::new(symbolic, initial, empty_sinks);
            let (_, solution) = solve_symbolic_zielonka(&epg, true).unwrap();

            for v in game.iter_vertices() {
                let vertex = manager
                    .with_manager_shared(|m| LDDFunction::singleton(m, &cubes[*v]))
                    .unwrap();
                let symbolic_winner = solution.winner(&vertex).expect("every vertex should be solved");
                let expected_winner = if expected[Player::Even.to_index()][*v] {
                    Player::Even
                } else {
                    Player::Odd
                };
                assert_eq!(symbolic_winner, expected_winner, "vertex {v}");
            }
        });
    }

    /// Cross-checks the sink handling of [`crate::SymbolicParityGame::compute_total_graph`]
    /// against an explicit oracle: a deadlock is resolved by giving it a self-loop whose priority
    /// has the parity of the *opponent* of its owner (so the self-loop's unique play is won by
    /// that opponent), matching `compute_total_graph`'s `winning[Even] |= sinks ∩ Odd-owned`.
    ///
    /// Also checks that early termination doesn't change the initial vertex's winner: since
    /// `solve_symbolic_zielonka(..., false)` fully solves the game (needed to cross-check every
    /// vertex against the oracle below, not just the initial one),
    /// `solve_symbolic_zielonka(..., true)` on the same game must agree with it there.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_symbolic_zielonka_solver_non_total() {
        random_test(100, |rng| {
            // See the same-sized game in `test_random_symbolic_zielonka_solver` above.
            let game = random_parity_game(rng, false, 20, 5, 3);

            // Build the explicit oracle: same game, but every deadlock gets a self-loop and a
            // priority whose parity is the opponent of its owner (shared with
            // `verify_symbolic_solution`'s decode path, since both need the same convention).
            let expected_game = make_parity_game_total(&game);
            let (expected, _) = solve_zielonka(&expected_game, false);

            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
            let radix = rng.random_range(2..=5);
            let num_groups = rng.random_range(1..=3);
            let (symbolic, all_vertices, cubes) =
                encode_parity_game(&manager, &game, rng, radix, num_groups, false).unwrap();

            let sinks = symbolic.sinks(&all_vertices, &all_vertices).unwrap();
            let initial = manager
                .with_manager_shared(|m| LDDFunction::singleton(m, &cubes[0]))
                .unwrap();

            let epg = ExtendedParityGame::new(symbolic, initial, sinks);
            let (_, solution) = solve_symbolic_zielonka(&epg, false).unwrap();

            let (early_winner, _) = solve_symbolic_zielonka(&epg, true).unwrap();
            assert_eq!(Some(early_winner), solution.winner(&epg.initial_vertex));

            for v in game.iter_vertices() {
                let vertex = manager
                    .with_manager_shared(|m| LDDFunction::singleton(m, &cubes[*v]))
                    .unwrap();
                let symbolic_winner = solution.winner(&vertex).expect("every vertex should be solved");
                let expected_winner = if expected[Player::Even.to_index()][*v] {
                    Player::Even
                } else {
                    Player::Odd
                };
                assert_eq!(symbolic_winner, expected_winner, "vertex {v}");
            }
        });
    }
}
