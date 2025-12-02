#![allow(nonstandard_style)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse

use std::fmt;

use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VariabilityPredecessors;
use crate::VertexIndex;

/// Solves the given variability parity game using the Zielonka algorithm.
pub fn solve_variability_zielonka(
    manager_ref: &BDDManagerRef,
    game: &VariabilityParityGame,
    alternative_solving: bool,
) -> Player {
    let mut zielonka = VariabilityZielonkaSolver::new(manager_ref, game, alternative_solving);

    let V = Submap::new(
        manager_ref,
        manager_ref.with_manager_shared(|manager| {
            if alternative_solving {
                BDDFunction::t(manager)
            } else {
                game.configuration().clone()
            }
        }),
        game.num_of_vertices(),
    );
    let W = zielonka.solve_recursive(V);

    // Check that the result is a valid partition
    unimplemented!("Check that the result is a valid partition");
}

struct VariabilityZielonkaSolver<'a> {
    game: &'a VariabilityParityGame,

    /// Whether to use an alternative solving method.
    alternative_solving: bool,

    /// Reused temporary queue for attractor computation.
    temp_queue: Vec<VertexIndex>,

    /// Stores the predecessors of the game.
    predecessors: VariabilityPredecessors,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// Keeps track of the total number of recursive calls.
    recursive_calls: usize,
}

impl<'a> VariabilityZielonkaSolver<'a> {
    /// Creates a new VariabilityZielonkaSolver for the given game.
    pub fn new(manager_ref: &BDDManagerRef, game: &'a VariabilityParityGame, alternative_solving: bool) -> Self {
        // Keep track of the vertices for each priority
        let mut priority_vertices = Vec::new();

        for v in game.iter_vertices() {
            let prio = game.priority(v);

            while prio >= priority_vertices.len() {
                priority_vertices.push(Vec::new());
            }

            priority_vertices[prio].push(v);
        }

        Self {
            game,
            temp_queue: Vec::new(),
            predecessors: VariabilityPredecessors::new(manager_ref, &game),
            priority_vertices,
            recursive_calls: 0,
            alternative_solving
        }
    }

    /// Solves the variability parity game for the given set of vertices V.
    fn solve_recursive(&mut self, V: Submap) -> [Submap; 2] {
        [V.clone(), V]
    }

    /// Computes the attractor for `player` to the set `U` within the vertices `V`.
    fn attractor(&mut self, alpha: Player, gamme: &Submap, U: Submap) -> Submap {
        U
    }

    /// Returns the highest and lowest priority in the given set of vertices V.
    fn get_highest_lowest_prio(&self, V: &Submap) -> (Priority, Priority) {
        let mut highest = usize::MIN;
        let mut lowest = usize::MAX;

        // for v in V.iter_ones() {
        //     let prio = self.game.priority(VertexIndex::new(v));
        //     highest = highest.max(*prio);
        //     lowest = lowest.min(*prio);
        // }

        (Priority::new(highest), Priority::new(lowest))
    }
}

/// A mapping from vertices to configurations.
#[derive(Clone)]
struct Submap {
    /// The mapping from vertex indices to BDD functions.
    mapping: Vec<BDDFunction>,

    /// Invariant: counts the number of empty positions in the mapping.
    non_empty_count: usize,
}

impl Submap {
    /// Creates a new empty Submap for the given number of vertices.
    fn new(manager_ref: &BDDManagerRef, initial: BDDFunction, num_of_vertices: usize) -> Self {
        Self {
            mapping: vec![initial.clone(); num_of_vertices],
            non_empty_count: 0,
        }
    }

    fn get(&self, v: VertexIndex) -> &BDDFunction {
        &self.mapping[*v]
    }
}

impl fmt::Debug for Submap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, func) in self.mapping.iter().enumerate() {
            writeln!(f, "  {}", i)?;
        }
        Ok(())
    }
}
