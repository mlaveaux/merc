//! Authors: Maurice Laveaux and Jan J. Martens

use std::fmt;
use std::mem::swap;

use bumpalo::Bump;
use log::debug;
use log::info;
use log::trace;
use merc_io::TimeProgress;
use merc_lts::IncomingTransitions;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

use merc_utilities::Timing;

use crate::BlockIndex;
use crate::BlockPartition;
use crate::BlockPartitionBuilder;
use crate::IndexedPartition;
use crate::Partition;
use crate::Signature;
use crate::SignatureBuilder;
use crate::branching_bisim_signature;
use crate::branching_bisim_signature_inductive;
use crate::branching_bisim_signature_sorted;
use crate::is_tau_hat;
use crate::preprocess_branching;
use crate::strong_bisim_signature;
use crate::weak_bisim_signature_sorted;
use crate::weak_bisim_signature_sorted_taus;

/// Computes a strong bisimulation partitioning using signature refinement
pub fn strong_bisim_sigref<L>(lts: L, timing: &mut Timing) -> (L, BlockPartition)
where
    L: LTS + Clone + fmt::Debug,
{
    let mut timepre = timing.start("preprocess");
    let incoming = IncomingTransitions::new(&lts);
    timepre.finish();

    let mut time = timing.start("reduction");
    let partition = signature_refinement::<_, _, _, false>(
        &lts,
        &incoming,
        |state_index, partition, _, builder| {
            strong_bisim_signature(state_index, &lts, partition, builder);
        },
        |_, _| None,
    );

    debug_assert_eq!(
        partition,
        strong_bisim_sigref_naive(lts.clone(), timing).1,
        "The resulting partition is not a valid strong bisimulation partition."
    );

    time.finish();
    (lts, partition)
}

/// Computes a strong bisimulation partitioning using signature refinement
pub fn strong_bisim_sigref_naive<L>(lts: L, timing: &mut Timing) -> (L, IndexedPartition)
where
    L: LTS + fmt::Debug,
{
    let mut time = timing.start("reduction");
    let partition = signature_refinement_naive::<_, _, false>(&lts, |state_index, partition, _, builder| {
        strong_bisim_signature(state_index, &lts, partition, builder);
    });

    time.finish();
    (lts, partition)
}

/// Computes a branching bisimulation partitioning using signature refinement
pub fn branching_bisim_sigref<L>(lts: L, timing: &mut Timing) -> (LabelledTransitionSystem, BlockPartition)
where
    L: LTS + fmt::Debug,
{
    let mut timepre = timing.start("preprocess");
    let preprocessed_lts = preprocess_branching(lts);
    let incoming = IncomingTransitions::new(&preprocessed_lts);
    timepre.finish();

    let mut time = timing.start("reduction");
    let mut expected_builder = SignatureBuilder::default();
    let mut visited = FxHashSet::default();
    let mut stack = Vec::new();

    let partition = signature_refinement::<_, _, _, true>(
        &preprocessed_lts,
        &incoming,
        |state_index, partition, state_to_key, builder| {
            branching_bisim_signature_inductive(state_index, &preprocessed_lts, partition, state_to_key, builder);

            // Compute the expected signature, only used in debugging.
            if cfg!(debug_assertions) {
                branching_bisim_signature(
                    state_index,
                    &preprocessed_lts,
                    partition,
                    &mut expected_builder,
                    &mut visited,
                    &mut stack,
                );
                let expected_result = builder.clone();

                let signature = Signature::new(builder);
                debug_assert_eq!(
                    signature.as_slice(),
                    expected_result,
                    "The sorted and expected signature should be the same"
                );
            }
        },
        |signature, key_to_signature| {
            // Inductive signatures.
            for (label, key) in signature.iter().rev() {
                if is_tau_hat(*label, &preprocessed_lts)
                    && key_to_signature[*key].is_subset_of(signature, (*label, *key))
                {
                    return Some(*key);
                }

                if !is_tau_hat(*label, &preprocessed_lts) {
                    return None;
                }
            }

            None
        },
    );

    debug_assert_eq!(
        partition,
        signature_refinement_naive::<_, _, false>(&preprocessed_lts, |state_index, partition, _, builder| {
            branching_bisim_signature(
                state_index,
                &preprocessed_lts,
                partition,
                builder,
                &mut visited,
                &mut stack,
            );
        }),
        "The resulting partition is not a branching bisimulation partition."
    );
    time.finish();

    // Combine the SCC partition with the branching bisimulation partition.
    (preprocessed_lts, partition)
}

/// Computes a branching bisimulation partitioning using signature refinement without dirty blocks.
pub fn branching_bisim_sigref_naive<L>(lts: L, timing: &mut Timing) -> (LabelledTransitionSystem, IndexedPartition)
where
    L: LTS + fmt::Debug,
{
    let mut timepre = timing.start("preprocess");
    let preprocessed_lts = preprocess_branching(lts);
    timepre.finish();

    let mut time = timing.start("reduction");
    let mut expected_builder = SignatureBuilder::default();
    let mut visited = FxHashSet::default();
    let mut stack = Vec::new();

    let partition = signature_refinement_naive::<_, _, false>(
        &preprocessed_lts,
        |state_index, partition, state_to_signature, builder| {
            branching_bisim_signature_sorted(state_index, &preprocessed_lts, partition, state_to_signature, builder);

            // Compute the expected signature, only used in debugging.
            if cfg!(debug_assertions) {
                branching_bisim_signature(
                    state_index,
                    &preprocessed_lts,
                    partition,
                    &mut expected_builder,
                    &mut visited,
                    &mut stack,
                );
                let expected_result = builder.clone();

                let signature = Signature::new(builder);
                debug_assert_eq!(
                    signature.as_slice(),
                    expected_result,
                    "The sorted and expected signature should be the same"
                );
            }
        },
    );
    time.finish();

    (preprocessed_lts, partition)
}

/// Computes a branching bisimulation partitioning using signature refinement without dirty blocks.
pub fn weak_bisim_sigref_naive<L>(lts: L, timing: &mut Timing) -> (LabelledTransitionSystem, IndexedPartition)
where
    L: LTS + fmt::Debug,
{
    let mut timepre = timing.start("preprocess");
    let preprocessed_lts = preprocess_branching(lts);
    timepre.finish();

    let mut time = timing.start("reduction");

    let partition = signature_refinement_naive::<_, _, true>(
        &preprocessed_lts,
        |state_index, partition, state_to_signature, builder| {
            weak_bisim_signature_sorted(state_index, &preprocessed_lts, partition, state_to_signature, builder)
        },
    );
    time.finish();

    (preprocessed_lts, partition)
}

/// General signature refinement algorithm that accepts an arbitrary signature
///
/// The signature function is called for each state and should fill the
/// signature builder with the signature of the state. It consists of the
/// current partition, the signatures per state for the next partition.
fn signature_refinement<F, G, L, const BRANCHING: bool>(
    lts: &L,
    incoming: &IncomingTransitions,
    mut signature: F,
    mut renumber: G,
) -> BlockPartition
where
    F: FnMut(StateIndex, &BlockPartition, &[BlockIndex], &mut SignatureBuilder),
    G: FnMut(&[(LabelIndex, BlockIndex)], &Vec<Signature>) -> Option<BlockIndex>,
    L: LTS + fmt::Debug,
{
    // Avoids reallocations when computing the signature.
    let mut arena = Bump::new();
    let mut builder = SignatureBuilder::default();
    let mut split_builder = BlockPartitionBuilder::default();

    // Put all the states in the initial partition { S }.
    let mut id: FxHashMap<Signature<'_>, BlockIndex> = FxHashMap::default();

    // Assigns the signature to each state.
    let mut partition = BlockPartition::new(lts.num_of_states());
    let mut state_to_key: Vec<BlockIndex> = Vec::new();
    state_to_key.resize_with(lts.num_of_states(), || BlockIndex::new(0));
    let mut key_to_signature: Vec<Signature> = Vec::new();

    // Refine partitions until stable.
    let mut iteration = 0usize;
    let mut states = Vec::new();

    // Used to keep track of dirty blocks.
    let mut worklist = vec![BlockIndex::new(0)];

    let mut progress = TimeProgress::new(
        |(iteration, blocks)| {
            info!("Iteration {iteration}, found {blocks} blocks...");
        },
        5,
    );

    while let Some(block_index) = worklist.pop() {
        // Clear the current partition to start the next blocks.
        id.clear();

        // Removes the existing signatures.
        key_to_signature.clear();

        // Safety: The current signatures have been removed, so it safe to reuse the memory.
        let id: &'_ mut FxHashMap<Signature<'_>, BlockIndex> = unsafe { std::mem::transmute(&mut id) };
        let key_to_signature: &'_ mut Vec<Signature<'_>> = unsafe { std::mem::transmute(&mut key_to_signature) };

        arena.reset();

        let block = partition.block(block_index);
        debug_assert!(
            block.has_marked(),
            "Every block in the worklist should have at least one marked state"
        );

        if BRANCHING {
            partition.mark_backward_closure(block_index, incoming);
        }

        // Blocks above this number are new in this iteration.
        let num_blocks = partition.num_of_blocks();

        // This is a workaround for a data race in bumpalo for zero-sized slices.
        let empty_slice: &[(LabelIndex, BlockIndex)] = &[];

        for new_block_index in
            partition.partition_marked_with(block_index, &mut split_builder, |state_index, partition| {
                signature(state_index, partition, &state_to_key, &mut builder);

                // Compute the signature of a single state
                let index = if let Some(key) = renumber(&builder, &key_to_signature) {
                    key
                } else {
                    if let Some((_, index)) = id.get_key_value(&Signature::new(&builder)) {
                        *index
                    } else {
                        let slice = if builder.len() == 0 { empty_slice } else { arena.alloc_slice_copy(&builder) };
                        let number = BlockIndex::new(key_to_signature.len());
                        id.insert(Signature::new(slice), number);
                        key_to_signature.push(Signature::new(slice));

                        number
                    }
                };

                // (branching) Keep track of the signature for every block in the next partition.
                state_to_key[state_index] = index;

                trace!("State {state_index} signature {builder:?} index {index}");
                index
            })
        {
            if block_index != new_block_index {
                // If this is a new block, mark the incoming states as dirty
                states.clear();
                states.extend(partition.iter_block(new_block_index));

                for &state_index in &states {
                    for transition in incoming.incoming_transitions(state_index) {
                        if BRANCHING {
                            // Mark incoming states into old blocks, or visible actions.
                            if !lts.is_hidden_label(transition.label)
                                || partition.block_number(transition.to) < num_blocks
                            {
                                let other_block = partition.block_number(transition.to);

                                if !partition.block(other_block).has_marked() {
                                    // If block was not already marked then add it to the worklist.
                                    worklist.push(other_block);
                                }

                                partition.mark_element(transition.to);
                            }
                        } else {
                            // In this case mark all incoming states.
                            let other_block = partition.block_number(transition.to);

                            if !partition.block(other_block).has_marked() {
                                // If block was not already marked then add it to the worklist.
                                worklist.push(other_block);
                            }

                            partition.mark_element(transition.to);
                        }
                    }
                }
            }
        }

        trace!("Iteration {iteration} partition {partition}");

        iteration += 1;

        progress.print((iteration, partition.num_of_blocks()));
    }

    trace!("Refinement partition {partition}");
    partition
}

/// General signature refinement algorithm that accepts an arbitrary signature
///
/// The signature function is called for each state and should fill the
/// signature builder with the signature of the state. It consists of the
/// current partition, the signatures per state for the next partition.
fn signature_refinement_naive<F, L, const WEAK: bool>(lts: &L, mut signature: F) -> IndexedPartition
where
    F: FnMut(StateIndex, &IndexedPartition, &Vec<Signature<'_>>, &mut SignatureBuilder),
    L: LTS + fmt::Debug,
{
    // Avoids reallocations when computing the signature.
    let mut arena = Bump::new();
    let mut builder = SignatureBuilder::default();

    // Put all the states in the initial partition { S }.
    let mut id: FxHashMap<Signature<'_>, BlockIndex> = FxHashMap::default();

    // Assigns the signature to each state.
    let mut partition = IndexedPartition::new(lts.num_of_states());
    let mut next_partition = IndexedPartition::new(lts.num_of_states());
    let mut state_to_signature: Vec<Signature<'_>> = Vec::new();
    state_to_signature.resize_with(lts.num_of_states(), Signature::default);

    // Refine partitions until stable.
    let mut old_count = 1;
    let mut iteration = 0;

    let mut progress = TimeProgress::new(
        |(iteration, blocks)| {
            debug!("Iteration {iteration}, found {blocks} blocks...",);
        },
        5,
    );

    // This is a workaround for a data race in bumpalo for zero-sized slices.
    let empty_slice: &[(LabelIndex, BlockIndex)] = &[];

    while old_count != id.len() {
        old_count = id.len();
        progress.print((iteration, old_count));
        swap(&mut partition, &mut next_partition);

        // Clear the current partition to start the next blocks.
        id.clear();

        state_to_signature.clear();
        state_to_signature.resize_with(lts.num_of_states(), Signature::default);

        // Safety: The current signatures have been removed, so it safe to reuse the memory.
        let id: &'_ mut FxHashMap<Signature<'_>, BlockIndex> = unsafe { std::mem::transmute(&mut id) };
        let state_to_signature: &mut Vec<Signature<'_>> = unsafe { std::mem::transmute(&mut state_to_signature) };

        // Remove the current signatures.
        arena.reset();

        if WEAK {
            for state_index in lts.iter_states() {
                weak_bisim_signature_sorted_taus(state_index, lts, &partition, &state_to_signature, &mut builder);

                trace!("State {state_index} signature {:?}", builder);

                // Keep track of the index for every state, either use the arena to allocate space or simply borrow the value.
                let slice = if builder.len() == 0 { empty_slice } else { arena.alloc_slice_copy(&builder) };
                state_to_signature[state_index] = Signature::new(slice);
            }
        }

        for state_index in lts.iter_states() {
            // Compute the signature of a single state
            signature(state_index, &partition, &state_to_signature, &mut builder);

            trace!("State {state_index} signature {builder:?}");

            // Keep track of the index for every state, either use the arena to allocate space or simply borrow the value.
            let mut new_id = BlockIndex::new(id.len());
            if let Some((signature, index)) = id.get_key_value(&Signature::new(&builder)) {
                state_to_signature[state_index] = Signature::new(signature.as_slice());
                new_id = *index;
            } else {
                let slice = if builder.len() == 0 { empty_slice } else { arena.alloc_slice_copy(&builder) };
                id.insert(Signature::new(slice), new_id);

                // (branching) Keep track of the signature for every block in the next partition.
                state_to_signature[state_index] = Signature::new(slice);
            }

            next_partition.set_block(state_index, new_id);
        }

        iteration += 1;

        debug_assert!(
            iteration <= lts.num_of_states().max(2),
            "There can never be more splits than number of states, but at least two iterations for stability"
        );
    }

    trace!("Refinement partition {partition}");
    debug_assert!(
        is_valid_refinement(lts, &partition, |state_index, partition, builder| signature(
            state_index,
            partition,
            &state_to_signature,
            builder
        )),
        "The resulting partition is not a valid partition."
    );
    partition
}

/// Returns true iff the given partition is a strong bisimulation partition
pub fn is_valid_refinement<F, P>(lts: &impl LTS, partition: &P, mut compute_signature: F) -> bool
where
    F: FnMut(StateIndex, &P, &mut SignatureBuilder),
    P: Partition,
{
    // Check that the partition is indeed stable and as such is a quotient of strong bisimulation
    let mut block_to_signature: Vec<Option<SignatureBuilder>> = vec![None; partition.num_of_blocks()];

    // Avoids reallocations when computing the signature.
    let mut builder = SignatureBuilder::default();

    for state_index in lts.iter_states() {
        let block = partition.block_number(state_index);

        // Compute the flat signature, which has Hash and is more compact.
        compute_signature(state_index, partition, &mut builder);
        let signature: Vec<(LabelIndex, BlockIndex)> = builder.clone();

        if let Some(block_signature) = &block_to_signature[block] {
            if signature != *block_signature {
                trace!(
                    "State {state_index} has a different signature {signature:?} then the block {block} which has signature {block_signature:?}"
                );
                return false;
            }
        } else {
            block_to_signature[block] = Some(signature);
        };
    }

    // Check if there are two blocks with the same signature
    let mut signature_to_block: FxHashMap<Signature, usize> = FxHashMap::default();

    for (block_index, signature) in block_to_signature
        .iter()
        .map(|signature| signature.as_ref().unwrap())
        .enumerate()
    {
        if let Some(other_block_index) = signature_to_block.get(&Signature::new(signature)) {
            if block_index != *other_block_index {
                trace!("Block {block_index} and {other_block_index} have the same signature {signature:?}");
                return false;
            }
        } else {
            signature_to_block.insert(Signature::new(signature), block_index);
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use merc_lts::random_lts;
    use merc_utilities::Timing;
    use merc_utilities::random_test;

    #[test]
    fn test_random_strong_bisim_sigref() {
        random_test(100, |rng| {
            let lts = random_lts(rng, 10, 3, 3);
            let mut timing = Timing::new();

            strong_bisim_sigref(lts, &mut timing);
        });
    }

    /// Checks that the branching bisimulation partition is a refinement of the strong bisimulation partition.
    fn is_refinement(
        lts: &LabelledTransitionSystem,
        strong_partition: &impl Partition,
        branching_partition: &impl Partition,
    ) {
        for state_index in lts.iter_states() {
            for other_state_index in lts.iter_states() {
                if strong_partition.block_number(state_index) == strong_partition.block_number(other_state_index) {
                    // If the states are together according to branching bisimilarity, then they should also be together according to strong bisimilarity.
                    assert_eq!(
                        branching_partition.block_number(state_index),
                        branching_partition.block_number(other_state_index),
                        "The branching partition should be a refinement of the strong partition, 
                        but states {state_index} and {other_state_index} are in different blocks"
                    );
                }
            }
        }
    }

    #[test]
    fn test_random_branching_bisim_sigref() {
        random_test(100, |rng| {
            let lts = random_lts(rng, 10, 3, 3);
            let mut timing = Timing::new();

            let (preprocessed_lts, branching_partition) = branching_bisim_sigref(lts, &mut timing);
            let strong_partition = strong_bisim_sigref(preprocessed_lts.clone(), &mut timing).1;
            is_refinement(&preprocessed_lts, &strong_partition, &branching_partition);
        });
    }

    #[test]
    fn test_random_branching_bisim_sigref_naive() {
        random_test(100, |rng| {
            let lts = random_lts(rng, 10, 3, 3);
            let mut timing = Timing::new();

            let (preprocessed_lts, branching_partition) = branching_bisim_sigref_naive(lts, &mut timing);
            let strong_partition = strong_bisim_sigref_naive(preprocessed_lts.clone(), &mut timing).1;
            is_refinement(&preprocessed_lts, &strong_partition, &branching_partition);
        });
    }
}
