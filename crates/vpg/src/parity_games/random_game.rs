use rand::Rng;
use rand::RngExt;

use crate::ParityGame;
use crate::ParityGameBuilder;
use crate::Player;
use crate::Priority;
use crate::VertexIndex;

#[cfg(test)]
use oxidd::BooleanFunction;
#[cfg(test)]
use oxidd::Manager;
#[cfg(test)]
use oxidd::ManagerRef;
#[cfg(test)]
use oxidd::bdd::BDDFunction;
#[cfg(test)]
use oxidd::bdd::BDDManagerRef;

#[cfg(test)]
use merc_symbolic::random_bdd;
#[cfg(test)]
use merc_utilities::MercError;

#[cfg(test)]
use crate::PG;
#[cfg(test)]
use crate::VariabilityParityGame;
#[cfg(test)]
use crate::make_vpg_total;

/// Creates a random parity game with the given number of vertices, priorities, and outdegree.
pub fn random_parity_game<R: Rng>(
    rng: &mut R,
    make_total: bool,
    num_of_vertices: usize,
    num_of_priorities: usize,
    outdegree: usize,
) -> ParityGame {
    assert!(num_of_vertices > 0, "Parity game must have at least one vertex");
    assert!(num_of_priorities > 0, "Parity game must have at least one priority");

    let mut builder = ParityGameBuilder::new(VertexIndex::new(0));

    for v in 0..num_of_vertices {
        let priority = Priority::new(rng.random_range(0..num_of_priorities));
        let owner = Player::from_index(rng.random_range(0..2));
        builder.add_vertex(VertexIndex::new(v), owner, priority);
    }

    // For each vertex, generate 0..outdegree outgoing edges.
    for v in 0..num_of_vertices {
        for _ in 0..rng.random_range(0..outdegree) {
            let to = rng.random_range(0..num_of_vertices);
            builder.add_edge(VertexIndex::new(v), VertexIndex::new(to));
        }
    }

    builder.finish(make_total, true)
}

/// Creates a random variability parity game with the given number of vertices, priorities, and
/// outdegree.
#[cfg(test)]
pub(crate) fn random_variability_parity_game<R: Rng>(
    manager_ref: &BDDManagerRef,
    rng: &mut R,
    make_total: bool,
    num_of_vertices: usize,
    num_of_priorities: usize,
    outdegree: usize,
    number_of_variables: u32,
) -> Result<VariabilityParityGame, MercError> {
    let pg = random_parity_game(rng, make_total, num_of_vertices, num_of_priorities, outdegree);

    // Check for existing variables.
    if manager_ref.with_manager_shared(|manager| manager.num_vars()) != 0 {
        return Err("BDD manager must not contain any variables yet".into());
    }

    // Create random feature variables.
    let variables = manager_ref.with_manager_exclusive(|manager| -> Result<Vec<BDDFunction>, MercError> {
        Ok(manager
            .add_vars(number_of_variables)
            .map(|i| BDDFunction::var(manager, i))
            .collect::<Result<Vec<_>, _>>()?)
    })?;

    // Overall configuration is the conjunction of all features (i.e., all features enabled).
    let configuration = random_bdd(manager_ref, rng, &variables, 10)?;

    // Create random edge configurations.
    let mut edges_configuration: Vec<BDDFunction> = Vec::with_capacity(pg.num_of_edges());
    for _ in 0..pg.num_of_edges() {
        edges_configuration.push(random_bdd(manager_ref, rng, &variables, 10)?);
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
    use merc_symbolic::BDD_CACHE_CAPACITY;
    use merc_symbolic::BDD_NODE_CAPACITY;
    use merc_utilities::random_test;

    use crate::PG;
    use crate::random_parity_game;
    use crate::random_variability_parity_game;

    #[test]
    #[cfg_attr(miri, ignore)] // bitvec does not work with miri
    fn test_random_parity_game() {
        random_test(100, |rng| {
            let pg = random_parity_game(rng, false, 10, 5, 3);
            assert_eq!(pg.num_of_vertices(), 10);
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // bitvec does not work with miri
    fn test_random_total_parity_game() {
        random_test(100, |rng| {
            let pg = random_parity_game(rng, true, 10, 5, 3);
            assert_eq!(pg.num_of_vertices(), 10);
            assert!(pg.is_total(), "Generated parity game should be total");
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_variability_parity_game() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let vpg = random_variability_parity_game(&manager_ref, rng, false, 6, 5, 2, 3).unwrap();
            assert_eq!(vpg.num_of_vertices(), 6);
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_total_variability_parity_game() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let vpg = random_variability_parity_game(&manager_ref, rng, true, 6, 5, 2, 3).unwrap();
            assert!(
                vpg.num_of_vertices() >= 6 && vpg.num_of_vertices() <= 8,
                "At least 6 vertices, with 2 additional vertices for totality"
            );
            assert!(
                vpg.is_vpg_total(&manager_ref).unwrap(),
                "Generated variability parity game should be total"
            );
        })
    }
}
