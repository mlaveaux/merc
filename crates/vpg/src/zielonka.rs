#![allow(nonstandard_style)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse

use std::ops::BitAnd;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use log::debug;

use crate::ParityGame;
use crate::Player;
use crate::Predecessors;
use crate::Priority;
use crate::VertexIndex;

type Set = BitVec<usize, Lsb0>;

/// Solves the given parity game using the Zielonka algorithm.
pub fn solve_zielonka(game: &ParityGame) -> Player {
    let mut V = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    V.set_elements(usize::MAX);

    let mut zielonka = ZielonkaSolver::new(game);

    let W = zielonka.solve_recursive(V);

    // Check that the result is a valid partition
    debug_assert!(
        W[0].clone().bitand(&W[1]).not_any(),
        "The winning sets are not disjoint"
    );
    debug_assert!(
        (W[0].clone() | W[1].clone()).all(),
        "The winning sets do not cover all vertices"
    );

    if W[0][*game.initial_vertex()] {
        Player::Even
    } else {
        Player::Odd
    }
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
    fn solve_recursive(&mut self, mut V: Set) -> [Set; 2] {
        self.recursive_calls += 1;

        if !V.any() {
            return [V.clone(), V];
        }

        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&V);
        let alpha = Player::from_priority(&highest_prio);

        // Collect the set U of vertices with the highest priority in V
        let mut U = bitvec![usize, Lsb0; 0; self.game.num_of_vertices()];
        for &v in self.priority_vertices[highest_prio].iter() {
            if V[*v] {
                U.set(*v, true);
            }
        }

        debug!(
            "solve_rec(V) |V| = {}, highest prio = {}, lowest prio = {}, player = {}, |U| = {}",
            V.count_ones(),
            highest_prio,
            lowest_prio,
            alpha,
            U.count_ones()
        );

        let A = self.attractor(alpha, &V, U);

        debug!("begin solve_rec(V \\ A)");
        let mut W_prime = self.solve_recursive(
            V.iter()
                .enumerate()
                .map(|(index, value)| value.bitand(!A[index]))
                .collect(),
        );
        debug!("end solve_rec(V \\ A)");

        if !W_prime[alpha.opponent().to_index()].any() {
            W_prime[alpha.to_index()] |= A;
            W_prime
        } else {
            // Get ownershop of a single element in the array.
            let W_prime_opponent = std::mem::take(&mut W_prime[alpha.opponent().to_index()]);
            let B = self.attractor(alpha.opponent(), &V, W_prime_opponent);

            // Computes V \ B in place
            for (index, value) in V.iter_mut().enumerate() {
                let tmp = value.bitand(!B[index]);
                value.commit(tmp);
            }

            debug!("begin solve_rec(V \\ B)");
            let mut W_double_prime = self.solve_recursive(V); // V has been updated to V \ B
            debug!("end solve_rec(V \\ B)");

            W_double_prime[alpha.to_index()] |= B;
            W_double_prime
        }
    }

    /// Computes the attractor for `player` to the set `U` within the vertices `V`.
    fn attractor(&mut self, player: Player, V: &Set, mut A: Set) -> Set {
        let initial_size = A.count_ones();

        // 2. Q = {v \in A}
        self.temp_queue.clear();
        for v in A.iter_ones() {
            self.temp_queue.push(VertexIndex::new(v));
        }

        // 4. While Q is not empty do
        // 5. w := Q.pop()
        while let Some(v) = self.temp_queue.pop() {
            // For every v \in Ew do
            for u in self.predecessors.predecessors(v) {
                if V[*v] {
                    let attracted = if self.game.owner(u) == player {
                        // v \in V and v in V_\alpha
                        true
                    } else {
                        // Check if all successors of u are in the attractor
                        self.game.outgoing_edges(u).all(|to| V[*to] && !A[*to])
                    };

                    if attracted && !A[*u] {
                        A.set(*u, true);
                        self.temp_queue.push(u);
                    }
                }
            }
        }

        debug!(
            "Attracted |A| = {} vertices towards |U| = {}",
            A.count_ones(),
            initial_size
        );

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
