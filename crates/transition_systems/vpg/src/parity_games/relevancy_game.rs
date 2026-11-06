//! A parity game whose vertices are numbered in relevance order.
//!
//! # Definitions
//!
//! Let `G = (V, V_0, V_1, E, pr)` be a parity game with `n = |V|` vertices
//! `V = {0, ..., n-1}`, partitioned into the Even vertices `V_0` and Odd
//! vertices `V_1`, edges `E ⊆ V × V` and priority function `pr: V → ℕ`.
//!
//! The *relevance order* `⊏` on `V` is the strict total order
//!
//! ```text
//!     v ⊏ w   iff   pr(v) < pr(w), or pr(v) = pr(w) and v < w
//! ```
//!
//! i.e. the lexicographic order on `(pr(v), v)`. A vertex `w` is *more
//! relevant* than `v` if `v ⊏ w`. Since the order is total there is a unique
//! bijection `π: V → {0, ..., n-1}` with `v ⊏ w ⟺ π(v) < π(w)`, mapping every
//! vertex to its rank in the relevance order.
//!
//! The *relevancy game* of `G` is the isomorphic copy
//! `G' = (V', V'_0, V'_1, E', pr')` along `π`:
//!
//! ```text
//!     V'    = {0, ..., n-1}
//!     V'_i  = { π(v)        | v ∈ V_i }
//!     E'    = { (π(v), π(w)) | (v, w) ∈ E }
//!     pr'   = pr ∘ π⁻¹
//! ```
//!
//! # Explanation
//!
//! In `G'` the relevance order coincides with the natural order on the vertex
//! indices, and `pr'` is monotone: `i < j` implies `pr'(i) ≤ pr'(j)`. Hence
//! the most relevant vertex of a set of vertices (for instance, the vertices
//! on a cycle, or the symmetric difference of two sets) is simply its maximum
//! index, and two vertices can be compared by relevance without looking up
//! their priorities. This is the vertex order used by the strategy
//! improvement valuation of Vöge and Jurdziński, see
//! [`crate::solve_strategy_improvement`].
//!
//! Since `π` is an isomorphism, `G'` has the same solution as `G`: a vertex
//! `v` is won by player `i` in `G` iff `π(v)` is won by player `i` in `G'`,
//! and a positional strategy `σ'` in `G'` is winning iff `π⁻¹ ∘ σ' ∘ π` is
//! winning in `G`. [`RelevancyGame::to_original`] and
//! [`RelevancyGame::to_relevancy`] implement `π⁻¹` and `π` respectively.
//!
//! Additionally, the outgoing edges of every vertex of `G'` are listed in
//! increasing (relevance) order and without duplicates, so that "the first
//! best successor" is well defined and independent of the input order.

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;

use crate::Edge;
use crate::PG;
use crate::ParityGame;
use crate::Player;
use crate::PlayerVec;
use crate::Priority;
use crate::VertexIndex;

/// The relevancy game of a parity game: an isomorphic copy in which the
/// vertices are numbered by the relevance order `(priority, index)`. See the
/// [module documentation](self) for the definitions.
pub struct RelevancyGame {
    /// The relabelled game `G'`, with initial vertex `π(v_I)`.
    game: ParityGame,
    /// `π⁻¹`: the original vertex of every relabelled vertex.
    to_original: Vec<VertexIndex>,
    /// `π`: the relabelled vertex of every original vertex.
    to_relevancy: Vec<VertexIndex>,
}

impl RelevancyGame {
    /// Constructs the relevancy game of the given game.
    pub fn new<G: PG>(game: &G) -> Self {
        let n = game.num_of_vertices();

        let mut to_original: Vec<VertexIndex> = game.iter_vertices().collect();
        to_original.sort_by_key(|&v| (game.priority(v), v));

        let mut to_relevancy = vec![VertexIndex::new(0); n];
        for (new, &old) in to_original.iter().enumerate() {
            to_relevancy[*old] = VertexIndex::new(new);
        }

        let mut owners = PlayerVec::with_capacity(n);
        let mut priorities = ByteCompressedVec::with_capacity(n, game.highest_priority().value().bytes_required());
        let mut vertices = Vec::with_capacity(n + 1);
        let mut edges_to = ByteCompressedVec::with_capacity(game.num_of_edges(), n.bytes_required());
        let mut successors = Vec::new();
        for &old in &to_original {
            owners.push(game.owner(old));
            priorities.push(game.priority(old));
            vertices.push(edges_to.len());

            successors.clear();
            successors.extend(game.outgoing_edges(old).map(|edge| to_relevancy[*edge.to()]));
            successors.sort_unstable();
            successors.dedup();
            for &w in &successors {
                edges_to.push(w);
            }
        }
        vertices.push(edges_to.len());

        // The empty game has no vertices to relabel the initial vertex with.
        let initial_vertex = to_relevancy
            .get(*game.initial_vertex())
            .copied()
            .unwrap_or(VertexIndex::new(0));

        Self {
            game: ParityGame::new(initial_vertex, owners, priorities, vertices, edges_to),
            to_original,
            to_relevancy,
        }
    }

    /// Returns `π⁻¹(v)`: the vertex of the original game for vertex `v` of this game.
    pub fn to_original(&self, v: VertexIndex) -> VertexIndex {
        self.to_original[*v]
    }

    /// Returns `π(v)`: the vertex of this game for vertex `v` of the original game.
    pub fn to_relevancy(&self, v: VertexIndex) -> VertexIndex {
        self.to_relevancy[*v]
    }
}

impl PG for RelevancyGame {
    type Label = ();

    fn initial_vertex(&self) -> VertexIndex {
        self.game.initial_vertex()
    }

    fn num_of_vertices(&self) -> usize {
        self.game.num_of_vertices()
    }

    fn num_of_edges(&self) -> usize {
        self.game.num_of_edges()
    }

    fn highest_priority(&self) -> Priority {
        self.game.highest_priority()
    }

    fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        self.game.iter_vertices()
    }

    fn outgoing_edges<'a>(&'a self, vertex: VertexIndex) -> impl Iterator<Item = Edge<'a, ()>> + 'a {
        self.game.outgoing_edges(vertex)
    }

    fn owner(&self, vertex: VertexIndex) -> Player {
        self.game.owner(vertex)
    }

    fn priority(&self, vertex: VertexIndex) -> Priority {
        self.game.priority(vertex)
    }

    fn is_total(&self) -> bool {
        self.game.is_total()
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::random_parity_game;
    use crate::read_pg;
    use crate::solve_zielonka;

    use super::*;

    #[test]
    fn test_relevancy_game_initial_vertex() {
        // Vertex 0 has the highest priority, so it becomes the last vertex.
        let game = read_pg("parity 3;\nstart 2;\n0 5 0 1;\n1 1 1 2;\n2 3 0 0;\n".as_bytes()).unwrap();
        assert_eq!(game.initial_vertex(), VertexIndex::new(2));

        let relevancy = RelevancyGame::new(&game);
        assert_eq!(relevancy.initial_vertex(), VertexIndex::new(1));
        assert_eq!(relevancy.to_original(relevancy.initial_vertex()), game.initial_vertex());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // bitvec is incompatible with miri.
    fn test_random_relevancy_game() {
        random_test(100, |rng| {
            let game = random_parity_game(rng, true, 50, 10, 5);
            let relevancy = RelevancyGame::new(&game);
            assert_eq!(relevancy.num_of_vertices(), game.num_of_vertices());
            assert!(relevancy.is_total());
            assert_eq!(relevancy.to_original(relevancy.initial_vertex()), game.initial_vertex());

            for v in relevancy.iter_vertices() {
                let old = relevancy.to_original(v);
                assert_eq!(relevancy.to_relevancy(old), v);
                assert_eq!(relevancy.owner(v), game.owner(old));
                assert_eq!(relevancy.priority(v), game.priority(old));

                // The relevance order is the natural order on the indices.
                if *v > 0 {
                    let previous = relevancy.to_original(VertexIndex::new(*v - 1));
                    assert!((game.priority(previous), previous) < (game.priority(old), old));
                }

                // The successors are sorted and map back to the original edges.
                let successors: Vec<VertexIndex> = relevancy.outgoing_edges(v).map(|edge| edge.to()).collect();
                assert!(successors.is_sorted_by(|a, b| a < b));
                let mut expected: Vec<VertexIndex> = game
                    .outgoing_edges(old)
                    .map(|edge| relevancy.to_relevancy(edge.to()))
                    .collect();
                expected.sort_unstable();
                expected.dedup();
                assert_eq!(successors, expected);
            }

            // Isomorphic games have the same solution.
            let (expected, _) = solve_zielonka(&game, false);
            let (solution, _) = solve_zielonka(&relevancy, false);
            for v in relevancy.iter_vertices() {
                let old = relevancy.to_original(v);
                assert_eq!(solution[0][*v], expected[0][*old]);
            }
        });
    }
}
