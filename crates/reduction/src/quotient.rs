use std::time::Instant;

use log::debug;
use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilder;
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

    /// Returns true iff the partitions are equal, runs in O(n^2)
    fn equal(&self, other: &impl Partition) -> bool {
        // Check that states in the same block, have a single (unique) number in
        // the other partition.
        for block_index in (0..self.num_of_blocks()).map(BlockIndex::new) {
            let mut other_block_index = None;

            for state_index in (0..self.len())
                .map(StateIndex::new)
                .filter(|&state_index| self.block_number(state_index) == block_index)
            {
                match other_block_index {
                    None => other_block_index = Some(other.block_number(state_index)),
                    Some(other_block_index) => {
                        if other.block_number(state_index) != other_block_index {
                            return false;
                        }
                    }
                }
            }
        }

        for block_index in (0..other.num_of_blocks()).map(BlockIndex::new) {
            let mut other_block_index = None;

            for state_index in (0..self.len())
                .map(StateIndex::new)
                .filter(|&state_index| other.block_number(state_index) == block_index)
            {
                match other_block_index {
                    None => other_block_index = Some(self.block_number(state_index)),
                    Some(other_block_index) => {
                        if self.block_number(state_index) != other_block_index {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }
}

/// Returns a new LTS based on the given partition.
///
/// The naive version will add the transitions of all states in the block to the quotient LTS.
pub fn quotient_lts_naive(
    lts: &impl LTS,
    partition: &impl Partition,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem {
    let start = std::time::Instant::now();
    // Introduce the transitions based on the block numbers, the number of blocks is a decent approximation for the number of transitions.
    let mut transitions = LtsBuilder::with_capacity(
        partition.num_of_blocks(),
        lts.num_of_labels(),
        partition.num_of_blocks(),
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

                transitions.add_transition(
                    StateIndex::new(block.value()),
                    transition.label,
                    StateIndex::new(to_block.value()),
                );
            }
        }
    }

    // Remove duplicates.
    transitions.remove_duplicates();

    let result = LabelledTransitionSystem::new(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        Some(partition.num_of_blocks()),
        || transitions.iter(),
        lts.labels().into(),
        Vec::new(),
    );
    debug!("Time quotient: {:.3}s", start.elapsed().as_secs_f64());
    result
}

/// Optimised implementation for block partitions.
///
/// Chooses a single state in the block as representative. If BRANCHING then the chosen state is a bottom state.
pub fn quotient_lts_block<const BRANCHING: bool>(
    lts: &impl LTS,
    partition: &BlockPartition,
) -> LabelledTransitionSystem {
    let start = Instant::now();
    let mut transitions = LtsBuilder::new();

    for block in (0..partition.num_of_blocks()).map(BlockIndex::new) {
        // Pick any state in the block
        let mut candidate = if let Some(state) = partition.iter_block(block).next() {
            state
        } else {
            panic!("Found empty block {}", block);
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

        if BRANCHING {
            for trans in lts.outgoing_transitions(candidate) {
                // Candidate is a bottom state, so add all transitions.
                debug_assert!(
                    !(lts.is_hidden_label(trans.label) && partition.block_number(trans.to) == block),
                    "This state is not bottom {}",
                    block
                );

                transitions.add_transition(
                    StateIndex::new(*block),
                    trans.label,
                    StateIndex::new(*partition.block_number(trans.to)),
                );
            }
        }

        debug_assert!(
            !partition.block(block).is_empty(),
            "Blocks in the partition should not be empty"
        );
    }
    // Remove duplicates.
    transitions.remove_duplicates();

    let result = LabelledTransitionSystem::new(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        Some(partition.num_of_blocks()),
        || transitions.iter(),
        lts.labels().into(),
        Vec::new(),
    );
    debug!("Time quotient: {:.3}s", start.elapsed().as_secs_f64());
    result
}
