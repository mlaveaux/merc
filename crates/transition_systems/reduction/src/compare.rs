#![forbid(unsafe_code)]

use log::warn;
use merc_lts::LTS;
use merc_utilities::Timing;

use crate::DistinguishingFormula;
use crate::Equivalence;
use crate::Partition;
use crate::branching_bisim_sigref;
use crate::branching_bisim_sigref_naive;
use crate::strong_bisim_sigref;
use crate::strong_bisim_sigref_naive;
use crate::strong_bisim_sigref_naive_with_counterexample;
use crate::weak_bisim_sigref_inductive_naive;
use crate::weak_bisim_sigref_naive;
use crate::weak_bisimulation;
use crate::weak_bisimulation_parallel;

/// Compares two LTSs for equivalence using the given algorithm.
///
/// When `counter_example` is set, a `false` result comes with a minimal-depth
/// distinguishing formula that holds in the left LTS but not in the right one.
///
/// Counter example generation is only supported for naive strong bisimulation
/// (StrongBisimNaive).
pub fn compare_lts<L: LTS>(
    equivalence: Equivalence,
    left: L,
    right: L,
    preprocess: bool,
    counter_example: bool,
    timing: &Timing,
) -> (bool, Option<DistinguishingFormula<L::Label>>) {
    let (merged, rhs_initial) = timing.measure("merge lts", || left.merge_disjoint(&right));
    drop(right); // No longer needed.

    if counter_example {
        match equivalence {
            Equivalence::StrongBisimNaive => {
                let (lts, partition, tree) = strong_bisim_sigref_naive_with_counterexample(merged, timing);
                let lhs_initial = lts.initial_state_index();

                return if partition.block_number(lhs_initial) == partition.block_number(rhs_initial) {
                    (true, None)
                } else {
                    let formula = tree
                        .distinguish(lhs_initial, rhs_initial, lts.labels())
                        .expect("states in different blocks under strong bisimulation must be distinguishable");
                    (false, Some(formula))
                };
            }
            _ => {
                warn!(
                    "Counter-example generation is only supported for naive strong bisimulation (StrongBisimNaive); comparing {equivalence:?} without one."
                );
            }
        }
    }

    // Reduce the merged LTS modulo the given equivalence and return the partition
    let equal = match equivalence {
        Equivalence::WeakBisim => {
            let (lts, rhs_initial, partition) = weak_bisimulation(merged, rhs_initial, preprocess, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimParallel => {
            let (lts, rhs_initial, partition) =
                weak_bisimulation_parallel(merged, rhs_initial, preprocess, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimDivergencePreserving => {
            let (lts, rhs_initial, partition) = weak_bisimulation(merged, rhs_initial, preprocess, true, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimParallelDivergencePreserving => {
            let (lts, rhs_initial, partition) =
                weak_bisimulation_parallel(merged, rhs_initial, preprocess, true, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimSigref => {
            let (lts, rhs_initial, partition) =
                weak_bisim_sigref_inductive_naive(merged, rhs_initial, preprocess, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimSigrefNaive => {
            let (lts, rhs_initial, partition) = weak_bisim_sigref_naive(merged, rhs_initial, preprocess, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimSigrefDivergencePreserving => {
            let (lts, rhs_initial, partition) =
                weak_bisim_sigref_inductive_naive(merged, rhs_initial, preprocess, true, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::WeakBisimSigrefNaiveDivergencePreserving => {
            let (lts, rhs_initial, partition) = weak_bisim_sigref_naive(merged, rhs_initial, preprocess, true, timing);
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
            let (lts, rhs_initial, partition) = branching_bisim_sigref(merged, rhs_initial, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::BranchingBisimNaive => {
            let (lts, rhs_initial, partition) = branching_bisim_sigref_naive(merged, rhs_initial, false, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::BranchingBisimDivergencePreserving => {
            let (lts, rhs_initial, partition) = branching_bisim_sigref(merged, rhs_initial, true, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
        Equivalence::BranchingBisimDivergencePreservingNaive => {
            let (lts, rhs_initial, partition) = branching_bisim_sigref_naive(merged, rhs_initial, true, timing);
            partition.block_number(lts.initial_state_index()) == partition.block_number(rhs_initial)
        }
    };

    (equal, None)
}
