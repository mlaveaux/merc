use merc_utilities::TagIndex;



/// The two players in a parity game.
#[derive(Clone)]
pub enum Player {
    Even,
    Odd
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

    /// Returns the opponent of the current player.
    pub fn opponent(&self) -> Self {
        match self {
            Player::Even => Player::Odd,
            Player::Odd => Player::Even,
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

    /// Offsets into the transition array for every vertex.
    vertices: Vec<usize>,
    transitions_to: Vec<VertexIndex>,

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
        debug_assert_eq!(owner.len(), priority.len(), "There should an owner and priority for every vertex");
        debug_assert_eq!(vertices.len(), owner.len(), "There should be an offset for every vertex");

        Self {
            owner,
            priority,
            vertices,
            transitions_to,
            initial_vertex,
        }
    }

    /// Returns the initial vertex of the parity game.
    pub fn initial_vertex(&self) -> VertexIndex {
        self.initial_vertex
    }
}