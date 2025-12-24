//! Authors: Maurice Laveaux and Sjef van Loo

use merc_collections::ByteCompressedVec;
use merc_collections::bytevec;

use crate::PG;
use crate::ParityGame;
use crate::VertexIndex;

/// Stores the predecessors for a given parity game.
pub struct Predecessors {
    edges_from: ByteCompressedVec<VertexIndex>,
    vertex_to_predecessors: ByteCompressedVec<usize>,
}

impl Predecessors {
    /// Creates the predecessors structure for the given parity game.
    pub fn new(game: &ParityGame) -> Self {
        let mut edges_from = bytevec![VertexIndex::new(0); game.num_of_edges()];
        let mut state2incoming = bytevec![0; game.num_of_vertices()];

        // Count the number of incoming transitions for each state
        for state_index in game.iter_vertices() {
            for to in game.outgoing_edges(state_index) {
                state2incoming.update(to.value(), |start| *start += 1);
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
            for to in game.outgoing_edges(state_index) {
                state2incoming.update(to.value(), |start| {
                    edges_from.set(*start, state_index);
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
            vertex_to_predecessors: state2incoming,
        }
    }

    /// Returns an iterator over the predecessors the given vertex.
    pub fn predecessors(&self, state_index: VertexIndex) -> impl Iterator<Item = VertexIndex> + '_ {
        let start = self.vertex_to_predecessors.index(state_index.value());
        let end = self.vertex_to_predecessors.index(state_index.value() + 1);
        (start..end).map(move |i| self.edges_from.index(i))
    }
}
