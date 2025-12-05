use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilder;
use merc_lts::StateIndex;
use merc_utilities::Timing;

use crate::Equivalence;
use crate::Partition;
use crate::branching_bisim_sigref;
use crate::branching_bisim_sigref_naive;
use crate::reduce;
use crate::strong_bisim_sigref;
use crate::strong_bisim_sigref_naive;
use crate::weak_bisim_sigref_naive;
use crate::weak_bisimulation;

// Compare two LTSs for equivalence using the given algorithm.
pub fn compare_lts(
    equivalence: Equivalence,
    left: LabelledTransitionSystem,
    right: &impl LTS,
    timing: &mut Timing,
) -> bool {
    let mut time_merge = timing.start("merge lts");
    let (merged, offset) = left.merge(right);
    time_merge.finish();

    // Returns the original
    let initial_state = merged.initial_state_index();

    // Reduce the merged LTS modulo the given equivalence and return the partition
    match equivalence {
        Equivalence::WeakBisim => {
            let (lts, partition) = weak_bisimulation(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
        Equivalence::WeakBisimSigref => {
            let (lts, partition) = weak_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
        Equivalence::StrongBisim => {
            let (lts, partition) = strong_bisim_sigref(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
        Equivalence::StrongBisimNaive => {
            let (lts, partition) = strong_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
        Equivalence::BranchingBisim => {
            let (lts, partition) = branching_bisim_sigref(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
        Equivalence::BranchingBisimNaive => {
            let (lts, partition) = branching_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(offset)
        }
    }
}
