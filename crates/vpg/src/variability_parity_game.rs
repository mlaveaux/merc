//! Authors: Maurice Laveaux and Sjef van Loo

use delegate::delegate;
use oxidd::bdd::BDDFunction;

use crate::ParityGame;
use crate::Player;
use crate::Priority;
use crate::VertexIndex;

/// A variability parity game is an extension of a parity game where each edge is
/// associated with a BDD function representing the configurations in which the
/// edge is enabled. This is also a max-priority parity game.
pub struct VariabilityParityGame {
    /// This is a normal parity game.
    game: ParityGame,

    /// The overall configurations for the variability parity game.
    configuration: BDDFunction,

    /// However, every edge has an associated BDD function representing the configurations
    /// in which the edge is enabled.
    edges_configuration: Vec<BDDFunction>,
}

/// Represents an edge in the parity game along with its configuration BDD.
pub struct Edge<'a> {
    to: VertexIndex,
    configuration: &'a BDDFunction,
}

impl<'a> Edge<'a> {
    /// Returns the target vertex of the edge.
    pub fn to(&self) -> VertexIndex {
        self.to
    }

    /// Returns the configuration BDD associated with the edge.
    pub fn configuration(&self) -> &BDDFunction {
        self.configuration
    }
}

impl VariabilityParityGame {
    /// Construct a new variability parity game from an iterator over transitions.
    pub fn new(
        parity_game: ParityGame,
        configuration: BDDFunction,
        edges_configuration: Vec<BDDFunction>,
    ) -> Self {
        // Check that the sizes are consistent
        debug_assert_eq!(
            edges_configuration.len(),
            parity_game.num_of_edges(),
            "There should be a configuration BDD for every transition"
        );

        Self {
            game: parity_game,
            configuration,
            edges_configuration,
        }
    }

    /// Returns an iterator over the outgoing edges of the given vertex.
    pub fn outgoing_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = Edge<'_>> + '_ {
        let start = self.game.vertices[*state_index];
        let end = self.game.vertices[*state_index + 1];
        self.edges_configuration[start..end]
            .iter()
            .zip(self.game.edges_to[start..end].iter())
            .map(|(configuration, &to)| Edge { to, configuration })
    }

    /// Returns the overall configuration BDD of the variability parity game.
    pub fn configuration(&self) -> &BDDFunction {
        &self.configuration
    }

    delegate! {
        to self.game {
            pub fn initial_vertex(&self) -> VertexIndex;
            pub fn num_of_vertices(&self) -> usize;
            pub fn num_of_edges(&self) -> usize;
            pub fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_;
            pub fn owner(&self, vertex: VertexIndex) -> Player;
            pub fn priority(&self, vertex: VertexIndex) -> Priority;
        }
    }
}
