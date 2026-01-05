//! Authors: Maurice Laveaux and Sjef van Loo

use std::fmt;

use delegate::delegate;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_symbolic::FormatConfigSet;
use merc_symbolic::minus;
use merc_utilities::MercError;

use crate::PG;
use crate::ParityGame;
use crate::Player;
use crate::Priority;
use crate::VertexIndex;

/// A variability parity game is an extension of a parity game where each edge is
/// associated with a BDD function representing the configurations in which the
/// edge is enabled.
///
/// # Details
///
/// This is also a max-priority parity game. There is also a configuration set associated
/// with the variability parity game, representing the overall configurations.
pub struct VariabilityParityGame {
    /// The underlying normal parity game.
    game: ParityGame,

    /// The overall configurations for the variability parity game.
    configuration: BDDFunction,

    /// The variables used in the configuration BDD.
    variables: Vec<BDDFunction>,

    /// Every edge has an associated BDD function representing the configurations
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
    /// Construct a new variability parity game from an iterator over edges.
    pub fn new(
        parity_game: ParityGame,
        configuration: BDDFunction,
        variables: Vec<BDDFunction>,
        edges_configuration: Vec<BDDFunction>,
    ) -> Self {
        // Check that the sizes are consistent
        debug_assert_eq!(
            edges_configuration.len(),
            parity_game.num_of_edges(),
            "There should be a configuration BDD for every edge"
        );

        Self {
            game: parity_game,
            configuration,
            variables,
            edges_configuration,
        }
    }

    /// Constructs a new variability parity game from an iterator over edges.
    ///
    /// The vertices are given by their owner and priority.
    /// The `edges` iterator should yield tuples of the form (from, configuration, to).
    pub fn from_edges<F, I>(
        manager_ref: &BDDManagerRef,
        initial_vertex: VertexIndex,
        owner: Vec<Player>,
        priority: Vec<Priority>,
        configuration: BDDFunction,
        variables: Vec<BDDFunction>,
        mut edges: F,
    ) -> Self
    where
        F: FnMut() -> I,
        I: Iterator<Item = (VertexIndex, BDDFunction, VertexIndex)>,
    {
        let num_of_vertices = owner.len();
        debug_assert_eq!(
            priority.len(),
            num_of_vertices,
            "Owner and priority vectors should have the same length"
        );

        let mut vertices = Vec::new();
        vertices.resize_with(num_of_vertices, Default::default);
        debug_assert!(
            initial_vertex.value() < num_of_vertices,
            "Initial vertex index {} out of bounds {num_of_vertices}",
            initial_vertex.value()
        );

        // Count the number of transitions for every state
        let mut num_of_edges = 0;
        for (from, _, to) in edges() {
            // Ensure that the states vector is large enough.
            if vertices.len() <= *from.max(to) {
                vertices.resize_with(*from.max(to) + 1, || 0);
            }

            vertices[*from] += 1;
            num_of_edges += 1;

            debug_assert!(
                *from < num_of_vertices && *to < num_of_vertices,
                "Vertex index out of bounds: from {:?}, to {:?}, num_of_vertices {}",
                from,
                to,
                num_of_vertices
            );
        }

        if initial_vertex.value() >= vertices.len() {
            // Ensure that the initial state is a valid state (and all states before it exist).
            vertices.resize_with(initial_vertex.value() + 1, Default::default);
        }

        // Sets the offset for every state into the edge arrays.
        vertices.iter_mut().fold(0, |count, start| {
            let result = count + *start;
            *start = count;
            result
        });

        // Place the transitions, and increment the end for every state.
        let mut edges_to = vec![VertexIndex::new(0); num_of_edges];
        let mut edges_configuration =
            manager_ref.with_manager_shared(|manager| vec![BDDFunction::f(manager); num_of_edges]);
        for (from, config, to) in edges() {
            let start = &mut vertices[*from];
            edges_to[*start] = to;
            edges_configuration[*start] = config;
            *start += 1;
        }

        // Reset the offset to the start.
        vertices.iter_mut().fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        vertices.push(num_of_edges); // Sentinel vertex

        Self {
            game: ParityGame::new(initial_vertex, owner, priority, vertices, edges_to),
            configuration,
            variables,
            edges_configuration,
        }
    }

    /// Returns an iterator over the outgoing edges of the given vertex.
    pub fn outgoing_conf_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = Edge<'_>> + '_ {
        let start = self.game.vertices()[*state_index];
        let end = self.game.vertices()[*state_index + 1];
        self.edges_configuration[start..end]
            .iter()
            .zip(self.game.edges_to()[start..end].iter())
            .map(|(configuration, &to)| Edge { to, configuration })
    }

    /// Returns true iff the parity game is total, checks all vertices have at least one outgoing edge.
    pub fn is_total(&self, manager_ref: &BDDManagerRef) -> Result<bool, MercError> {
        // Check that every vertex has at least one outgoing edge.
        for v in self.iter_vertices() {
            if self.outgoing_edges(v).next().is_none() {
                return Ok(false);
            }
        }

        // Check that the configurations of the outgoing edges cover the overall configuration.
        for v in self.iter_vertices() {
            // Compute the disjunction of all outgoing edge configurations.
            let covered = self.outgoing_conf_edges(v).try_fold(
                manager_ref.with_manager_shared(|manager| BooleanFunction::f(manager)),
                |acc: BDDFunction, edge| acc.or(edge.configuration()),
            )?;

            // If there are configurations not covered by the outgoing edges, the game is not total.
            if minus(&self.configuration, &covered)?.satisfiable() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Returns the overall configuration BDD of the variability parity game.
    pub fn configuration(&self) -> &BDDFunction {
        &self.configuration
    }

    /// Returns the variables used in the configuration BDD.
    pub fn variables(&self) -> &Vec<BDDFunction> {
        &self.variables
    }

    /// Returns the owners of the vertices in the variability parity game.
    pub(crate) fn owners(&self) -> &Vec<Player> {
        self.game.owners()
    }

    /// Returns the priorities of the vertices in the variability parity game.
    pub(crate) fn priorities(&self) -> &Vec<Priority> {
        self.game.priorities()
    }
}

impl PG for VariabilityParityGame {
    delegate! {
        to self.game {
            fn initial_vertex(&self) -> VertexIndex;
            fn num_of_vertices(&self) -> usize;
            fn num_of_edges(&self) -> usize;
            fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_;
            fn owner(&self, vertex: VertexIndex) -> Player;
            fn priority(&self, vertex: VertexIndex) -> Priority;
            fn outgoing_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = VertexIndex> + '_;
        }
    }
}

impl fmt::Display for VariabilityParityGame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Variability Parity Game:")?;
        writeln!(f, "Configuration BDD: {}", FormatConfigSet(self.configuration()))?;

        for v in self.iter_vertices() {
            write!(
                f,
                "v{}: {:?}, priority={}, edges=[",
                v,
                self.owner(v).to_index(),
                self.priority(v)
            )?;

            let mut first = true;
            for edge in self.outgoing_conf_edges(v) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "(v{}, {})", edge.to(), FormatConfigSet(edge.configuration()))?;
                first = false;
            }

            writeln!(f, "]")?;
        }

        Ok(())
    }
}
