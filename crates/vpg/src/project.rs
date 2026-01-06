use merc_utilities::Timing;
use oxidd::BooleanFunction;
use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

use merc_symbolic::CubeIterAll;
use merc_utilities::MercError;

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
            // Check if the edge is enabled by the feature selection, if so, include it.
            if feature_selection.and(edge.configuration())?.satisfiable() {
                edges.push((v, edge.to()));
            }
        }
    }

    Ok(ParityGame::from_edges(
        vpg.initial_vertex(),
        vpg.owners().clone(),
        vpg.priorities().clone(),
        true, // It can be that after removing edges the result is not a total parity game.
        || edges.iter().cloned(),
    ))
}

/// Projects all configurations of a variability parity game into standard parity games.
pub fn project_variability_parity_games_iter<'a>(
    vpg: &'a VariabilityParityGame,
    timing: &'a Timing,
) -> impl Iterator<Item = Result<((Vec<OptBool>, BDDFunction, ParityGame), &'a Timing), MercError>> {
    CubeIterAll::new(vpg.variables(), vpg.configuration()).map(move |cube| {
        let (cube, bdd) = cube?;

        let mut time_proj = timing.start("project");
        let pg = project_variability_parity_game(vpg, &bdd)?;
        time_proj.finish();

        Ok(((cube, bdd, pg), timing))
    })
}
