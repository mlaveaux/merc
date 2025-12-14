//! Authors: Maurice Laveaux and Sjef van Loo
use std::fmt;

use itertools::Itertools;

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

    /// Offsets into the transition array for every vertex.
    vertices: Vec<usize>,
    edges_to: Vec<VertexIndex>,

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
        debug_assert_eq!(initial_vertex, 0, "The initial vertex should be vertex 0");

        Self {
            owner,
            priority,
            vertices,
            edges_to,
            initial_vertex,
        }
    }

    /// Constructs a new parity game from an iterator over edges.
    ///
    /// The vertices are given by their owner and priority. The `edges` iterator
    /// should yield tuples of the form (from, to). If `make_total` is true,
    /// self-loops are added to vertices with no outgoing edges.
    pub fn from_edges<F, I>(
        initial_vertex: VertexIndex,
        owner: Vec<Player>,
        priority: Vec<Priority>,
        make_total: bool,
        mut edges: F,
    ) -> Self
    where
        F: FnMut() -> I,
        I: Iterator<Item = (VertexIndex, VertexIndex)>,
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
        for (from, to) in edges() {
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

        // If make_total is true, reserve space for self-loops on vertices with no outgoing edges
        if make_total {
            for count in vertices.iter_mut() {
                if *count == 0 {
                    *count = 1;
                    num_of_edges += 1;
                }
            }
        }

        // Sets the offset for every state into the edge arrays.
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

        // If make_total is true, add self-loops for vertices that had no outgoing edges
        if make_total {
            for vertex_idx in 0..num_of_vertices {
                let start = vertices[vertex_idx];
                let previous = if vertex_idx > 0 {
                    vertices[vertex_idx - 1]
                } else {
                    0
                };
                if start == previous {
                    // No outgoing edges, add self-loop
                    edges_to[start] = VertexIndex::new(vertex_idx);
                    vertices[vertex_idx] += 1; // Increment end offset
                }
            }
        }

        // Reset the offset to the start.
        vertices.iter_mut().fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        vertices.push(num_of_edges); // Sentinel vertex

        Self {
            initial_vertex,
            owner,
            priority,
            vertices,
            edges_to,
        }
    }

    /// Returns true iff the parity game is total, checks all vertices have at least one outgoing edge.
    pub fn is_total(&self) -> bool {       
        for v in self.iter_vertices() {
            if self.outgoing_edges(v).next().is_none() {
                return false;
            }
        }

        true
    }

    /// Returns the vertices array.
    pub(crate) fn vertices(&self) -> &Vec<usize> {
        &self.vertices
    }

    /// Returns the edges_to array.
    pub(crate) fn edges_to(&self) -> &Vec<VertexIndex> {
        &self.edges_to
    }

    /// Returns the owners array.
    pub(crate) fn owners(&self) -> &Vec<Player> {
        &self.owner
    }

    /// Returns the priorities array.
    pub(crate) fn priorities(&self) -> &Vec<Priority> {
        &self.priority
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

impl fmt::Debug for ParityGame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ParityGame {{")?;
        writeln!(f, "  num_vertices: {},", self.num_of_vertices())?;
        writeln!(f, "  num_edges: {},", self.num_of_edges())?;
        writeln!(f, "  initial_vertex: v{},", *self.initial_vertex)?;
        writeln!(f, "  vertices: [")?;
        for v in self.iter_vertices() {
            let owner = self.owner(v);
            let prio = self.priority(v);
            write!(f, "    {}: ({:?}, priority: {}, outgoing: [", *v, owner.to_index(), *prio)?;

            write!(f, "{}", self.outgoing_edges(v).format(", "))?;
            writeln!(f, "]),")?;
        }
        writeln!(f, "  ]")?;
        writeln!(f, "}}")
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
