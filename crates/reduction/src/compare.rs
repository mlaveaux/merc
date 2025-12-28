#![forbid(unsafe_code)]

use merc_lts::LTS;
use merc_utilities::Timing;

use crate::Equivalence;
use crate::Partition;
use crate::branching_bisim_sigref;
use crate::branching_bisim_sigref_naive;
use crate::strong_bisim_sigref;
use crate::strong_bisim_sigref_naive;
use crate::weak_bisim_sigref_naive;
use crate::weak_bisimulation;

// Compare two LTSs for equivalence using the given algorithm.
pub fn compare_lts<L: LTS>(equivalence: Equivalence, left: L, right: L, timing: &mut Timing) -> bool {
    let mut time_merge = timing.start("merge lts");
    let (merged, rhs_initial) = left.merge_disjoint(&right);
    drop(right); // No longer needed.
    time_merge.finish();

    // Reduce the merged LTS modulo the given equivalence and return the partition
    match equivalence {
        Equivalence::WeakBisim => {
            let (lts, partition) = weak_bisimulation(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimSigref => {
            let (lts, partition) = weak_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::StrongBisim => {
            let (lts, partition) = strong_bisim_sigref(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::StrongBisimNaive => {
            let (lts, partition) = strong_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::BranchingBisim => {
            let (lts, partition) = branching_bisim_sigref(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::BranchingBisimNaive => {
            let (lts, partition) = branching_bisim_sigref_naive(merged, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
    }
}
