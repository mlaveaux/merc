#![allow(nonstandard_style)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
//!
//! Implements the standard Zielonka recursive solver for any parity game
//! implementing the [`crate::PG`] trait.

use std::ops::BitAnd;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use log::debug;
use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

use crate::PG;
use crate::ParityGame;
use crate::Player;
use crate::Predecessors;
use crate::Priority;
use crate::Repeat;
use crate::VariabilityParityGame;
use crate::VertexIndex;
use crate::compute_reachable;
use crate::project_variability_parity_games_iter;

type Set = BitVec<usize, Lsb0>;

/// Solves the given parity game using the Zielonka algorithm.
pub fn solve_zielonka(game: &ParityGame) -> [Set; 2] {
    debug_assert!(game.is_total(), "Zielonka solver requires a total parity game");

    let mut V = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    V.set_elements(usize::MAX);

    let mut zielonka = ZielonkaSolver::new(game);

    let W = zielonka.solve_recursive(V, 0);

    // Check that the result is a valid partition
    debug_assert!(
        {
            let intersection = W[0].clone() & &W[1];
            if intersection.any() {
                let non_disjoint: Vec<_> = intersection.iter_ones().collect();
                panic!(
                    "The winning sets are not disjoint. Vertices in both sets: {:?}",
                    non_disjoint
                );
            }
            true
        },
        "The winning sets are not disjoint"
    );
    debug_assert!(
        {
            let both = W[0].clone() | &W[1];
            if !both.all() {
                let missing: Vec<_> = both.iter_zeros().take(game.num_of_vertices()).collect();
                panic!(
                    "The winning sets do not cover all vertices. Missing vertices: {:?}",
                    missing
                );
            }
            true
        },
        "The winning sets do not cover all vertices"
    );

    W
}

/// Solves the given variability parity game using the product-based Zielonka algorithm.
pub fn solve_variability_product_zielonka(vpg: &VariabilityParityGame) -> impl Iterator<Item = (Vec<OptBool>, BDDFunction, [Set;2])> {
    project_variability_parity_games_iter(&vpg)
        .map(|result| {
            let (cube, bdd, pg) = result.expect("Projection should not fail");
            let (reachable_pg, projection) = compute_reachable(&pg);

            let pg_solution = solve_zielonka(&reachable_pg);
            let mut new_solution = [bitvec![usize, Lsb0; 0; vpg.num_of_vertices()], bitvec![usize, Lsb0; 0; vpg.num_of_vertices()]];
            for v in pg.iter_vertices() {
                if let Some(proj_v) = projection[*v] {
                    // Vertex is reachable in the projection, set its solution
                    if pg_solution[0][proj_v] {
                        new_solution[0].set(*v, true);
                    }
                    if pg_solution[1][proj_v] {
                        new_solution[1].set(*v, true);
                    }
                }
            }

            (cube, bdd, new_solution)
        })
}

struct ZielonkaSolver<'a> {
    game: &'a ParityGame,

    /// Reused temporary queue for attractor computation.
    temp_queue: Vec<VertexIndex>,

    /// Stores the predecessors of the game.
    predecessors: Predecessors,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// Keeps track of the total number of recursive calls.
    recursive_calls: usize,
}

impl ZielonkaSolver<'_> {
    /// Creates a new Zielonka solver for the given parity game.
    fn new<'a>(game: &'a ParityGame) -> ZielonkaSolver<'a> {
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
            recursive_calls: 0,
        }
    }

    /// Recursively solves the parity game for the given set of vertices V.
    fn solve_recursive(&mut self, mut V: Set, depth: usize) -> [Set; 2] {
        self.recursive_calls += 1;
        let indent = Repeat::new(" ", depth);

        if !V.any() {
            return [V.clone(), V];
        }

        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&V);
        let alpha = Player::from_priority(&highest_prio);
        let not_alpha = alpha.opponent();

        // Collect the set U of vertices with the highest priority in V
        let mut U = bitvec![usize, Lsb0; 0; self.game.num_of_vertices()];
        for &v in self.priority_vertices[highest_prio].iter() {
            if V[*v] {
                U.set(*v, true);
            }
        }

        debug!(
            "{}solve_rec(V) |V| = {}, highest prio = {}, lowest prio = {}, player = {}, |U| = {}",
            indent,
            V.count_ones(),
            highest_prio,
            lowest_prio,
            alpha,
            U.count_ones()
        );

        let A = self.attractor(alpha, &V, U);

        debug!("{}solve_rec(V \\ A) |A| = {}", indent, A.count_ones());
        let mut W_prime = self.solve_recursive(
            V.iter()
                .enumerate()
                .map(|(index, value)| value.bitand(!A[index]))
                .collect(),
            depth + 1,
        );

        if !W_prime[not_alpha.to_index()].any() {
            W_prime[alpha.to_index()] |= A;
            W_prime
        } else {
            // Get ownershop of a single element in the array.
            let W_prime_opponent = std::mem::take(&mut W_prime[not_alpha.to_index()]);
            let B = self.attractor(not_alpha, &V, W_prime_opponent);

            // Computes V \ B in place
            for (index, value) in V.iter_mut().enumerate() {
                let tmp = value.bitand(!B[index]);
                value.commit(tmp);
            }

            debug!("{}solve_rec(V \\ B)", indent);
            let mut W_double_prime = self.solve_recursive(V, depth + 1); // V has been updated to V \ B

            W_double_prime[not_alpha.to_index()] |= B;
            W_double_prime
        }
    }

    /// Computes the attractor for `alpha` to the set `U` within the vertices `V`.
    fn attractor(&mut self, alpha: Player, V: &Set, mut A: Set) -> Set {
        // 2. Q = {v \in A}
        self.temp_queue.clear();
        for v in A.iter_ones() {
            self.temp_queue.push(VertexIndex::new(v));
        }

        let initial_size = A.count_ones();

        // 4. While Q is not empty do
        // 5. w := Q.pop()
        while let Some(w) = self.temp_queue.pop() {
            // For every u \in Ew do
            for v in self.predecessors.predecessors(w) {
                if V[*v] {
                    let attracted = if self.game.owner(v) == alpha {
                        // v \in V and v in V_\alpha
                        true
                    } else {
                        // Check if all successors of v are in the attractor
                        self.game.outgoing_edges(v).all(|w_prime| V[*w_prime] && A[*w_prime])
                    };

                    if attracted && !A[*v] {
                        A.set(*v, true);
                        self.temp_queue.push(v);
                    }
                }
            }
        }

        A
    }

    /// Returns the highest and lowest priority in the given set of vertices V.
    fn get_highest_lowest_prio(&self, V: &Set) -> (Priority, Priority) {
        let mut highest = usize::MIN;
        let mut lowest = usize::MAX;

        for v in V.iter_ones() {
            let prio = self.game.priority(VertexIndex::new(v));
            highest = highest.max(*prio);
            lowest = lowest.min(*prio);
        }

        (Priority::new(highest), Priority::new(lowest))
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::random_parity_game;
    use crate::solve_zielonka;

    #[test]
    #[cfg_attr(miri, ignore)] // Very slow under Miri
    fn test_random_parity_game_solve() {
        random_test(100, |rng| {
            let pg = random_parity_game(rng, true, 100, 5, 3);
            println!("{:?}", pg);

            solve_zielonka(&pg);
        })
    }
}
