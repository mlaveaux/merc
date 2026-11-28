//! Authors: Maurice Laveaux, Eduardo Costa Martins
//!
//! Implements the weak bisimulation algorithm by Eduardo Costa Martins.
#![forbid(unsafe_code)]

use std::iter;

use bitvec::bitvec;
use bitvec::order::Lsb0;

use bitvec::vec::BitVec;
use log::info;
use log::trace;
use merc_collections::BlockIndex;
use merc_io::TimeProgress;
use merc_lts::IncomingTransitions;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use merc_utilities::Timing;

use crate::DivergencePreservingLts;
use crate::Equivalence;
use crate::MarkedBlockPartition;
use crate::reduce_lts;
use crate::tau_cycle_elimination_and_reorder;

/// Type alias because we use bitvec for marking states
type BitArray = BitVec<u64, Lsb0>;

/// Apply weak bisimulation reduction
///
/// # Details
///
/// The `preprocess` flag indicates whether to preprocess the LTS using
/// branching bisimulation.
///
/// The `state` is any state for which we return the equivalent state in the
/// preprocessed LTS.
pub(crate) fn weak_bisimulation<L: LTS>(
    lts: L,
    state: StateIndex,
    preprocess: bool,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, MarkedBlockPartition) {
    // Preprocess the LTS if desired.
    if preprocess {
        let lts = timing.measure("preprocess", || {
            if divergence_preserving {
                reduce_lts(lts, Equivalence::BranchingBisimDivergencePreserving, true, timing)
            } else {
                reduce_lts(lts, Equivalence::BranchingBisim, true, timing)
            }
        });
        weak_bisimulation_preprocessed(lts, state, divergence_preserving, timing)
    } else {
        weak_bisimulation_preprocessed(lts, state, divergence_preserving, timing)
    }
}

/// Apply weak bisimulation reduction using the parallel variant of the
/// algorithm
///
/// # Details
///
/// The `preprocess` flag indicates whether to preprocess the LTS using
/// branching bisimulation.
///
/// The `state` is any state for which we return the equivalent state in the
/// preprocessed LTS.
pub(crate) fn weak_bisimulation_parallel<L: LTS>(
    lts: L,
    state: StateIndex,
    preprocess: bool,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, MarkedBlockPartition) {
    // Preprocess the LTS if desired.
    if preprocess {
        let lts = timing.measure("preprocess", || {
            if divergence_preserving {
                reduce_lts(lts, Equivalence::BranchingBisimDivergencePreserving, true, timing)
            } else {
                reduce_lts(lts, Equivalence::BranchingBisim, true, timing)
            }
        });
        weak_bisimulation_parallel_preprocessed(lts, state, divergence_preserving, timing)
    } else {
        weak_bisimulation_parallel_preprocessed(lts, state, divergence_preserving, timing)
    }
}

/// Core weak bisimulation algorithm implementation.
fn weak_bisimulation_preprocessed<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, MarkedBlockPartition) {
    let (tau_loop_free_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });

    timing.measure("reduction", || {
        let blocks = if divergence_preserving {
            let divergence_preserving_lts = DivergencePreservingLts::new(&tau_loop_free_lts);
            weak_bisimulation_impl(&divergence_preserving_lts)
        } else {
            weak_bisimulation_impl(&tau_loop_free_lts)
        };
        (tau_loop_free_lts, mapped_state, blocks)
    })
}

/// The implementation of [weak_bisimulation_parallel] that assumes the LTS has already been preprocessed.
fn weak_bisimulation_parallel_preprocessed<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, MarkedBlockPartition) {
    let (tau_loop_free_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });

    timing.measure("reduction", || {
        let blocks = if divergence_preserving {
            let divergence_preserving_lts = DivergencePreservingLts::new(&tau_loop_free_lts);
            weak_bisimulation_parallel_impl(&divergence_preserving_lts)
        } else {
            weak_bisimulation_parallel_impl(&tau_loop_free_lts)
        };
        (tau_loop_free_lts, mapped_state, blocks)
    })
}

/// The implementation of [weak_bisimulation].
fn weak_bisimulation_impl<L: LTS>(lts: &L) -> MarkedBlockPartition {
    let mut blocks = MarkedBlockPartition::new(lts.num_of_states());

    let mut act_mark = bitvec![u64, Lsb0; 0; lts.num_of_states()];
    let mut tau_mark = bitvec![u64, Lsb0; 0; lts.num_of_states()];

    let incoming = IncomingTransitions::new(lts);

    let progress = TimeProgress::new(
        |num_of_blocks: usize| {
            info!("Found {} blocks...", num_of_blocks);
        },
        1,
    );

    loop {
        let mut stable = true;
        for block_index in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
            progress.print(blocks.num_of_blocks());
            if *blocks.block(block_index).annotation() {
                continue;
            }

            trace!("Stabilising block {:?}", block_index);
            stable = false;
            blocks.mark_block_stable(block_index);

            // tau is the first label.
            for label in lts.labels().iter().enumerate().map(|(i, _)| LabelIndex::new(i)) {
                compute_weak_act(
                    &mut act_mark,
                    &mut tau_mark,
                    lts,
                    &blocks,
                    &incoming,
                    block_index,
                    label,
                );

                // Note that we cannot use the block references here, and instead uses indices, because stabilise
                // also modifies the blocks structure.
                for block_prime in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
                    stabilise(block_prime, &mut act_mark, &mut blocks);
                }
            }
        }

        if stable {
            trace!("Partition is stable!");
            break;
        }
    }

    blocks
}

/// The implementation of [weak_bisimulation_parallel].
fn weak_bisimulation_parallel_impl<L: LTS>(lts: &L) -> MarkedBlockPartition {
    let progress = TimeProgress::new(
        |num_of_blocks: usize| {
            info!("Found {} blocks...", num_of_blocks);
        },
        1,
    );

    let mut blocks = MarkedBlockPartition::new(lts.num_of_states());

    // Represents the s.marked[a] from the pseudocode.
    let mut marked = Vec::from_iter(iter::repeat_n(
        bitvec![u64, Lsb0; 0; lts.labels().len()],
        lts.num_of_states(),
    ));

    let incoming = IncomingTransitions::new(lts);

    loop {
        let mut stable = true;
        for block_index in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
            progress.print(blocks.num_of_blocks());
            if *blocks.block(block_index).annotation() {
                continue;
            }

            trace!("Stabilising block {:?}", block_index);
            stable = false;
            blocks.mark_block_stable(block_index);

            // marked := 0 for all states and actions
            for act_mark in &mut marked {
                act_mark.fill(false);
            }

            compute_weak_acts(&mut marked, lts, &incoming, &blocks, block_index);

            while let Some(label) = find_act(lts, &blocks, &mut marked) {
                for block_index in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
                    stabilise_act(block_index, label, &mut marked, &mut blocks);
                }
            }
        }

        if stable {
            trace!("Partition is stable!");
            break;
        }
    }

    blocks
}

/// Sets s.act_mark to true iff exists t: S. s =\not{a}=> t
/// If a = tau, then also updates s.tau_mark
fn compute_weak_act<L: LTS>(
    act_mark: &mut BitArray,
    tau_mark: &mut BitArray,
    lts: &L,
    blocks: &MarkedBlockPartition,
    incoming: &IncomingTransitions,
    block: BlockIndex,
    label: LabelIndex,
) {
    for s in lts.iter_states() {
        // s.act_mark := true iff s in B && a == tau
        act_mark.set(
            *s,
            lts.is_hidden_label(label) && blocks.iter_block(block).any(|state| state == *s),
        );

        for transition in lts.outgoing_transitions(s) {
            if transition.label == label {
                // s.act_mark := true iff a != tau && tau_mark[t]
                if !lts.is_hidden_label(transition.label) && tau_mark[*transition.to] {
                    act_mark.set(*s, true);
                }
            }
        }
    }

    for t in lts.iter_states() {
        // t.tau_mark := t.act_mark if a == tau
        if lts.is_hidden_label(label) {
            tau_mark.set(*t, act_mark[*t]);
        }

        if act_mark[*t] {
            for transition in incoming.incoming_silent_transitions(t) {
                act_mark.set(*transition.from, true);
            }
        }
    }
}

/// Computing weak reachability for all actions at once. The `marked` array contains |Act| entries per state.
///
/// # Details
///
/// Requires s.tau_mark iff s ->> B.
/// For all a in A sets s.marked\[a\] iff s =\[a\]> B.
///
/// Note that `B` is only used for debugging checks, and is not used in the actual algorithm.
fn compute_weak_acts<L: LTS>(
    marked: &mut [BitArray],
    lts: &L,
    incoming: &IncomingTransitions<'_>,
    blocks: &MarkedBlockPartition,
    block: BlockIndex,
) {
    if cfg!(debug_assertions) {
        // Check that compute_weak_act results in the same markings as the optimised compute_weak_acts procedure.

        // Determine the tau_mark first, the act_mark result is ignored.
        let mut tau_mark = bitvec![u64, Lsb0; 0; lts.num_of_states()];
        for s in lts.iter_states() {
            tau_mark.set(*s, marked[*s][0]);
        }

        // Determine the act_mark for every label that is not tau
        let act_mark = (0..lts.labels().len())
            .map(|label| {
                let mut act_mark = bitvec![u64, Lsb0; 0; lts.num_of_states()];

                for s in lts.iter_states() {
                    act_mark.set(*s, marked[*s][label]);
                }

                compute_weak_act(
                    &mut act_mark,
                    &mut tau_mark,
                    lts,
                    blocks,
                    incoming,
                    block,
                    LabelIndex::new(label),
                );
                act_mark
            })
            .collect::<Vec<_>>();

        // Compute the markings using the optimised procedure.
        compute_weak_acts_inner(marked, lts, incoming, blocks, block);

        // Check that the markings are the same for all labels, except tau
        for label in 1..lts.labels().len() {
            // The act_mark array starts at the first action, because we skip the tau action (index 0).
            debug_assert!(
                act_mark[label].iter().zip(marked.iter()).all(|(a, m)| a == m[label]),
                "The act mark should be the same as the corresponding column in marked"
            );
        }
    } else {
        // No checking for correctness.
        compute_weak_acts_inner(marked, lts, incoming, blocks, block);
    }
}

/// The inner implementation of [compute_weak_acts]. For all action a, sets s.marked\[a\] iff s =\[a\]> B, where B is the given block.
///
/// # Details
///
/// Requires that marked = 0 for all states.
fn compute_weak_acts_inner<L: LTS>(
    marked: &mut [BitArray],
    lts: &L,
    incoming: &IncomingTransitions<'_>,
    blocks: &MarkedBlockPartition,
    block: BlockIndex,
) {
    debug_assert!(
        marked.iter().all(|m| m.not_any()),
        "The marked array should be empty when calling compute_weak_acts_inner"
    );

    // TODO: This should probably not be hardcoded.
    let tau_index = LabelIndex::new(0);
    debug_assert!(
        lts.is_hidden_label(tau_index),
        "The first label should be the tau action"
    );

    // For t in B: t.marked[tau] := true
    for t in blocks.iter_block(block) {
        marked[t].set(*tau_index, true);
    }

    for t in lts.iter_states() {
        if marked[*t][*tau_index] {
            // For each s -[tau]-> t do
            for transition in incoming.incoming_silent_transitions(t) {
                // s.marked[tau] := True if t.marked[tau]
                marked[transition.from].set(*tau_index, true);
            }
        }
    }

    for t in lts.iter_states() {
        // For each t -[a]-> u do
        for transition in lts.outgoing_transitions(t) {
            // If u.marked[tau] then t.marked[a] := true
            if marked[*transition.to][*tau_index] {
                marked[*t].set(*transition.label, true);
            }
        }

        // For each s -[tau]-> t do
        for transition in incoming.incoming_silent_transitions(t) {
            // Computes s.marked[a] := s.marked[a] | t.marked[a] in place.
            let [marked_s, marked_t] = marked
                .get_disjoint_mut([*transition.from, *t])
                .expect("The indices are disjoint");
            let marked_t_raw = marked_t.as_raw_slice();
            for (s, t) in marked_s.as_raw_mut_slice().iter_mut().zip(marked_t_raw) {
                *s |= *t;
            }
        }
    }
}

/// Finding an action that can be used to perform a refinement step.
fn find_act<L: LTS>(_lts: &L, blocks: &MarkedBlockPartition, marked: &mut [BitArray]) -> Option<LabelIndex> {
    for block in (0..blocks.num_of_blocks()).map(BlockIndex::new) {
        // Pick a representative state s from the block
        let s = blocks.iter_block(block).next().expect("Block is non-empty");
        for t in blocks.iter_block(block) {
            if marked[s] != marked[t] {
                // Find an action a such that s.marked[a] != t.marked[a]
                for (label, marked_s) in marked[s].iter().enumerate() {
                    if marked_s != marked[t][label] {
                        return Some(LabelIndex::new(label));
                    }
                }
            }
        }
    }

    None
}

/// Splits the given block according to the given marking.
fn stabilise(block: BlockIndex, act_mark: &mut BitArray, blocks: &mut MarkedBlockPartition) {
    blocks.split_block(block, |state| act_mark[*state]);
}

/// Splits the given block according to the given marking.
fn stabilise_act(block: BlockIndex, act: LabelIndex, marked: &mut [BitArray], blocks: &mut MarkedBlockPartition) {
    blocks.split_block(block, |state| marked[*state][*act]);
}
