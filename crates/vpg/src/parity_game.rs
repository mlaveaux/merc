//! Authors: Maurice Laveaux and Sjef van Loo

use core::fmt;

use merc_utilities::TagIndex;

/// The two players in a parity game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Player {
    Even,
    Odd,
}

impl Player {
    /// Constructs a player from its index.
    pub fn from_index(index: u8) -> Self {
        match index {
            0 => Player::Even,
            1 => Player::Odd,
            _ => panic!("Invalid player index {}", index),
        }
    }

    /// Constructs a player from a priority.
    pub fn from_priority(priority: &Priority) -> Self {
        if priority.value() % 2 == 0 {
            Player::Even
        } else {
            Player::Odd
        }
    }

    /// Returns the index of the player.
    pub fn to_index(&self) -> usize {
        match self {
            Player::Even => 0,
            Player::Odd => 1,
        }
    }

    /// Returns the opponent of the current player.
    pub fn opponent(&self) -> Self {
        match self {
            Player::Even => Player::Odd,
            Player::Odd => Player::Even,
        }
    }

    /// Returns the string representation of the solution for this player.
    pub fn solution(&self) -> &'static str {
        match self {
            Player::Even => "true",
            Player::Odd => "false",
        }
    }
}

impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Player::Even => write!(f, "even"),
            Player::Odd => write!(f, "odd"),
        }
    }
}

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

    /// Constructs a new parity game from an iterator over edges.
    pub fn from_edges<F, I>(
        initial_vertex: VertexIndex,
        owner: Vec<Player>,
        priority: Vec<Priority>,
        num_of_vertices: Option<usize>,
        mut edges: F,
    ) -> Self
    where
        F: FnMut() -> I,
        I: Iterator<Item = (VertexIndex, VertexIndex)>,
    {
        let mut vertices = Vec::new();
        if let Some(num_of_vertices) = num_of_vertices {
            vertices.resize_with(num_of_vertices, Default::default);
            debug_assert!(
                initial_vertex.value() < num_of_vertices,
                "Initial vertex index {} out of bounds {num_of_vertices}",
                initial_vertex.value()
            );
        }

        // Count the number of transitions for every state
        let mut num_of_edges = 0;
        for (from, to) in edges() {
            // Ensure that the states vector is large enough.
            if vertices.len() <= *from.max(to) {
                vertices.resize_with(*from.max(to) + 1, || 0);
            }

            vertices[*from] += 1;
            num_of_edges += 1;

            if let Some(num_of_vertices) = num_of_vertices {
                debug_assert!(
                    *from < num_of_vertices && *to < num_of_vertices,
                    "Vertex index out of bounds: from {:?}, to {:?}, num_of_vertices {}",
                    from,
                    to,
                    num_of_vertices
                );
            }
        }

        if initial_vertex.value() >= vertices.len() {
            // Ensure that the initial state is a valid state (and all states before it exist).
            vertices.resize_with(initial_vertex.value() + 1, Default::default);
        }

        // Track the number of transitions before every state.
        vertices.iter_mut().fold(0, |count, start| {
            let result = count + *start;
            *start = count;
            result
        });

        // Place the transitions, and increment the end for every state.
        let mut edges_to = vec![VertexIndex::new(0); num_of_edges];
        for (from, to) in edges() {
            let start = &mut vertices[*from];
            edges_to[*start] = to;
            *start += 1;
        }

        // Reset the offset.
        vertices.iter_mut().fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        Self {
            initial_vertex,
            owner,
            priority,
            vertices,
            edges_to,
        }
    }

    /// Returns the initial vertex of the parity game.
    pub fn initial_vertex(&self) -> VertexIndex {
        self.initial_vertex
    }

    /// Returns the number of vertices in the parity game.
    pub fn num_of_vertices(&self) -> usize {
        self.owner.len()
    }

    pub fn num_of_edges(&self) -> usize {
        self.edges_to.len()
    }

    /// Returns an iterator over all vertices in the parity game.
    pub fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        (0..self.num_of_vertices()).map(VertexIndex::new)
    }

    /// Returns an iterator over the outgoing edges for the given vertex.
    pub fn outgoing_edges(&self, state_index: VertexIndex) -> impl Iterator<Item = VertexIndex> + '_ {
        let start = self.vertices[*state_index];
        let end = self.vertices[*state_index + 1];

        (start..end).map(move |i| self.edges_to[i])
    }

    /// Returns the owner of the given vertex.
    pub fn owner(&self, vertex: VertexIndex) -> Player {
        self.owner[*vertex]
    }

    /// Returns the priority of the given vertex.
    pub fn priority(&self, vertex: VertexIndex) -> Priority {
        self.priority[*vertex]
    }
}
