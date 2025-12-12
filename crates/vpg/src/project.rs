use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::bdd::BDDFunction;

use crate::PG;
use crate::ParityGame;
use crate::VariabilityParityGame;

/// Projects a variability parity game into a standard parity game by removing
/// edges that are not enabled by the given feature selection.
pub fn project_variability_parity_game(
    vpg: &VariabilityParityGame,
    feature_selection: &BDDFunction,
) -> Result<ParityGame, MercError> {
    let mut edges = Vec::new();

    for v in vpg.iter_vertices() {
        for edge in vpg.outgoing_conf_edges(v) {
            if !feature_selection.and(&edge.configuration())?.satisfiable() {
                edges.push((v, edge.to()));
            }
        }
    }

    Ok(ParityGame::from_edges(
        vpg.initial_vertex(),
        vpg.owners().clone(),
        vpg.priorities().clone(),
        || edges.iter().cloned(),
    ))
}

#[cfg(test)]
mod tests {
    
}