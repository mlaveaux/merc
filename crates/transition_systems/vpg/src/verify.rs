use bitvec::bitvec;
use bitvec::order::Lsb0;
use delegate::delegate;
use log::trace;

use merc_collections::Graph;
use merc_collections::scc_decomposition_iterative;

use crate::Edge;
use crate::PG;
use crate::Player;
use crate::Priority;
use crate::Set;
use crate::Strategy;
use crate::VertexIndex;
use crate::check_partition;
use crate::solve_solitaire_game;

/// Verifies that a proposed solution is valid for the given parity game and
/// strategies.
///
/// # Details
///
/// This is done by restricting the `pg` according to the strategy for each
/// player, and computing the solution for the induced solitaire game.
pub fn verify_solution<G: PG>(pg: &G, solution: &[Set; 2], strategy: &[Strategy; 2]) {
    debug_assert!(pg.is_total(), "Verifying requires a total parity game");

    trace!("Solution for even: {}", solution[Player::Even.to_index()]);
    trace!("Solution for odd: {}", solution[Player::Odd.to_index()]);

    trace!("Strategy for even: {:?}", strategy[Player::Even.to_index()]);
    trace!("Strategy for odd: {:?}", strategy[Player::Odd.to_index()]);

    // The set of all vertices in the game.
    let vertices = bitvec![usize, Lsb0; 1; pg.num_of_vertices()];

    // Check that the input solution is a proper partitioning
    check_partition(&solution[0], &solution[1], &vertices);

    // We check both players' strategies
    for player in [Player::Even, Player::Odd] {
        // Check that the strategies are consistent with the ownership of the vertices
        strategy[player.to_index()].check_consistent(pg, &solution[player.to_index()], player);

        // Restricted game according to the strategy for the current player
        let restricted = Restricted::new(pg, player, &strategy[player.to_index()]);

        // The opponent is the solitaire player, so invert the solution.
        // Furthermore, only consider the vertices that are in the solution for
        // the current player.
        let expected = !solve_solitaire_game(&restricted) & solution[player.to_index()].clone();
        if expected != solution[player.to_index()] {
            panic!(
                "The strategy for player {} is incorrect, expected solution {}, got {}",
                player,
                expected,
                solution[player.to_index()]
            );
        }
    }
}

/// Checks a proposed solution, consisting of winning regions `solution` and
/// positional strategies `strategy` for both players, by SCC computations
/// only. Returns a description of the first defect found.
///
/// # Details
///
/// A solution is accepted iff
///  1. `solution[0]` and `solution[1]` partition the vertices;
///  2. for each player `i`, the region `W_i` is closed: every vertex of `W_i`
///     owned by `i` has a strategy move along an edge of the game that stays in
///     `W_i`, and every successor of a vertex of `W_i` owned by the opponent
///     lies in `W_i`;
///  3. for each player `i`, in the one-player graph `G[W_i]` where `i`'s moves
///     are fixed by `strategy[i]`, every cycle has a maximum priority of `i`'s
///     parity.
///
/// Conditions 2 and 3 say that `strategy[i]` wins every play that starts in
/// `W_i`, so the checked solution is a certificate by itself: no reference
/// solver is needed (the regions are disjoint winning regions that cover all
/// vertices, so they are exactly the winning regions).
///
/// Condition 3 is checked by recursive SCC peeling, one round at a time:
/// compute the SCCs of the alive vertices (initially `W_i`), drop trivial SCCs
/// (one vertex without self-loop); a nontrivial SCC whose maximum priority has
/// the opponent's parity contains a cycle through that vertex, so the solution
/// is rejected; otherwise the vertices with the SCC's maximum priority are
/// removed (every cycle through them is won by `i`) and the next round
/// continues on the rest. The SCCs of an induced subgraph refine the SCCs of
/// the graph, so a round processes all remaining SCCs in a single call of
/// [`scc_decomposition_iterative`]. The maximum priority of every surviving SCC
/// strictly decreases per round, so there are at most `d + 1` rounds for `d`
/// distinct priorities, and the total time is `O(d · (n + m))` (each round is
/// `O(n + m)`); in practice the number of rounds is small.
pub fn check_solution<G: PG>(pg: &G, solution: &[Set; 2], strategy: &[Strategy; 2]) -> Result<(), String> {
    let n = pg.num_of_vertices();
    if solution[0].len() != n || solution[1].len() != n {
        return Err(format!("The winning regions do not have {n} entries"));
    }

    // 1. The regions partition V.
    if let Some(v) = (solution[0].clone() & &solution[1]).first_one() {
        return Err(format!("Vertex {v} is in both winning regions"));
    }
    if let Some(v) = (solution[0].clone() | &solution[1]).first_zero() {
        return Err(format!("Vertex {v} is in neither winning region"));
    }

    for player in [Player::Even, Player::Odd] {
        let region = &solution[player.to_index()];
        let player_strategy = &strategy[player.to_index()];

        for (&from, _) in player_strategy.iter() {
            if *from >= n || pg.owner(from) != player {
                return Err(format!(
                    "Strategy of {player} is defined for vertex {from} it does not own"
                ));
            }
        }

        // 2. Closedness of W_i, and the strategy as a successor array.
        let mut successor = vec![usize::MAX; n];
        for v in region.iter_ones() {
            let vertex = VertexIndex::new(v);
            if pg.owner(vertex) == player {
                let Some(&to) = player_strategy.get(vertex) else {
                    return Err(format!(
                        "Strategy of {player} is undefined for vertex {v} in its region"
                    ));
                };
                if *to >= n || !pg.outgoing_edges(vertex).any(|edge| edge.to() == to) {
                    return Err(format!("Strategy of {player} moves {v} -> {to}, which is not an edge"));
                }
                if !region[*to] {
                    return Err(format!("Strategy of {player} moves {v} -> {to} out of its region"));
                }
                successor[v] = *to;
            } else if let Some(edge) = pg.outgoing_edges(vertex).find(|edge| !region[*edge.to()]) {
                return Err(format!(
                    "{} can escape from the region of {player} by {v} -> {}",
                    player.opponent(),
                    edge.to()
                ));
            }
        }

        // 3. Recursive SCC peeling of the one-player graph G[W_i] with σ_i fixed.
        let mut alive = region.clone();
        let mut rounds = 0;
        while alive.any() {
            rounds += 1;
            let graph = OnePlayerGraph {
                game: pg,
                player,
                successor: &successor,
                alive: &alive,
            };
            let sccs = scc_decomposition_iterative(&graph, |_, _, _| true);

            let k = sccs.num_of_blocks();
            let mut size = vec![0usize; k];
            let mut self_loop = vec![false; k];
            let mut max_priority = vec![0usize; k];
            let mut witness = vec![usize::MAX; k];
            for v in alive.iter_ones() {
                let block = sccs.block(v).value();
                size[block] += 1;
                let priority = pg.priority(VertexIndex::new(v)).value();
                if witness[block] == usize::MAX || priority > max_priority[block] {
                    max_priority[block] = priority;
                    witness[block] = v;
                }
                if !self_loop[block] && graph.outgoing_edges(VertexIndex::new(v)).any(|(_, to)| *to == v) {
                    self_loop[block] = true;
                }
            }

            let mut next = bitvec![usize, Lsb0; 0; n];
            for v in alive.iter_ones() {
                let block = sccs.block(v).value();
                if size[block] == 1 && !self_loop[block] {
                    continue;
                }
                if Player::from_priority(Priority::new(max_priority[block])) != player {
                    return Err(format!(
                        "Strategy of {player} admits a cycle through vertex {} with maximum priority {} in its region",
                        witness[block], max_priority[block]
                    ));
                }
                if pg.priority(VertexIndex::new(v)).value() < max_priority[block] {
                    next.set(v, true);
                }
            }
            alive = next;
        }
        trace!("Checked the region of {player} in {rounds} SCC rounds");
    }

    Ok(())
}

/// The one-player graph `G[alive]` in which the moves of `player` are fixed by
/// `successor`, used by [`check_solution`].
struct OnePlayerGraph<'a, G: PG> {
    game: &'a G,
    player: Player,
    successor: &'a [usize],
    alive: &'a Set,
}

impl<G: PG> Graph for OnePlayerGraph<'_, G> {
    type VertexIndex = VertexIndex;
    type LabelIndex = ();

    fn num_of_vertices(&self) -> usize {
        self.game.num_of_vertices()
    }

    fn iter_vertices(&self) -> impl Iterator<Item = Self::VertexIndex> {
        self.alive.iter_ones().map(VertexIndex::new)
    }

    fn outgoing_edges(&self, vertex: VertexIndex) -> impl Iterator<Item = ((), VertexIndex)> {
        let fixed = (self.game.owner(vertex) == self.player).then(|| self.successor[*vertex]);
        self.game
            .outgoing_edges(vertex)
            .map(|edge| edge.to())
            .filter(move |to| fixed.is_none_or(|s| s == **to) && self.alive[**to])
            .map(|to| ((), to))
    }
}

/// A subgame of a parity game that is induced by taking the strategy for the
/// given player into account.
struct Restricted<'a, G: PG> {
    /// The game that is being restricted.
    game: &'a G,

    /// The strategy that is applied to the game.
    strategy: &'a Strategy,

    /// The player that owns the strategy.
    player: Player,
}

impl<G: PG> Restricted<'_, G> {
    /// Create a new sub-game induced by the given strategy on the given game.
    pub(crate) fn new<'a>(game: &'a G, player: Player, strategy: &'a Strategy) -> Restricted<'a, G> {
        Restricted { game, player, strategy }
    }
}

impl<G: PG> PG for Restricted<'_, G> {
    type Label = G::Label;

    fn owner(&self, _vertex_index: VertexIndex) -> Player {
        // We know that player only makes moves according to the strategy, so we can treat all vertices in the game as if they were owned by the opponent.
        self.player.opponent()
    }

    fn outgoing_edges<'a>(&'a self, vertex_index: VertexIndex) -> impl Iterator<Item = Edge<'a, G::Label>> + 'a {
        self.game.outgoing_edges(vertex_index).filter(move |edge| {
            // Only consider edges that follow the strategy.
            self.game.owner(vertex_index) != self.player || self.strategy.get(vertex_index) == Some(&edge.to())
        })
    }

    fn is_total(&self) -> bool {
        // The sub-game is total if every vertex in the restricted set has at least one outgoing edge.
        self.iter_vertices().all(|v| self.outgoing_edges(v).next().is_some())
    }

    delegate! {
        to self.game {
            fn initial_vertex(&self) -> VertexIndex;
            fn num_of_vertices(&self) -> usize;
            fn num_of_edges(&self) -> usize;
            fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_;
            fn priority(&self, vertex: VertexIndex) -> Priority;
            fn highest_priority(&self) -> Priority;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::panic::catch_unwind;

    use merc_utilities::random_test;
    use rand::RngExt;

    use crate::PG;
    use crate::Player;
    use crate::StrategyImprovementAlgorithm;
    use crate::VertexIndex;
    use crate::random_parity_game;
    use crate::solve_strategy_improvement_default;
    use crate::solve_zielonka;

    use super::check_solution;
    use super::verify_solution;

    /// Correct solutions (Zielonka, H-B) are accepted, and the solitaire-based
    /// checker gives the same verdict as the SCC checker on randomly corrupted
    /// solutions.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_check_solution() {
        let mut rejected = 0;
        random_test(300, |rng| {
            let game = random_parity_game(rng, true, 40, 6, 3);
            let (expected, strategy) = solve_zielonka(&game, true);
            check_solution(&game, &expected, &strategy.unwrap()).unwrap();

            let (solution, strategy) =
                solve_strategy_improvement_default(&game, StrategyImprovementAlgorithm::HybridB, true);
            let mut strategy = strategy.unwrap();
            assert_eq!(solution, expected);
            check_solution(&game, &solution, &strategy).unwrap();

            // Corrupt one strategy move, or move one vertex to the other region.
            let mut solution = solution;
            let v = VertexIndex::new(rng.random_range(0..game.num_of_vertices()));
            if rng.random_bool(0.7) {
                let player = game.owner(v);
                let successors: Vec<_> = game.outgoing_edges(v).map(|edge| edge.to()).collect();
                strategy[player.to_index()].set(v, successors[rng.random_range(0..successors.len())]);
            } else {
                let won_by_even = solution[0][*v];
                solution[0].set(*v, !won_by_even);
                solution[1].set(*v, won_by_even);
            }

            let new = check_solution(&game, &solution, &strategy);
            let old = catch_unwind(AssertUnwindSafe(|| verify_solution(&game, &solution, &strategy)));
            assert_eq!(new.is_ok(), old.is_ok(), "verdicts differ: {new:?}");
            if solution != expected {
                assert!(new.is_err(), "wrong winning regions were accepted");
            }
            if new.is_err() {
                rejected += 1;
            }
        });
        assert!(rejected > 0, "no corruption was ever detected");
    }

    /// A strategy that closes a cycle with a maximum priority of the wrong parity is rejected.
    #[test]
    fn test_check_solution_bad_cycle() {
        let mut builder = crate::ParityGameBuilder::new(VertexIndex::new(0));
        builder.add_vertex(VertexIndex::new(0), Player::Even, crate::Priority::new(1));
        builder.add_vertex(VertexIndex::new(1), Player::Even, crate::Priority::new(2));
        builder.add_edge(VertexIndex::new(0), VertexIndex::new(0));
        builder.add_edge(VertexIndex::new(0), VertexIndex::new(1));
        builder.add_edge(VertexIndex::new(1), VertexIndex::new(0));
        let game = builder.finish(false, false);

        let all = bitvec::bitvec![usize, bitvec::order::Lsb0; 1; 2];
        let none = bitvec::bitvec![usize, bitvec::order::Lsb0; 0; 2];
        let mut good = crate::Strategy::new();
        good.set(VertexIndex::new(0), VertexIndex::new(1));
        good.set(VertexIndex::new(1), VertexIndex::new(0));
        let solution = [all, none];
        check_solution(&game, &solution, &[good, crate::Strategy::new()]).unwrap();

        let mut bad = crate::Strategy::new();
        bad.set(VertexIndex::new(0), VertexIndex::new(0));
        bad.set(VertexIndex::new(1), VertexIndex::new(0));
        assert!(check_solution(&game, &solution, &[bad, crate::Strategy::new()]).is_err());
    }
}
