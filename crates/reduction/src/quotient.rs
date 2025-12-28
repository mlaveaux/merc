#![forbid(unsafe_code)]

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilderFast;
use merc_lts::StateIndex;
use merc_utilities::TagIndex;

use crate::BlockPartition;

/// A zero sized tag for the block.
pub struct BlockTag {}

/// The index for blocks.
pub type BlockIndex = TagIndex<usize, BlockTag>;

/// A trait for partition refinement algorithms that expose the block number for
/// every state. Can be used to compute the quotient labelled transition system.
///
/// The invariants are that the union of all blocks is the original set, and
/// that each block contains distinct elements
pub trait Partition {
    /// Returns the block number for the given state.
    fn block_number(&self, state_index: StateIndex) -> BlockIndex;

    /// Returns the number of blocks in the partition.
    fn num_of_blocks(&self) -> usize;

    /// Returns the number of elements in the partition.
    fn len(&self) -> usize;

    /// Returns whether the partition is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Returns a new LTS based on the given partition.
///
/// The naive version will add the transitions of all states in the block to the quotient LTS.
pub fn quotient_lts_naive<L: LTS>(
    lts: &L,
    partition: &impl Partition,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    // Introduce the transitions based on the block numbers, the number of blocks is a decent approximation for the number of transitions.
    let mut builder = LtsBuilderFast::with_capacity(
        lts.labels().into(),
        Vec::new(),
        partition.num_of_blocks(), // We expect one transition per state.
    );

    for state_index in lts.iter_states() {
        for transition in lts.outgoing_transitions(state_index) {
            let block = partition.block_number(state_index);
            let to_block = partition.block_number(transition.to);

            // If we eliminate tau loops then check if the 'to' and 'from' end up in the same block
            if !(eliminate_tau_loops && lts.is_hidden_label(transition.label) && block == to_block) {
                debug_assert!(
                    partition.block_number(state_index) < partition.num_of_blocks(),
                    "Quotienting assumes that the block numbers do not exceed the number of blocks"
                );

                builder.add_transition(
                    StateIndex::new(block.value()),
                    &lts.labels()[transition.label],
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

/// Optimised implementation for block partitions.
///
/// Chooses a single state in the block as representative. If BRANCHING then the chosen state is a bottom state.
pub fn quotient_lts_block<L: LTS, const BRANCHING: bool>(
    lts: &L,
    partition: &BlockPartition,
) -> LabelledTransitionSystem<L::Label> {
    let mut builder = LtsBuilderFast::new(lts.labels().into(), Vec::new());

    for block in (0..partition.num_of_blocks()).map(BlockIndex::new) {
        // Pick any state in the block
        let mut candidate = if let Some(state) = partition.iter_block(block).next() {
            state
        } else {
            panic!("Blocks in the partition should not be empty {}", block);
        };

        if BRANCHING {
            // DFS into a bottom state.
            let mut found = false;
            while !found {
                found = true;

                if let Some(trans) = lts
                    .outgoing_transitions(candidate)
                    .find(|trans| lts.is_hidden_label(trans.label) && partition.block_number(trans.to) == block)
                {
                    found = false;
                    candidate = trans.to;
                }
            }
        }

        // Add all transitions from the representative state.
        for transition in lts.outgoing_transitions(candidate) {
            if BRANCHING {
                // Candidate is a bottom state, so add all transitions.
                debug_assert!(
                    !(lts.is_hidden_label(transition.label) && partition.block_number(transition.to) == block),
                    "This state is not bottom {}",
                    block
                );
            }

            builder.add_transition(
                StateIndex::new(*block),
                &lts.labels()[transition.label],
                StateIndex::new(*partition.block_number(transition.to)),
            );
        }
    }

    builder.require_num_of_states(partition.num_of_blocks());
    builder.finish(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        true,
    )
}
