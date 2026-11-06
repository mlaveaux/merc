#[cfg(debug_assertions)]
use std::collections::HashSet;
use std::fmt;

use itertools::Itertools;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;
use merc_collections::Graph;
use merc_utilities::TagIndex;

use crate::Player;
use crate::PlayerVec;

/// A strong type for the vertices.
pub struct VertexTag;

/// A strong type for the priorities.
pub struct PriorityTag;

/// The index for a vertex.
pub type VertexIndex = TagIndex<usize, VertexTag>;

/// The strong type for a priority.
pub type Priority = TagIndex<usize, PriorityTag>;

/// Represents an explicit max-priority parity game. This means that higher
/// priority values are more significant.
///
/// # Details
///
/// The per-vertex and per-edge arrays are stored compactly, since a game (and
/// any derived game, e.g. from [`crate::compute_reachable`]) is kept around for
/// as long as it is being solved: owners as one bit per vertex (see
/// [`PlayerVec`]), and priorities, offsets and edge targets byte compressed,
/// which for the small priority and vertex-count ranges that games in practice
/// have means far fewer than the eight bytes a `usize` normally costs.
pub struct ParityGame {
    /// Stores the owner of every vertex.
    owner: PlayerVec,

    /// Stores the priority of every vertex.
    priority: ByteCompressedVec<Priority>,

    /// Offsets into the transition array for every vertex, plus a sentinel.
    vertices: ByteCompressedVec<usize>,
    edges_to: ByteCompressedVec<VertexIndex>,

    initial_vertex: VertexIndex,
}

impl ParityGame {
    /// Constructs a new parity game from an iterator over edges.
    ///
    /// The vertices are given by their owner and priority. The `edges` iterator
    /// should yield tuples of the form (from, to). If `make_total` is true,
    /// self-loops are added on-the-fly to vertices with no outgoing edges.
    pub fn from_edges<F, I>(
        initial_vertex: VertexIndex,
        owner: PlayerVec,
        mut priority: ByteCompressedVec<Priority>,
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

        // Place the transitions directly into a byte-compressed array.
        let mut edges_to = ByteCompressedVec::with_capacity(num_of_edges, num_of_vertices.bytes_required());
        edges_to.resize_zeroed(num_of_edges, num_of_vertices.bytes_required());
        for (from, to) in edges() {
            let start = &mut vertices[*from];
            edges_to.set(*start, to);
            *start += 1;
        }

        // If make_total is true, add self-loops for vertices that had no outgoing edges
        if make_total {
            for vertex_idx in 0..num_of_vertices {
                let start = vertices[vertex_idx];
                let previous = if vertex_idx > 0 { vertices[vertex_idx - 1] } else { 0 };
                if start == previous {
                    // No outgoing edges, add self-loop
                    edges_to.set(start, VertexIndex::new(vertex_idx));
                    vertices[vertex_idx] += 1; // Increment end offset

                    // Change the priority of the vertex such that the self-loop is winning for the opponent.
                    priority.set(vertex_idx, Priority::new(owner.index(vertex_idx).opponent().to_index()));
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
        Self::new(initial_vertex, owner, priority, vertices, edges_to)
    }

    /// Constructs a parity game directly from the given arrays.
    ///
    /// `vertices` is taken as a plain vector since every caller builds it with a
    /// construction that repeatedly mutates offsets.
    pub(crate) fn new(
        initial_vertex: VertexIndex,
        owner: PlayerVec,
        priority: ByteCompressedVec<Priority>,
        vertices: Vec<usize>,
        edges_to: ByteCompressedVec<VertexIndex>,
    ) -> Self {
        // The offsets are monotonically increasing up to the sentinel, so its
        // maximum bounds the number of bytes needed without having to scan for
        // the actual maximum.
        let num_of_edges = edges_to.len();

        let mut compressed_vertices = ByteCompressedVec::with_capacity(vertices.len(), num_of_edges.bytes_required());
        for offset in vertices {
            compressed_vertices.push(offset);
        }

        let result = Self {
            owner,
            priority,
            vertices: compressed_vertices,
            edges_to,
            initial_vertex,
        };
        result.assert_consistent();
        result
    }

    /// Returns the vertices array.
    pub(crate) fn vertices(&self) -> &ByteCompressedVec<usize> {
        &self.vertices
    }

    /// Returns the edges_to array.
    pub(crate) fn edges_to(&self) -> &ByteCompressedVec<VertexIndex> {
        &self.edges_to
    }

    /// Returns the owners array.
    pub(crate) fn owners(&self) -> &PlayerVec {
        &self.owner
    }

    /// Returns the priorities array.
    pub(crate) fn priorities(&self) -> &ByteCompressedVec<Priority> {
        &self.priority
    }

    /// Asserts that the internal data structures are consistent with each other.
    fn assert_consistent(&self) {
        // Check that the sizes are consistent
        debug_assert_eq!(
            self.owner.len(),
            self.priority.len(),
            "There should an owner and priority for every vertex"
        );
        debug_assert_eq!(
            self.vertices.len(),
            self.owner.len() + 1,
            "There should be an offset for every vertex, and the sentinel state"
        );
        debug_assert!(
            self.initial_vertex.value() < self.owner.len().max(1),
            "The initial vertex {} should be a vertex of the game",
            self.initial_vertex.value()
        );

        // Check that there are no duplicate edges
        #[cfg(debug_assertions)]
        {
            let mut seen = HashSet::new();
            for vertex in self.iter_vertices() {
                seen.clear();
                for edge in self.outgoing_edges(vertex) {
                    if !seen.insert(edge.to()) {
                        panic!("Duplicate edge from vertex {:?} to vertex {:?}", vertex, edge.to());
                    }
                }
            }
        }
    }
}

impl PG for ParityGame {
    type Label = ();

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

    fn outgoing_edges<'a>(&'a self, state_index: VertexIndex) -> impl Iterator<Item = Edge<'a, ()>> + 'a {
        let start = self.vertices.index(*state_index);
        let end = self.vertices.index(*state_index + 1);

        (start..end).map(move |i| Edge::new(&(), self.edges_to.index(i)))
    }

    fn owner(&self, vertex: VertexIndex) -> Player {
        self.owner.index(*vertex)
    }

    fn priority(&self, vertex: VertexIndex) -> Priority {
        self.priority.index(*vertex)
    }

    fn highest_priority(&self) -> Priority {
        self.priority.iter().fold(Priority::new(0), |max, p| max.max(p))
    }

    fn is_total(&self) -> bool {
        // The parity game is total if every vertex has at least one outgoing edge.
        self.iter_vertices().all(|v| self.outgoing_edges(v).next().is_some())
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
            write!(
                f,
                "    {}: ({:?}, priority: {}, outgoing: [",
                *v,
                owner.to_index(),
                *prio
            )?;

            write!(f, "{}", self.outgoing_edges(v).map(|edge| edge.to()).format(", "))?;
            writeln!(f, "]),")?;
        }
        writeln!(f, "  ]")?;
        writeln!(f, "}}")
    }
}

/// A trait for parity games.
pub trait PG {
    type Label;

    /// Returns the initial vertex of the parity game.
    fn initial_vertex(&self) -> VertexIndex;

    /// Returns the number of vertices in the parity game.
    fn num_of_vertices(&self) -> usize;

    /// Returns the number of edges in the parity game.
    fn num_of_edges(&self) -> usize;

    /// Returns the highest priority in the parity game.
    fn highest_priority(&self) -> Priority;

    /// Returns an iterator over all vertices in the parity game.
    ///
    /// Assumes that the returned vertices are in the range 0 to
    /// num_of_vertices() - 1.
    fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_;

    /// Returns an iterator over the outgoing edges for the given vertex.
    fn outgoing_edges<'a>(&'a self, vertex: VertexIndex) -> impl Iterator<Item = Edge<'a, Self::Label>> + 'a;

    /// Returns the owner of the given vertex.
    fn owner(&self, vertex: VertexIndex) -> Player;

    /// Returns the priority of the given vertex.
    fn priority(&self, vertex: VertexIndex) -> Priority;

    /// Returns true iff the parity game is total, checks all vertices have at least one outgoing edge.
    fn is_total(&self) -> bool;
}

/// Represents an edge in a parity game.
pub struct Edge<'a, L> {
    label: &'a L,
    to: VertexIndex,
}

impl<'a, L> Edge<'a, L> {
    /// Creates a new edge.
    pub fn new(label: &'a L, to: VertexIndex) -> Self {
        Self { label, to }
    }

    /// Returns the label of the edge.
    pub fn label(&self) -> &'a L {
        self.label
    }

    pub fn to(&self) -> VertexIndex {
        self.to
    }
}

/// A wrapper to view a parity game as a graph.
pub struct AsGraph<'a, G: PG>(pub &'a G);

/// Every parity game is also a graph.
impl<G: PG> Graph for AsGraph<'_, G> {
    type VertexIndex = VertexIndex;
    type LabelIndex = ();

    fn num_of_vertices(&self) -> usize {
        self.0.num_of_vertices()
    }

    fn iter_vertices(&self) -> impl Iterator<Item = Self::VertexIndex> {
        self.0.iter_vertices()
    }

    fn outgoing_edges(
        &self,
        vertex: <Self as Graph>::VertexIndex,
    ) -> impl Iterator<Item = (Self::LabelIndex, Self::VertexIndex)> {
        self.0.outgoing_edges(vertex).map(|edge| ((), edge.to()))
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::PG;
    use crate::random_parity_game;

    #[test]
    #[cfg_attr(miri, ignore)] // bitvec is incompatible with miri.
    fn test_random_parity_game_make_total() {
        random_test(100, |rng| {
            let game = random_parity_game(rng, true, 50, 10, 5);
            assert!(game.is_total());
        });
    }
}
