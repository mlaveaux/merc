#![allow(nonstandard_style)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
//!
//! Implements the standard Zielonka recursive solver for any parity game
//! implementing the [`crate::PG`] trait.

use core::fmt;
use std::ops::BitAnd;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use itertools::Itertools;
use log::debug;
use log::trace;

use crate::PG;
use crate::Player;
use crate::Pred;
use crate::Predecessors;
use crate::Priority;
use crate::Repeat;
use crate::Strat;
use crate::Strategy;
use crate::VertexIndex;

/// The type for a set of vertices.
pub type Set = BitVec<usize, Lsb0>;

/// Solves the given parity game using the Zielonka algorithm.
///
/// If `compute_strategy` is true, also computes the winning strategy for both
/// players. Otherwise, returns `None` strategies.
pub fn solve_zielonka<G: PG>(game: &G, compute_strategy: bool) -> ([Set; 2], Option<[Strategy; 2]>) {
    if compute_strategy {
        let (solution, strategy) = solve_zielonka_impl::<G, Strategy>(game);
        (solution, Some(strategy))
    } else {
        let (solution, _) = solve_zielonka_impl::<G, ()>(game);
        (solution, None)
    }
}

/// Solves the given parity game using the Zielonka algorithm, computing a
/// strategy representation of type `S`.
fn solve_zielonka_impl<G: PG, S: Strat>(game: &G) -> ([Set; 2], [S; 2]) {
    debug_assert!(game.is_total(), "Zielonka solver requires a total parity game");

    // Initial set of vertices V = all vertices
    let mut V = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    V.set_elements(usize::MAX);
    let full_V = V.clone(); // Used for debugging.

    let mut zielonka = ZielonkaSolver::<_, S>::new(game);

    let (W0, S0, W1, S1) = zielonka.zielonka_rec(V, 0);

    // Check that the result is a valid partition
    debug!("Performed {} recursive calls", zielonka.recursive_calls);
    if cfg!(debug_assertions) {
        check_partition(&W0, &W1, &full_V);
    }

    ([W0, W1], [S0, S1])
}

struct ZielonkaSolver<'a, G: PG, S: Strat> {
    game: &'a G,

    /// Reused temporary queue for attractor computation.
    temp_queue: Vec<VertexIndex>,

    /// Reused per-vertex counter, used during attractor computation to count the
    /// number of successors of an opponent vertex that are still within the
    /// subgame but not yet in the attractor set.
    attractor_counters: Vec<usize>,

    /// Stores the predecessors of the game.
    predecessors: Predecessors<'a>,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// Keeps track of the total number of recursive calls.
    recursive_calls: usize,

    /// The `S` is not actually stored in the struct, but we need to keep track of the type for the recursive calls.
    _strategy: std::marker::PhantomData<S>,
}

impl<G: PG, S: Strat> ZielonkaSolver<'_, G, S> {
    /// Creates a new Zielonka solver for the given parity game.
    fn new<'a>(game: &'a G) -> ZielonkaSolver<'a, G, S> {
        // Keep track of the vertices for each priority
        let mut priority_vertices = Vec::new();

        for v in game.iter_vertices() {
            let prio = game.priority(v);

            while prio >= priority_vertices.len() {
                priority_vertices.push(Vec::new());
            }

            priority_vertices[prio].push(v);
        }

        ZielonkaSolver {
            game,
            predecessors: Predecessors::new(game),
            priority_vertices,
            temp_queue: Vec::new(),
            attractor_counters: vec![0; game.num_of_vertices()],
            recursive_calls: 0,
            _strategy: std::marker::PhantomData,
        }
    }

    /// Recursively solves the parity game for the given set of vertices V.
    ///
    /// # Details
    ///
    /// The strategy computation is taken from the following paper:
    ///
    /// >  Oliver Friedmann. Recursive algorithm for parity games requires exponential time. RAIRO Theor. Informatics Appl. 45(4): 449-457 (2011) [DOI](https://doi.org/10.1051/ita/2011124).
    fn zielonka_rec(&mut self, V: Set, depth: usize) -> (Set, S, Set, S) {
        self.recursive_calls += 1;
        #[cfg(debug_assertions)]
        let full_V = V.clone(); // Used for debugging
        let indent = Repeat::new(" ", depth);

        if !V.any() {
            return (V.clone(), S::new(), V.clone(), S::new());
        }

        let highest_prio = self.get_highest_prio(&V);
        let alpha = Player::from_priority(highest_prio);
        let not_alpha = alpha.opponent();

        // Collect the set U of vertices in V whose priority lies in the top block
        // of alpha's parity, i.e. above the highest priority of the opponent's
        // parity occurring in V.
        let mut U = bitvec![usize, Lsb0; 0; self.game.num_of_vertices()];
        for prio in (0..=*highest_prio).rev() {
            if Player::from_priority(Priority::new(prio)) == alpha {
                for &v in &self.priority_vertices[prio] {
                    if V[*v] {
                        U.set(*v, true);
                    }
                }
            } else if self.priority_vertices[prio].iter().any(|v| V[**v]) {
                break;
            }
        }

        debug!(
            "{}|V| = {}, highest prio = {}, lowest prio = {}, player = {}, |U| = {}",
            indent,
            V.count_ones(),
            highest_prio,
            self.get_lowest_prio(&V),
            alpha,
            U.count_ones()
        );
        trace!("{}Vertices in U: {}", indent, DisplaySet(&U));

        let U_clone = U.clone();

        let (A, A_strategy) = self.attractor(alpha, &V, U);

        trace!("{}Vertices in A: {}", indent, DisplaySet(&A));
        debug!("{}zielonka(V \\ A) |A| = {}", indent, A.count_ones());
        let (W1_0, S1_0, W1_1, S1_1) = self.zielonka_rec(V.clone().bitand(!A.clone()), depth + 1);

        let (mut W1_alpha, mut S1_alpha, W1_not_alpha, S1_not_alpha) =
            x_and_not_x_strategy(W1_0, S1_0, W1_1, S1_1, alpha);

        if !W1_not_alpha.any() {
            W1_alpha |= A;
            // Combine the strategy from the attractor with the recursive strategy.
            S1_alpha = S1_alpha
                .union(A_strategy)
                .extend_arbitrary(self.game, &U_clone, &V, alpha);
            combine_with_strategy(W1_alpha, S1_alpha, W1_not_alpha, S::new(), alpha)
        } else {
            let (B, B_strategy) = self.attractor(not_alpha, &V, W1_not_alpha);

            trace!("{}Vertices in B: {}", indent, DisplaySet(&B));
            debug!("{}zielonka(V \\ B)", indent);
            let (W2_0, S2_0, W2_1, S2_1) = self.zielonka_rec(V.bitand(!B.clone()), depth + 1);

            let (W2_alpha, S2_alpha, mut W2_not_alpha, mut S2_not_alpha) =
                x_and_not_x_strategy(W2_0, S2_0, W2_1, S2_1, alpha);

            W2_not_alpha |= B;
            // Combine the strategy from the attractor with the recursive strategy
            S2_not_alpha = S2_not_alpha.union(B_strategy).union(S1_not_alpha);

            #[cfg(debug_assertions)]
            check_partition(&W2_alpha, &W2_not_alpha, &full_V);
            combine_with_strategy(W2_alpha, S2_alpha, W2_not_alpha, S2_not_alpha, alpha)
        }
    }

    /// Computes the attractor for `alpha` to the set `U` within the vertices `V`.
    ///
    /// # Details
    ///
    /// Instead of rescanning the outgoing edges of an opponent vertex every time
    /// one of its successors is added to the attractor, a per-vertex counter of
    /// the successors still outside the attractor is maintained. The opponent
    /// vertex is attracted once that counter reaches zero, giving an overall
    /// linear-time attractor.
    fn attractor(&mut self, alpha: Player, V: &Set, mut A: Set) -> (Set, S) {
        // 1. strategy := empty
        let mut strategy = S::new();

        self.initialize_attractor_counters(alpha, V);

        // 2. Q = {v \in A}
        self.temp_queue.clear();
        for v in A.iter_ones() {
            self.temp_queue.push(VertexIndex::new(v));
        }

        // 4. While Q is not empty do
        // 5. w := Q.pop()
        while let Some(w) = self.temp_queue.pop() {
            // For every u \in Ew do
            for v in self.predecessors.predecessors(w) {
                if !V[*v] || A[*v] {
                    continue;
                }

                let attracted = if self.game.owner(v) == alpha {
                    // v \in V_\alpha can move to w \in A, so it is attracted.
                    true
                } else {
                    // One more successor of v (namely w) entered the attractor;
                    // v is attracted once all of its successors within V did.
                    self.attractor_counters[*v] -= 1;
                    self.attractor_counters[*v] == 0
                };

                if attracted {
                    if self.game.owner(v) == alpha {
                        strategy.set(v, w);
                    }

                    A.set(*v, true);
                    self.temp_queue.push(v);
                }
            }
        }

        (A, strategy)
    }

    /// Initialise the counter of every opponent vertex in `V` to the number of
    /// its successors inside `V`.
    fn initialize_attractor_counters(&mut self, alpha: Player, V: &Set) {
        for v in V.iter_ones().map(VertexIndex::new) {
            if self.game.owner(v) != alpha {
                self.attractor_counters[*v] = self.game.outgoing_edges(v).filter(|edge| V[*edge.to()]).count();
            }
        }
    }

    /// Returns the highest priority occurring in the given set of vertices V.
    fn get_highest_prio(&self, V: &Set) -> Priority {
        let mut highest = usize::MIN;
        for v in V.iter_ones() {
            highest = highest.max(*self.game.priority(VertexIndex::new(v)));
        }
        Priority::new(highest)
    }

    /// Returns the lowest priority occurring in the given set of vertices V.
    ///
    /// Only used for debug logging, so it is kept out of the hot path.
    fn get_lowest_prio(&self, V: &Set) -> Priority {
        let mut lowest = usize::MAX;
        for v in V.iter_ones() {
            lowest = lowest.min(*self.game.priority(VertexIndex::new(v)));
        }
        Priority::new(lowest)
    }
}

/// Checks that the given solutions are a valid partition of the vertices in V
pub fn check_partition(W0: &Set, W1: &Set, V: &Set) {
    let intersection = W0.clone() & W1;
    if intersection.any() {
        panic!(
            "The winning sets are not disjoint. Vertices in both sets: {}",
            intersection
        );
    }

    let both = W0.clone() | W1;
    if both != *V {
        let missing = V.clone() & !both;
        panic!(
            "The winning sets do not cover all vertices. Missing vertices: {}",
            missing
        );
    }
}

/// Returns the given pair ordered by player, left is alpha and right is not_alpha.
pub fn x_and_not_x<U>(omega_0: U, omega_1: U, player: Player) -> (U, U) {
    match player {
        Player::Even => (omega_0, omega_1),
        Player::Odd => (omega_1, omega_0),
    }
}

/// Combines a pair of submaps ordered by player into a pair even, odd.
pub fn combine<U>(omega_x: U, omega_not_x: U, player: Player) -> (U, U) {
    match player {
        Player::Even => (omega_x, omega_not_x),
        Player::Odd => (omega_not_x, omega_x),
    }
}

/// Returns the given pair ordered by player, left is alpha and right is not_alpha.
pub fn x_and_not_x_strategy<U, V>(
    omega_0: U,
    strategy_0: V,
    omega_1: U,
    strategy_1: V,
    player: Player,
) -> (U, V, U, V) {
    match player {
        Player::Even => (omega_0, strategy_0, omega_1, strategy_1),
        Player::Odd => (omega_1, strategy_1, omega_0, strategy_0),
    }
}

/// Combines a pair of submaps ordered by player into a pair even, odd.
pub fn combine_with_strategy<U, V>(
    omega_x: U,
    strategy_x: V,
    omega_not_x: U,
    strategy_not_x: V,
    player: Player,
) -> (U, V, U, V) {
    match player {
        Player::Even => (omega_x, strategy_x, omega_not_x, strategy_not_x),
        Player::Odd => (omega_not_x, strategy_not_x, omega_x, strategy_x),
    }
}

/// Helper struct to display a set of vertices.
struct DisplaySet<'a>(&'a Set);

impl fmt::Display for DisplaySet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}}}", self.0.iter_ones().format(", "))
    }
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_utilities::random_test;

    use crate::random_parity_game;
    use crate::verify_solution;
    use crate::write_pg;

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow for this test.
    fn test_random_zielonka_solver() {
        random_test(100, |rng| {
            let files = DumpFiles::new("test_random_zielonka_solver");
            let game = random_parity_game(rng, true, 100, 5, 3);

            files.dump("input.pg", |writer| write_pg(writer, &game)).unwrap();

            let (solution, strategy) = super::solve_zielonka(&game, true);

            verify_solution(&game, &solution, &strategy.unwrap());
        });
    }
}
