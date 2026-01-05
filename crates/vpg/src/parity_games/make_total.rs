use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_symbolic::minus;

use crate::PG;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VertexIndex;

/// Makes the given variability parity game total by adding edges to true/false nodes as needed.
pub fn make_vpg_total(
    manager_ref: &BDDManagerRef,
    vpg: &VariabilityParityGame,
) -> Result<VariabilityParityGame, MercError> {
    // The universe for totality is the game's overall configuration, not global true.
    let universe = manager_ref.with_manager_shared(|manager| BDDFunction::t(manager));

    // For a total game we need to potentially add new edges to true/false nodes.
    let mut edges = Vec::new();

    // Add the true and false nodes.
    let mut owners = vpg.owners().clone();
    let mut priorities = vpg.priorities().clone();

    // Owner does not matter, priority must be even for true node and odd for false node.
    let true_node = VertexIndex::new(owners.len());
    owners.push(Player::Even);
    priorities.push(Priority::new(0)); // Even priority for true node

    let false_node = VertexIndex::new(owners.len());
    owners.push(Player::Even);
    priorities.push(Priority::new(1)); // Odd priority for false node

    edges.push((true_node, universe.clone(), true_node)); // Self-loop on true node
    edges.push((false_node, universe.clone(), false_node)); // Self-loop on false node

    for vertex in vpg.iter_vertices() {
        let mut all_outgoing = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
        for edge in vpg.outgoing_conf_edges(vertex) {
            // Add a new edge with a random configuration.
            edges.push((vertex, edge.configuration().clone(), edge.to()));

            // Keep track of the overall outgoing configuration.
            all_outgoing = all_outgoing.or(edge.configuration())?;
        }

        // Missing configurations are those in the universe not covered by any outgoing edge.
        let missing = minus(&universe, &all_outgoing)?;
        if missing.satisfiable() {
            if owners[*vertex] == Player::Even {
                // Even player: add edge to true node for the remaining configurations.
                edges.push((vertex, universe.clone(), true_node));
            } else {
                // Odd player: add edge to false node for the remaining configurations.
                edges.push((vertex, universe.clone(), false_node));
            }
        }
    }

    Ok(VariabilityParityGame::from_edges(
        manager_ref,
        vpg.initial_vertex(),
        owners,
        priorities,
        vpg.configuration().clone(),
        vpg.variables().clone(),
        || edges.iter().cloned(),
    ))
}
