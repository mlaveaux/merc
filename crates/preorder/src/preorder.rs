use clap::ValueEnum;
use merc_lts::LTS;
use merc_utilities::Timing;

use crate::ExplorationStrategy;
use crate::is_failures_refinement;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum RefinementType {
    Trace,
}

pub fn is_refinement<L: LTS>(impl_lts: L, spec_lts: L, preorder: RefinementType, timing: &mut Timing) -> bool {
    match preorder {
        RefinementType::Trace => is_failures_refinement::<L, false>(
            impl_lts,
            spec_lts,
            RefinementType::Trace,
            ExplorationStrategy::BFS,
            true,
            timing,
        ),
    }
}
