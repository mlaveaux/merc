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

/// Represents an explicit parity game.
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
        transitions_to: Vec<VertexIndex>,
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
            edges_to: transitions_to,
            initial_vertex,
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
