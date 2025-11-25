use std::fmt;

use clap::ValueEnum;
use merc_lts::{LTS, LabelledTransitionSystem};
use merc_utilities::Timing;

use crate::{branching_bisim_sigref, branching_bisim_sigref_naive, quotient_lts_block, quotient_lts_naive, strong_bisim_sigref, strong_bisim_sigref_naive, weak_bisim_sigref_naive};


#[derive(Clone, Debug, ValueEnum)]
pub enum Equivalence {
    WeakBisim,
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
    match equivalence {
        Equivalence::WeakBisim => {
            let partition = weak_bisim_sigref_naive(lts.clone(), timing);
            quotient_lts_naive(&lts, &partition, true)
        }
        Equivalence::StrongBisim => {
            let (lts, partition) = strong_bisim_sigref(lts, timing);
            quotient_lts_block::<false>(&lts, &partition)
        }
        Equivalence::StrongBisimNaive => {
            let (lts, partition) = strong_bisim_sigref_naive(lts, timing);
            quotient_lts_naive(&lts, &partition, false)
        }
        Equivalence::BranchingBisim => {
            let (lts, partition) = branching_bisim_sigref(lts, timing);
            quotient_lts_block::<true>(&lts, &partition)
        }
        Equivalence::BranchingBisimNaive => {
            let (lts, partition) = branching_bisim_sigref_naive(lts, timing);
            quotient_lts_naive(&lts, &partition, true)
        }
    }
}