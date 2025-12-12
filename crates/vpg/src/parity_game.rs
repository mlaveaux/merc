//! Authors: Maurice Laveaux and Sjef van Loo
use merc_utilities::TagIndex;

use crate::Player;

/// A unique type for the vertices.
pub struct VertexTag;

/// A unique type for the priorities.
pub struct PriorityTag;

/// The index for a vertex.
pub type VertexIndex = TagIndex<usize, VertexTag>;

/// The strong type for a priority.
pub type Priority = TagIndex<usize, PriorityTag>;

/// Represents an explicit max-priority parity game. This
/// means that higher priority values are more significant.
pub struct ParityGame {
    /// Stores the owner of every vertex.
    owner: Vec<Player>,

    /// Stores the priority of every vertex.
    priority: Vec<Priority>,

    // TODO: These should only be accessible in VariabilityParityGame
    /// Offsets into the transition array for every vertex.
    pub vertices: Vec<usize>,
    pub edges_to: Vec<VertexIndex>,

    initial_vertex: VertexIndex,
}

impl ParityGame {
    /// Construct a new parity game from an iterator over transitions.
    pub fn new(
        initial_vertex: VertexIndex,
        owner: Vec<Player>,
        priority: Vec<Priority>,
        vertices: Vec<usize>,
        edges_to: Vec<VertexIndex>,
    ) -> Self {
        // Check that the sizes are consistent
        debug_assert_eq!(
            owner.len(),
            priority.len(),
            "There should an owner and priority for every vertex"
        );
        debug_assert_eq!(
            vertices.len(),
            owner.len() + 1,
            "There should be an offset for every vertex, and the sentinel state"
        );

        Self {
            owner,
            priority,
            vertices,
            edges_to,
            initial_vertex,
        }
    }
}

impl PG for ParityGame {
    fn initial_vertex(&self) -> VertexIndex {
        self.initial_vertex
    }

    fn num_of_vertices(&self) -> usize {
        self.owner.len()
    }

    fn num_of_edges(&self) -> usize {
        self.edges_to.len()
    }

    fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        (0..self.num_of_vertices()).map(VertexIndex::new)
    }

    fn outgoing_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = VertexIndex> + '_ {
        let start = self.vertices[*state_index];
        let end = self.vertices[*state_index + 1];

        (start..end).map(move |i| self.edges_to[i])
    }

    fn owner(&self, vertex: VertexIndex) -> Player {
        self.owner[*vertex]
    }

    fn priority(&self, vertex: VertexIndex) -> Priority {
        self.priority[*vertex]
    }
}

/// A trait for types that can be interpreted as parity games.
pub trait PG {
    
    /// Returns the initial vertex of the parity game.
    fn initial_vertex(&self) -> VertexIndex;

    /// Returns the number of vertices in the parity game.
    fn num_of_vertices(&self) -> usize;

    /// Returns the number of edges in the parity game.
    fn num_of_edges(&self) -> usize;

    /// Returns an iterator over all vertices in the parity game.
    fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_;

    /// Returns an iterator over the outgoing edges for the given vertex.
    fn outgoing_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = VertexIndex> + '_;

    /// Returns the owner of the given vertex.
    fn owner(&self, vertex: VertexIndex) -> Player;

    /// Returns the priority of the given vertex.
    fn priority(&self, vertex: VertexIndex) -> Priority;
}