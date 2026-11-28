#![forbid(unsafe_code)]

use log::trace;
use merc_collections::BlockIndex;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilder;
use merc_lts::LtsBuilderMem;
use merc_lts::StateIndex;
use merc_lts::reachability;

use crate::BlockPartition;
use crate::Partition;

/// Returns a new LTS based on the given partition.
///
/// Computes the existential quotient of the given LTS based on the given
/// partition:
///
/// > \[p\] -a-> \[q\] iff there exist states s in p and t in q such that s -a-> t
///
/// If `eliminate_inert_taus` is true then non self-loop tau steps \[p\] -tau-> \[p\] are eliminated.
/// If `eliminate_tau_loops` is true then tau self-loops s -tau-> s are eliminated.
/// The two parameters are independent: each controls a disjoint set of transitions.
pub fn quotient_lts_naive<L: LTS, P: Partition>(
    lts: &L,
    partition: &P,
    eliminate_inert_taus: bool,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    // Introduce the transitions based on the block numbers. `lts.num_of_transitions()`
    // is an exact upper bound on the quotient's transition count (quotienting only
    // ever drops or merges transitions), and `partition.num_of_blocks()` is its exact
    // state count.
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        partition.num_of_blocks(),
        lts.num_of_transitions(),
    );
    let labels = builder.label_indices(lts.labels());

    for state_index in lts.iter_states() {
        for transition in lts.outgoing_transitions(state_index) {
            let block = partition.block_number(state_index);
            let to_block = partition.block_number(transition.to);

            // Eliminate non-self-loop inert taus and (independently) tau self-loops.
            if !(eliminate_inert_taus
                && lts.is_hidden_label(transition.label)
                && block == to_block
                && state_index != transition.to)
                && !(eliminate_tau_loops && lts.is_hidden_label(transition.label) && state_index == transition.to)
            {
                debug_assert!(
                    partition.block_number(state_index) < partition.num_of_blocks(),
                    "Quotienting assumes that the block numbers do not exceed the number of blocks"
                );

                builder.add_transition_index(
                    StateIndex::new(block.value()),
                    labels[transition.label.value()],
                    StateIndex::new(to_block.value()),
                );
            }
        }
    }

    builder.require_num_of_states(partition.num_of_blocks());
    builder.finish(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        true,
    )
}

/// Returns a weak bisimulation quotient that additionally removes transitions
/// subsumed by a one-hidden-step alternative.
///
/// If `eliminate_tau_loops` is true then tau self-loops are eliminated.
pub(crate) fn quotient_lts_weak<L: LTS, P: Partition>(
    lts: &L,
    partition: &P,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    let quotient = quotient_lts_naive(lts, partition, true, eliminate_tau_loops);
    remove_redundant_transitions(&quotient)
}

/// Weak bisimulation quotient that removes redundant transitions.
fn remove_redundant_transitions<L: LTS>(lts: &L) -> LabelledTransitionSystem<L::Label> {
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        lts.num_of_states(),
        lts.num_of_transitions(),
    );
    let labels = builder.label_indices(lts.labels());
    builder.require_num_of_states(lts.num_of_states());

    for from in lts.iter_states() {
        for transition in lts.outgoing_transitions(from) {
            if !is_redundant_transition(lts, from, transition.label, transition.to) {
                builder.add_transition_index(from, labels[transition.label.value()], transition.to);
            } else {
                trace!(
                    "Removing redundant transition: {} -[{}]-> {}",
                    from,
                    lts.labels()[transition.label],
                    transition.to
                );
            }
        }
    }

    builder.finish(lts.initial_state_index(), true)
}

/// Returns true when transition `from -label-> target` is redundant.
///
/// A transition `s -a-> u` is redundant iff there exist states `t` and `v` such that:
/// - `s -tau*-> t`
/// - `t -a-> v` (for hidden `a`, any hidden transition)
/// - `v -tau*-> u`
///
/// The exact transition `s -a-> u` itself is not considered a witness.
fn is_redundant_transition<L: LTS>(lts: &L, from: StateIndex, label: LabelIndex, target: StateIndex) -> bool {
    let mut redundant = false;

    reachability(
        lts,
        from,
        |l| lts.is_hidden_label(l),
        |middle| {
            if redundant {
                return;
            }

            for transition in lts.outgoing_transitions(middle) {
                let same_action = if lts.is_hidden_label(label) {
                    lts.is_hidden_label(transition.label)
                } else {
                    transition.label == label
                };

                if !same_action {
                    continue;
                }

                // Skip the exact transition being tested.
                if middle == from && transition.label == label && transition.to == target {
                    continue;
                }

                // Skip tau self-loops on the middle state, as they do not contribute to the redundancy.
                if lts.is_hidden_label(transition.label) && middle == transition.to {
                    continue;
                }

                reachability(
                    lts,
                    transition.to,
                    |l| lts.is_hidden_label(l),
                    |reached| {
                        if reached == target {
                            redundant = true;
                        }
                    },
                );

                if redundant {
                    break;
                }
            }
        },
    );

    redundant
}

/// Optimised implementation for block partitions.
///
/// Chooses a single state in the block as representative. If `BRANCHING` then
/// the chosen state is a bottom state, i.e. a state with no outgoing tau
/// transition that stays inside the block. For `BRANCHING` we only consider
/// bottom states as representatives.
///
/// If `eliminate_tau_loops` is true then tau self-loops are eliminated.
///
/// # Panics
///
/// If `BRANCHING`, every block is assumed to have a genuine bottom state.
pub fn quotient_lts_block<L: LTS, const BRANCHING: bool>(
    lts: &L,
    partition: &BlockPartition,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        partition.num_of_blocks(),
        lts.num_of_transitions(),
    );
    let labels = builder.label_indices(lts.labels());

    // Reused across blocks to find bottom states when BRANCHING.
    let mut visited = vec![false; lts.num_of_states()];
    // Only touched states are reset to avoid clearing the entire visited vector.
    let mut touched = Vec::new();

    for block in (0..partition.num_of_blocks()).map(BlockIndex::new) {
        // Pick any state in the block
        let mut candidate = if let Some(state) = partition.iter_block(block).next() {
            state
        } else {
            panic!("Blocks in the partition should not be empty {}", block);
        };

        if BRANCHING {
            // traverse any outgoing transition to find a bottom state.
            'outer: loop {
                if visited[candidate] {
                    panic!(
                        "quotient_lts_block's BRANCHING precondition is violated: block {} has no \
                         bottom state (its tau-subgraph is a cycle with no exit), state {}",
                        block, candidate
                    );
                }
                visited[candidate] = true;
                touched.push(candidate);

                if let Some(trans) = lts.outgoing_transitions(candidate).find(|trans| {
                    lts.is_hidden_label(trans.label)
                        && candidate != trans.to // Ignore self loops for the bottom state search.
                        && partition.block_number(trans.to) == block
                }) {
                    candidate = trans.to;
                    continue 'outer;
                }

                // No outgoing tau transition to the same block, so we found a bottom state.
                break;
            }

            // Reset only the entries touched by this walk.
            for state in touched.drain(..) {
                visited[state] = false;
            }
        }

        // Add all transitions from the representative state (or the bottom state if BRANCHING) to the quotient LTS.
        for transition in lts.outgoing_transitions(candidate) {
            if BRANCHING {
                debug_assert!(
                    !(lts.is_hidden_label(transition.label)
                        && candidate != transition.to
                        && partition.block_number(transition.to) == block),
                    "The representative {} is not bottom state",
                    candidate
                );
            }

            if !(eliminate_tau_loops && lts.is_hidden_label(transition.label) && candidate == transition.to) {
                builder.add_transition_index(
                    StateIndex::new(*block),
                    labels[transition.label.value()],
                    StateIndex::new(*partition.block_number(transition.to)),
                );
            }
        }
    }

    builder.require_num_of_states(partition.num_of_blocks());
    builder.finish(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        true,
    )
}
