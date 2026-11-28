//! Authors: Maurice Laveaux and Jan Martens.
use std::mem::swap;

use bumpalo::Bump;
use log::debug;
use log::info;
use log::log_enabled;
use log::trace;
use merc_io::TimeProgress;
use merc_lts::IncomingTransitions;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

use merc_collections::BlockIndex;
use merc_collections::IndexedPartition;
use merc_utilities::Timing;

use crate::BlockPartition;
use crate::BlockPartitionBuilder;
use crate::DivergencePreservingLts;
use crate::Partition;
use crate::PartitionTree;
use crate::RefinementForest;
use crate::Signature;
use crate::SignatureBuilder;
use crate::branching_bisim_signature;
use crate::branching_bisim_signature_inductive;
use crate::branching_bisim_signature_sorted;
use crate::is_tau_hat;
use crate::longest_tau_path;
use crate::quotient_lts_block;
use crate::strong_bisim_signature;
use crate::tau_cycle_elimination_and_reorder;
use crate::weak_bisim_presignature_sorted;
use crate::weak_bisim_signature_sorted;
use crate::weak_bisim_signature_sorted_full;
use crate::weak_bisim_signature_sorted_taus;

/// Computes a strong bisimulation partitioning using signature refinement
pub fn strong_bisim_sigref<L: LTS>(lts: L, timing: &Timing) -> (L, BlockPartition) {
    let incoming = timing.measure("preprocess", || IncomingTransitions::new(&lts));

    let partition = timing.measure("reduction", || {
        signature_refinement::<_, _, _, false>(
            &lts,
            &incoming,
            |state_index, partition, _, builder| {
                strong_bisim_signature(state_index, &lts, partition, builder);
            },
            |_, _| None,
        )
    });

    (lts, partition)
}

/// Computes a strong bisimulation partitioning using signature refinement
pub(crate) fn strong_bisim_sigref_naive<L: LTS>(lts: L, timing: &Timing) -> (L, IndexedPartition) {
    let partition = strong_bisim_sigref_naive_impl(&lts, &mut (), timing);
    (lts, partition)
}

/// Computes a strong bisimulation partitioning using signature refinement,
/// additionally returning the block-history tree recorded during refinement.
///
/// The tree can reconstruct a minimal-depth distinguishing formula for any
/// two states found in different blocks.
pub(crate) fn strong_bisim_sigref_naive_with_counterexample<L: LTS>(
    lts: L,
    timing: &Timing,
) -> (L, IndexedPartition, PartitionTree) {
    let mut tree = PartitionTree::new(lts.num_of_states());
    let partition = strong_bisim_sigref_naive_impl(&lts, &mut tree, timing);
    (lts, partition, tree)
}

/// Shared implementation of the naive strong bisimulation refinement.
fn strong_bisim_sigref_naive_impl<L: LTS, R: RefinementForest>(
    lts: &L,
    forest: &mut R,
    timing: &Timing,
) -> IndexedPartition {
    timing.measure("reduction", || {
        signature_refinement_naive::<_, _, _, false>(
            lts,
            |state_index, partition, _, builder| {
                strong_bisim_signature(state_index, lts, partition, builder);
            },
            forest,
        )
    })
}

/// Computes a branching bisimulation partitioning using signature refinement.
///
/// Internally preprocesses `lts` for tau-cycles, so the returned LTS and
/// `StateIndex` correspond to `state` after preprocessing, not before. When
/// `divergence_preserving` is true, computes divergence-preserving branching
/// bisimulation instead.
pub fn branching_bisim_sigref<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, BlockPartition) {
    let (preprocessed_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });

    let partition = if divergence_preserving {
        branching_bisim_sigref_impl(&DivergencePreservingLts::new(&preprocessed_lts), timing)
    } else {
        branching_bisim_sigref_impl(&preprocessed_lts, timing)
    };

    (preprocessed_lts, mapped_state, partition)
}

/// Implementation of [branching_bisim_sigref].
fn branching_bisim_sigref_impl<L: LTS>(preprocessed_lts: &L, timing: &Timing) -> BlockPartition {
    let incoming = timing.measure("preprocess", || IncomingTransitions::new(preprocessed_lts));

    if log_enabled!(log::Level::Debug) {
        let path = longest_tau_path(preprocessed_lts);
        debug!("longest_tau_path" = path.len(); "The longest tau path is {:?}", path);
    }

    let mut expected_builder = SignatureBuilder::default();
    let mut visited = FxHashSet::default();
    let mut stack = Vec::new();

    timing.measure("reduction", || {
        signature_refinement::<_, _, _, true>(
            preprocessed_lts,
            &incoming,
            |state_index, partition, state_to_key, builder| {
                branching_bisim_signature_inductive(state_index, preprocessed_lts, partition, state_to_key, builder);

                // Compute the expected signature, only used in debugging.
                if cfg!(debug_assertions) {
                    branching_bisim_signature(
                        state_index,
                        preprocessed_lts,
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
                    if is_tau_hat(*label, preprocessed_lts)
                        && key_to_signature[*key].is_subset_of(signature, (*label, *key))
                    {
                        return Some(*key);
                    }

                    if !is_tau_hat(*label, preprocessed_lts) {
                        return None;
                    }
                }

                None
            },
        )
    })
}

/// Computes a branching bisimulation partitioning using signature
/// refinement, without the process-the-smaller-half (dirty block)
/// optimisation.
///
/// Internally preprocesses `lts` for tau-cycles, so the returned LTS and
/// `StateIndex` correspond to `state` after preprocessing, not before. When
/// `divergence_preserving` is true, computes divergence-preserving branching
/// bisimulation instead.
pub(crate) fn branching_bisim_sigref_naive<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, IndexedPartition) {
    let (preprocessed_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });

    let partition = if divergence_preserving {
        branching_bisim_sigref_naive_impl(&DivergencePreservingLts::new(&preprocessed_lts), timing)
    } else {
        branching_bisim_sigref_naive_impl(&preprocessed_lts, timing)
    };

    (preprocessed_lts, mapped_state, partition)
}

/// Implementation of [branching_bisim_sigref_naive].
fn branching_bisim_sigref_naive_impl<L: LTS>(preprocessed_lts: &L, timing: &Timing) -> IndexedPartition {
    timing.measure("reduction", || {
        let mut expected_builder = SignatureBuilder::default();
        let mut visited = FxHashSet::default();
        let mut stack = Vec::new();

        signature_refinement_naive::<_, _, _, false>(
            preprocessed_lts,
            |state_index, partition, state_to_signature, builder| {
                branching_bisim_signature_sorted(state_index, preprocessed_lts, partition, state_to_signature, builder);

                // Compute the expected signature, only used in debugging.
                if cfg!(debug_assertions) {
                    branching_bisim_signature(
                        state_index,
                        preprocessed_lts,
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
            &mut (),
        )
    })
}

/// Computes a weak bisimulation partitioning using signature refinement with
/// inductive signatures, without the process-the-smaller-half (dirty block)
/// optimisation.
///
/// If `preprocess` is true, first reduces modulo branching bisimulation; the
/// returned LTS and `StateIndex` correspond to `state` after whatever
/// preprocessing occurred.
pub(crate) fn weak_bisim_sigref_inductive_naive<L: LTS>(
    lts: L,
    state: StateIndex,
    preprocess: bool,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, IndexedPartition) {
    // Preprocess the LTS if desired.
    if preprocess {
        let (preprocessed_lts, mapped_state, partition) =
            branching_bisim_sigref(lts, state, divergence_preserving, timing);
        let quotiented_state = StateIndex::new(*partition.block_number(mapped_state));
        let lts = timing.measure("quotient", || {
            quotient_lts_block::<_, true>(&preprocessed_lts, &partition, !divergence_preserving)
        });
        weak_bisim_sigref_inductive_naive_impl(lts, quotiented_state, divergence_preserving, timing)
    } else {
        weak_bisim_sigref_inductive_naive_impl(lts, state, divergence_preserving, timing)
    }
}

/// Implementation of [weak_bisim_sigref_inductive_naive] that deals with both preprocessed and regular LTSs.
///
/// The `state` is any state for which we return the equivalent state in the preprocessed LTS.
pub(crate) fn weak_bisim_sigref_inductive_naive_impl<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, IndexedPartition) {
    let (preprocessed_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });
    let partition = timing.measure("reduction", || {
        if divergence_preserving {
            signature_refinement_weak(&DivergencePreservingLts::new(&preprocessed_lts))
        } else {
            signature_refinement_weak(&preprocessed_lts)
        }
    });
    (preprocessed_lts, mapped_state, partition)
}

/// Computes a weak bisimulation partitioning using naive signature
/// refinement (no inductive signatures).
///
/// If `preprocess` is true, first reduces modulo branching bisimulation; the
/// returned LTS and `StateIndex` correspond to `state` after whatever
/// preprocessing occurred.
pub(crate) fn weak_bisim_sigref_naive<L: LTS>(
    lts: L,
    state: StateIndex,
    preprocess: bool,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, IndexedPartition) {
    // Preprocess the LTS if desired.
    if preprocess {
        let (preprocessed_lts, mapped_state, partition) =
            branching_bisim_sigref(lts, state, divergence_preserving, timing);
        let quotiented_state = StateIndex::new(*partition.block_number(mapped_state));
        let lts = timing.measure("quotient", || {
            quotient_lts_block::<_, true>(&preprocessed_lts, &partition, !divergence_preserving)
        });
        weak_bisim_sigref_naive_impl(lts, quotiented_state, divergence_preserving, timing)
    } else {
        weak_bisim_sigref_naive_impl(lts, state, divergence_preserving, timing)
    }
}

/// Implementation of [weak_bisim_sigref_naive] that deals with both
/// preprocessed and regular LTSs.
///
/// The `state` is any state for which we return the equivalent state in the
/// preprocessed LTS.
fn weak_bisim_sigref_naive_impl<L: LTS>(
    lts: L,
    state: StateIndex,
    divergence_preserving: bool,
    timing: &Timing,
) -> (LabelledTransitionSystem<L::Label>, StateIndex, IndexedPartition) {
    let (preprocessed_lts, mapped_state) = timing.measure("preprocess", || {
        tau_cycle_elimination_and_reorder(lts, state, !divergence_preserving)
    });
    let partition = timing.measure("reduction", || {
        if divergence_preserving {
            let divergence_preserving_lts = DivergencePreservingLts::new(&preprocessed_lts);
            signature_refinement_naive::<_, _, _, true>(
                &divergence_preserving_lts,
                |state_index, partition, state_to_signature, builder| {
                    weak_bisim_signature_sorted(
                        state_index,
                        &divergence_preserving_lts,
                        partition,
                        state_to_signature,
                        builder,
                    )
                },
                &mut (),
            )
        } else {
            signature_refinement_naive::<_, _, _, true>(
                &preprocessed_lts,
                |state_index, partition, state_to_signature, builder| {
                    weak_bisim_signature_sorted(state_index, &preprocessed_lts, partition, state_to_signature, builder)
                },
                &mut (),
            )
        }
    });

    (preprocessed_lts, mapped_state, partition)
}

/// Signature refinement algorithm that accepts an arbitrary signature and uses
/// process-the-smaller-half optimisation by marking dirty states.
///
/// The `signature` function is called for each state and should fill the
/// signature builder with the signature of the state.
///
/// The `renumber` function can be used to renumber the signatures, which is
/// used in inductive signatures.
///
/// If `BRANCHING` then incoming tau-paths are considered for marking the
/// incoming blocks. Furthermore, the signature function receives the
/// `state_to_key` mapping that contains the signature index for every state,
/// required for inductive signatures. And the signatures are computed in the
/// order of the given `lts`.
fn signature_refinement<F, G, L, const BRANCHING: bool>(
    lts: &L,
    incoming: &IncomingTransitions,
    mut signature: F,
    mut renumber: G,
) -> BlockPartition
where
    F: FnMut(StateIndex, &BlockPartition, &[BlockIndex], &mut SignatureBuilder),
    G: FnMut(&[(LabelIndex, BlockIndex)], &Vec<Signature>) -> Option<BlockIndex>,
    L: LTS,
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

    let progress = TimeProgress::new(
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

        // SAFETY: `id` was cleared above, so it holds no `Signature` borrowing
        // from the arena; the transmute only relaxes that borrow's lifetime so the
        // map can be refilled with slices allocated after the `arena.reset()`
        // below.
        let id: &mut FxHashMap<Signature<'_>, BlockIndex> = unsafe { std::mem::transmute(&mut id) };
        // SAFETY: see above; `key_to_signature` was cleared.
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
                let index = if let Some(key) = renumber(&builder, key_to_signature) {
                    key
                } else if let Some((_, index)) = id.get_key_value(&Signature::new(&builder)) {
                    *index
                } else {
                    let slice = if builder.is_empty() {
                        empty_slice
                    } else {
                        arena.alloc_slice_copy(&builder)
                    };
                    let number = BlockIndex::new(key_to_signature.len());
                    id.insert(Signature::new(slice), number);
                    key_to_signature.push(Signature::new(slice));

                    number
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
                                || partition.block_number(transition.from) < num_blocks
                            {
                                let other_block = partition.block_number(transition.from);

                                if !partition.block(other_block).has_marked() {
                                    // If block was not already marked then add it to the worklist.
                                    worklist.push(other_block);
                                }

                                partition.mark_element(transition.from);
                            }
                        } else {
                            // In this case mark all incoming states.
                            let other_block = partition.block_number(transition.from);

                            if !partition.block(other_block).has_marked() {
                                // If block was not already marked then add it to the worklist.
                                worklist.push(other_block);
                            }

                            partition.mark_element(transition.from);
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

/// Weak signature refinement algorithm, doing inductive signatures naively.
///
/// The signature function is called for each state and should fill the
/// signature builder with the pre_signature of the state.
fn signature_refinement_weak<L: LTS>(lts: &L) -> IndexedPartition {
    // Avoids reallocations when computing the signature.
    let mut arena = Bump::new();
    let mut builder = SignatureBuilder::default();

    // Put all the states in the initial partition { S }.
    let mut id: FxHashMap<Signature<'_>, BlockIndex> = FxHashMap::default();

    // Assigns the signature to each state.
    let mut partition = IndexedPartition::new(lts.num_of_states());
    let mut next_partition = IndexedPartition::new(lts.num_of_states());
    let mut state_to_signature: Vec<Option<usize>> = Vec::new();
    let mut key_to_signature: Vec<Signature> = Vec::new();
    let mut state_to_taus: Vec<Signature> = Vec::new();

    state_to_signature.resize_with(lts.num_of_states(), || None);
    state_to_taus.resize_with(lts.num_of_states(), Signature::default);

    let mut old_count = 1;
    let mut iteration = 0;

    let progress = TimeProgress::new(
        |(iteration, blocks)| {
            debug!("Iteration {iteration}, found {blocks} blocks...",);
        },
        5,
    );

    // This is a workaround for a data race in bumpalo for zero-sized slices.
    let empty_slice: &[(LabelIndex, BlockIndex)] = &[];
    // Refine partitions until stable.

    while old_count != id.len() {
        old_count = id.len();
        progress.print((iteration, old_count));
        swap(&mut partition, &mut next_partition);

        // Clear every collection that borrows from the arena *before* resetting
        // it, so that no `Signature` borrow can outlive the storage it points
        // into.
        id.clear();
        state_to_signature.clear();
        key_to_signature.clear();
        state_to_taus.clear();

        // Remove the current signatures; safe now that every borrowing collection
        // has been cleared above.
        arena.reset();

        state_to_signature.resize_with(lts.num_of_states(), || None);

        // SAFETY: `id` was cleared above, so it holds no `Signature` borrowing
        // from the arena; the transmute only relaxes that borrow's lifetime so the
        // map can be refilled with slices allocated from the freshly reset arena
        // during this iteration.
        let id: &'_ mut FxHashMap<Signature<'_>, BlockIndex> = unsafe { std::mem::transmute(&mut id) };
        // SAFETY: see above; `key_to_signature` was cleared.
        let key_to_signature: &'_ mut Vec<Signature<'_>> = unsafe { std::mem::transmute(&mut key_to_signature) };
        // SAFETY: see above; `state_to_taus` was cleared.
        let state_to_taus: &'_ mut Vec<Signature<'_>> = unsafe { std::mem::transmute(&mut state_to_taus) };

        // Compute for each state its tau signature. This seems inefficient, but for now it works.
        state_to_taus.resize_with(lts.num_of_states(), Signature::default);
        for state in lts.iter_states() {
            weak_bisim_signature_sorted_taus(state, lts, &partition, state_to_taus, &mut builder);

            let slice = if builder.is_empty() {
                empty_slice
            } else {
                arena.alloc_slice_copy(&builder)
            };
            state_to_taus[state] = Signature::new(slice);
        }

        for state_index in lts.iter_states() {
            // Compute the Presignature of a single state
            weak_bisim_presignature_sorted(
                state_index,
                lts,
                &partition,
                state_to_taus,
                &state_to_signature,
                &mut builder,
            );

            // Inductive step see if presig is a subset of a tau reachable state.
            let mut inductive_key = None;
            for keyvalue in builder.as_slice() {
                if is_tau_hat(keyvalue.0, lts) {
                    let tau_sig = &key_to_signature[keyvalue.1.value()];
                    let presig = Signature::new(&builder);

                    if tau_sig.is_subset_of(presig.as_slice(), *keyvalue) {
                        inductive_key = Some(*keyvalue.1);
                        break;
                    }
                }
            }
            if let Some(inductive_key) = inductive_key {
                trace!(
                    "State {state_index} with pre {:?} uses inductive key {inductive_key}:{:?}",
                    builder.as_slice(),
                    key_to_signature[inductive_key].as_slice()
                );
                state_to_signature[state_index] = Some(inductive_key);
                next_partition.set_block(*state_index, BlockIndex::new(inductive_key));
            } else {
                // If not: expand the signature completely.
                weak_bisim_signature_sorted_full(
                    state_index,
                    lts,
                    &partition,
                    state_to_taus,
                    &state_to_signature,
                    key_to_signature,
                    &mut builder,
                );
                trace!("State {state_index} final signature {:?}", builder.as_slice());

                // Keep track of the index for every state
                let mut new_id = BlockIndex::new(key_to_signature.len());
                if let Some((_signature, index)) = id.get_key_value(&Signature::new(&builder)) {
                    state_to_signature[state_index] = Some(index.value());
                    new_id = *index;
                } else {
                    let slice = if builder.is_empty() {
                        empty_slice
                    } else {
                        arena.alloc_slice_copy(&builder)
                    };
                    id.insert(Signature::new(slice), new_id);
                    key_to_signature.push(Signature::new(slice));

                    state_to_signature[state_index] = Some(new_id.value());
                }

                next_partition.set_block(*state_index, new_id);
            };
        }

        iteration += 1;

        debug_assert!(
            iteration <= lts.num_of_states().max(2),
            "There can never be more splits than number of states, but at least two iterations for stability"
        );
    }

    trace!("Refinement partition {partition}");
    partition
}

/// General signature refinement algorithm that accepts an arbitrary signature.
///
/// The `signature` closure receives the current partition and the per-state
/// signatures computed so far for the next partition, and fills the builder
/// with the signature of the given state.
///
/// The `forest` records the block-history of the refinement, which can later
/// be used to reconstruct a distinguishing formula for any two states in
/// different final blocks. Pass `&mut ()` when this is not needed; the no-op
/// implementation is free.
fn signature_refinement_naive<F, L: LTS, R: RefinementForest, const WEAK: bool>(
    lts: &L,
    mut signature: F,
    forest: &mut R,
) -> IndexedPartition
where
    F: FnMut(StateIndex, &IndexedPartition, &Vec<Signature<'_>>, &mut SignatureBuilder),
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

    let progress = TimeProgress::new(
        |(iteration, blocks)| {
            debug!("Iteration {iteration}, found {blocks} blocks...",);
        },
        5,
    );

    // This is a workaround for a data race in bumpalo for zero-sized slices.
    let empty_slice: &[(LabelIndex, BlockIndex)] = &[];

    while old_count != id.len() {
        trace!("Iteration {} ({} blocks)", iteration, id.len());

        old_count = id.len();
        progress.print((iteration, old_count));
        swap(&mut partition, &mut next_partition);

        // Clear the current partition to start the next blocks.
        id.clear();

        state_to_signature.clear();
        state_to_signature.resize_with(lts.num_of_states(), Signature::default);

        // SAFETY: `id` was cleared above, so it holds no `Signature` borrowing
        // from the arena; the transmute only relaxes that borrow's lifetime so it
        // can be refilled with slices allocated after the `arena.reset()` below.
        let id: &'_ mut FxHashMap<Signature<'_>, BlockIndex> = unsafe { std::mem::transmute(&mut id) };
        // SAFETY: see above; `state_to_signature` was cleared (it now holds only
        // the empty default signatures, which borrow no arena storage).
        let state_to_signature: &mut Vec<Signature<'_>> = unsafe { std::mem::transmute(&mut state_to_signature) };

        // Remove the current signatures.
        arena.reset();

        if WEAK {
            for state_index in lts.iter_states() {
                weak_bisim_signature_sorted_taus(state_index, lts, &partition, state_to_signature, &mut builder);

                trace!("State {state_index} weak signature {:?}", builder);

                // Keep track of the index for every state, either use the arena to allocate space or simply borrow the value.
                let slice = if builder.is_empty() {
                    empty_slice
                } else {
                    arena.alloc_slice_copy(&builder)
                };
                state_to_signature[state_index] = Signature::new(slice);
            }
        }

        for state_index in lts.iter_states() {
            // Compute the signature of a single state
            signature(state_index, &partition, state_to_signature, &mut builder);

            trace!("State {state_index} signature {builder:?}");

            // Keep track of the index for every state, either use the arena to allocate space or simply borrow the value.
            let mut new_id = BlockIndex::new(id.len());
            if let Some((signature, index)) = id.get_key_value(&Signature::new(&builder)) {
                // SAFETY: `signature` borrows from the arena, which outlives this
                // iteration; the transmute only re-labels that borrow with the
                // lifetime expected by `state_to_signature`.
                state_to_signature[state_index] = unsafe {
                    std::mem::transmute::<Signature<'_>, Signature<'_>>(Signature::new(signature.as_slice()))
                };
                new_id = *index;
            } else {
                let slice = if builder.is_empty() {
                    empty_slice
                } else {
                    arena.alloc_slice_copy(&builder)
                };
                id.insert(Signature::new(slice), new_id);

                // (branching) Keep track of the signature for every block in the next partition.
                state_to_signature[state_index] = Signature::new(slice);
            }

            next_partition.set_block(*state_index, new_id);
        }

        forest.record_level(&partition, &next_partition);

        iteration += 1;

        debug_assert!(
            iteration <= lts.num_of_states().max(2),
            "There can never be more splits than number of states, but at least two iterations for stability"
        );
    }

    forest.finalize(lts, &partition);

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

/// Returns true iff `partition` is stable under `compute_signature`: every
/// state in a block has the same signature, and no two blocks share a
/// signature.
pub(crate) fn is_valid_refinement<F, P, L>(lts: &L, partition: &P, mut compute_signature: F) -> bool
where
    F: FnMut(StateIndex, &P, &mut SignatureBuilder),
    P: Partition,
    L: LTS,
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
        .map(|signature: &Option<SignatureBuilder>| signature.as_ref().expect("Signature should be defined"))
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
