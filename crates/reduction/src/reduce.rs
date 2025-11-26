use std::fmt;

use clap::ValueEnum;
use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_utilities::Timing;

use crate::branching_bisim_sigref;
use crate::branching_bisim_sigref_naive;
use crate::quotient_lts_block;
use crate::quotient_lts_naive;
use crate::strong_bisim_sigref;
use crate::strong_bisim_sigref_naive;
use crate::weak_bisim_sigref_naive;

#[derive(Clone, Debug, ValueEnum)]
pub enum Equivalence {
    WeakBisim,
    /// Various signature based reduction algorithms.
    WeakBisimSigref,
    StrongBisim,
    StrongBisimNaive,
    BranchingBisim,
    BranchingBisimNaive,
}

/// Reduces the given LTS modulo the given equivalence using signature refinement
pub fn reduce<L>(lts: L, equivalence: Equivalence, timing: &mut Timing) -> LabelledTransitionSystem
where
    L: LTS + Clone + fmt::Debug,
{
    let (result, mut timer) = match equivalence {
        Equivalence::WeakBisim => {
            let (lts, partition) = weak_bisim_sigref_naive(lts.clone(), timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_naive(&lts, &partition, true), quotient_time)
        }
        Equivalence::WeakBisimSigref => {
            let (lts, partition) = weak_bisim_sigref_naive(lts, timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_naive(&lts, &partition, true), quotient_time)
        }
        Equivalence::StrongBisim => {
            let (lts, partition) = strong_bisim_sigref(lts, timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_block::<false>(&lts, &partition), quotient_time)
        }
        Equivalence::StrongBisimNaive => {
            let (lts, partition) = strong_bisim_sigref_naive(lts, timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_naive(&lts, &partition, false), quotient_time)
        }
        Equivalence::BranchingBisim => {
            let (lts, partition) = branching_bisim_sigref(lts, timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_block::<true>(&lts, &partition), quotient_time)
        }
        Equivalence::BranchingBisimNaive => {
            let (lts, partition) = branching_bisim_sigref_naive(lts, timing);
            let quotient_time = timing.start("quotient");
            (quotient_lts_naive(&lts, &partition, true), quotient_time)
        }
    };

    timer.finish();
    result
}
