#![forbid(unsafe_code)]

use std::fmt;

use merc_collections::Block;
use merc_collections::BlockIndex;
use merc_collections::BlockIter;
use merc_collections::BlockPartition;
use merc_lts::StateIndex;

use crate::Partition;

/// A partition that explicitly stores a list of blocks and their indexing into
/// the list of elements. Similar to [super::BlockPartition] but without taking
/// the stability of individual elements into account.
#[derive(Debug)]
pub(crate) struct MarkedBlockPartition {
    partition: BlockPartition<bool>,
}

impl MarkedBlockPartition {
    /// Create an initial partition where all the states are in a single block
    /// 0. And all the elements in the block are marked.
    pub(crate) fn new(num_of_elements: usize) -> Self {
        Self {
            partition: BlockPartition::new(num_of_elements),
        }
    }

    /// Marks the given block as stable
    pub(crate) fn mark_block_stable(&mut self, block_index: BlockIndex) {
        *self.partition.block_annotation(block_index) = true;
    }

    /// Return a reference to the given block.
    pub(crate) fn block(&self, block_index: BlockIndex) -> &Block<bool> {
        self.partition.block(block_index)
    }

    /// Splits a block into two blocks according to the given predicate. If
    /// a split occurs both blocks are marked as unstable.
    ///
    /// If the predicate holds for all or none of the elements, no split occurs.
    pub(crate) fn split_block(
        &mut self,
        block_index: BlockIndex,
        predicate: impl Fn(StateIndex) -> bool,
    ) -> Option<BlockIndex> {
        let result = self
            .partition
            .split_block(block_index, |element| predicate(StateIndex::new(element)));

        if let Some(new_block_index) = result {
            *self.partition.block_annotation(block_index) = false;
            *self.partition.block_annotation(new_block_index) = false;
        }

        result
    }

    /// Returns the number of blocks in the partition.
    pub(crate) fn num_of_blocks(&self) -> usize {
        self.partition.num_of_blocks()
    }

    /// Returns an iterator over the elements of a given block.
    pub(crate) fn iter_block(&self, block_index: BlockIndex) -> BlockIter<'_> {
        self.partition.iter_block(block_index)
    }
}

impl fmt::Display for MarkedBlockPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.partition)
    }
}

impl Partition for MarkedBlockPartition {
    fn block_number(&self, state_index: StateIndex) -> BlockIndex {
        // Note that this is O(n) in the number of blocks. This could be improved
        // by storing a mapping from state index to block index. However, this
        // is only used in the comparison functions, so it is not a big issue.
        for block_index in 0..self.partition.num_of_blocks() {
            for element in self.partition.iter_block(BlockIndex::new(block_index)) {
                if element == *state_index {
                    return BlockIndex::new(block_index);
                }
            }
        }

        panic!("State index {:?} not found in partition {:?}", state_index, self);
    }

    fn num_of_blocks(&self) -> usize {
        self.partition.num_of_blocks()
    }

    fn len(&self) -> usize {
        self.partition.len()
    }
}
