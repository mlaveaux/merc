//! Authors: Maurice Laveaux and Sjef van Loo

use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_collections::ByteCompressedVec;
use merc_collections::bytevec;

use crate::PG;
use crate::VariabilityParityGame;
use crate::VertexIndex;

/// Stores the incoming transitions for a given variability parity game.
pub struct VariabilityPredecessors {
    edges_from: ByteCompressedVec<VertexIndex>,
    edges_configuration: Vec<BDDFunction>,
    vertex_to_predecessors: ByteCompressedVec<usize>,
}

impl VariabilityPredecessors {
    /// Creates the predecessors structure for the given parity game.
    pub fn new(manager_ref: &BDDManagerRef, game: &VariabilityParityGame) -> Self {
        let mut edges_from = bytevec![VertexIndex::new(0); game.num_of_edges()];
        let mut edges_configuration =
            manager_ref.with_manager_shared(|manager| vec![BDDFunction::f(manager); game.num_of_edges()]);
        let mut state2incoming = bytevec![0; game.num_of_vertices()];

        // Count the number of incoming transitions for each state
        for state_index in game.iter_vertices() {
            for edge in game.outgoing_conf_edges(state_index) {
                state2incoming.update(*edge.to(), |start| *start += 1);
            }
        }

        // Compute the start offsets (prefix sum)
        state2incoming.fold(0, |offset, start| {
            let new_offset = offset + *start;
            *start = offset;
            new_offset
        });

        // Place the transitions
        for state_index in game.iter_vertices() {
            for edge in game.outgoing_conf_edges(state_index) {
                state2incoming.update(*edge.to(), |start| {
                    edges_from.set(*start, state_index);
                    edges_configuration[*start] = edge.configuration().clone();
                    *start += 1;
                });
            }
        }

        state2incoming.fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        // Add sentinel state
        state2incoming.push(edges_from.len());

        Self {
            edges_from,
            edges_configuration,
            vertex_to_predecessors: state2incoming,
        }
    }

    /// Returns an iterator over the incoming transitions for the given state.
    pub fn predecessors(&self, state_index: VertexIndex) -> impl Iterator<Item = (VertexIndex, &BDDFunction)> + '_ {
        let start = self.vertex_to_predecessors.index(state_index.value());
        let end = self.vertex_to_predecessors.index(state_index.value() + 1);
        (start..end).map(move |i| (self.edges_from.index(i), &self.edges_configuration[i]))
    }
}
