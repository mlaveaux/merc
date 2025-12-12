use rand::Rng;

use crate::ParityGame;
use crate::Player;
use crate::Priority;
use crate::VertexIndex;

/// Creates a random parity game with the given number of vertices, priorities, and outdegree.
pub fn random_parity_game(
    rng: &mut impl Rng,
    num_of_vertices: usize,
    num_of_priorities: usize,
    outdegree: usize,
) -> ParityGame {
    assert!(num_of_vertices > 0, "Parity game must have at least one vertex");
    assert!(num_of_priorities > 0, "Parity game must have at least one priority");

    // Randomly assign priorities to each vertex in range [0, num_of_priorities).
    let priority: Vec<Priority> = (0..num_of_vertices)
        .map(|_| Priority::new(rng.random_range(0..num_of_priorities)))
        .collect();

    // Option 1: owner based on parity of priority; Option 2: random owner.
    // Mirror random_lts_monolithic style by using randomness.
    let owner: Vec<Player> = (0..num_of_vertices)
        .map(|_| Player::from_index(rng.random_range(0..2)))
        .collect();

    // Build edges using a closure that can be iterated twice (as required by from_edges).
    // We generate a deterministic set by capturing a precomputed edge list.
    let mut edge_list: Vec<(VertexIndex, VertexIndex)> = Vec::new();
    edge_list.reserve(num_of_vertices * outdegree);

    for v in 0..num_of_vertices {
        // For each vertex, generate 0..outdegree outgoing edges.
        for _ in 0..rng.random_range(0..outdegree) {
            let to = rng.random_range(0..num_of_vertices);
            edge_list.push((VertexIndex::new(v), VertexIndex::new(to)));
        }
    }

    // Ensure at least the initial vertex exists.
    let initial_vertex = VertexIndex::new(0);

    ParityGame::from_edges(initial_vertex, owner, priority, || edge_list.iter().cloned())
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::PG;
    use crate::random_parity_game;

    #[test]
    fn test_random_parity_game() {
        random_test(100, |rng| {
            let pg = random_parity_game(rng, 10, 5, 3);
            assert_eq!(pg.num_of_vertices(), 10);
        })
    }
}
