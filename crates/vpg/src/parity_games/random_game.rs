use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use rand::Rng;

use merc_symbolic::create_variables;
use merc_symbolic::random_bdd;
use merc_utilities::MercError;

use crate::PG;
use crate::ParityGame;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VertexIndex;
use crate::make_vpg_total;

/// Creates a random parity game with the given number of vertices, priorities, and outdegree.
pub fn random_parity_game(
    rng: &mut impl Rng,
    make_total: bool,
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
    let mut edge_list: Vec<(VertexIndex, VertexIndex)> = Vec::with_capacity(num_of_vertices * outdegree);

    for v in 0..num_of_vertices {
        // For each vertex, generate 0..outdegree outgoing edges.
        for _ in 0..rng.random_range(0..outdegree) {
            let to = rng.random_range(0..num_of_vertices);
            edge_list.push((VertexIndex::new(v), VertexIndex::new(to)));
        }
    }

    // Ensure at least the initial vertex exists.
    let initial_vertex = VertexIndex::new(0);

    ParityGame::from_edges(initial_vertex, owner, priority, make_total, || {
        edge_list.iter().cloned()
    })
}

/// Creates a random parity game with the given number of vertices, priorities, and outdegree.
pub fn random_variability_parity_game(
    manager_ref: &BDDManagerRef,
    rng: &mut impl Rng,
    make_total: bool,
    num_of_vertices: usize,
    num_of_priorities: usize,
    outdegree: usize,
    number_of_variables: u32,
) -> Result<VariabilityParityGame, MercError> {
    let pg = random_parity_game(rng, make_total, num_of_vertices, num_of_priorities, outdegree);

    // Create random feature variables.
    let variables: Vec<BDDFunction> = create_variables(manager_ref, number_of_variables)?;

    // Overall configuration is the conjunction of all features (i.e., all features enabled).
    let configuration = random_bdd(manager_ref, rng, &variables)?;

    // Create random edge configurations.
    let mut edges_configuration: Vec<BDDFunction> = Vec::with_capacity(pg.num_of_edges());
    for _ in 0..pg.num_of_edges() {
        edges_configuration.push(random_bdd(manager_ref, rng, &variables)?);
    }

    let result = VariabilityParityGame::new(pg, configuration, variables, edges_configuration);

    if make_total {
        make_vpg_total(manager_ref, &result)
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::PG;
    use crate::random_parity_game;
    use crate::random_variability_parity_game;

    #[test]
    fn test_random_parity_game() {
        random_test(100, |rng| {
            let pg = random_parity_game(rng, false, 10, 5, 3);
            assert_eq!(pg.num_of_vertices(), 10);
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_variability_parity_game() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let vpg = random_variability_parity_game(&manager_ref, rng, false, 10, 5, 3, 3).unwrap();
            assert_eq!(vpg.num_of_vertices(), 10);
        })
    }
}
