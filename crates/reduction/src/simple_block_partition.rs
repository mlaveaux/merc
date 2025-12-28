#![forbid(unsafe_code)]

use std::fmt;

use itertools::Itertools;

use log::trace;
use merc_lts::StateIndex;

use crate::BlockIndex;
use crate::Partition;

/// A partition that explicitly stores a list of blocks and their indexing into
/// the list of elements. Similar to [super::BlockPartition] but without taking
/// the stability of individual elements into account.
#[derive(Debug)]
pub struct SimpleBlockPartition {
    elements: Vec<StateIndex>,
    blocks: Vec<SimpleBlock>,
}

impl SimpleBlockPartition {
    /// Create an initial partition where all the states are in a single block
    /// 0. And all the elements in the block are marked.
    pub fn new(num_of_elements: usize) -> Self {
        debug_assert!(num_of_elements > 0, "Cannot partition the empty set");

        let blocks = vec![SimpleBlock::new(0, num_of_elements)];
        let elements = (0..num_of_elements).map(StateIndex::new).collect();

        Self { elements, blocks }
    }

    /// Marks the given block as stable
    pub fn mark_block_stable(&mut self, block_index: BlockIndex) {
        self.blocks[block_index].stable = true;
    }

    /// Return a reference to the given block.
    pub fn block(&self, block_index: BlockIndex) -> &SimpleBlock {
        &self.blocks[block_index]
    }

    /// Splits a block into two blocks according to the given predicate. If the
    /// predicate holds for all or none of the elements, no split occurs.
    pub fn split_block(
        &mut self,
        block_index: BlockIndex,
        predicate: impl Fn(StateIndex) -> bool,
    ) -> Option<BlockIndex> {
        // Size of the new block.
        let mut size = 0usize;

        for state in self.blocks[block_index].begin..self.blocks[block_index].end {
            if predicate(self.elements[state]) {
                self.elements.swap(self.blocks[block_index].begin + size, state);
                size += 1;
            }
        }

        // The original block are now the first [begin, begin + size) elements
        if size == 0 || size == self.blocks[block_index].len() {
            // No split occurred
            return None;
        }

        // Create a new block for the remaining elements
        let new_block = SimpleBlock::new(self.blocks[block_index].begin + size, self.blocks[block_index].end);
        let last_block = self.blocks.len();
        self.blocks.push(new_block);

        // Update the original block
        self.blocks[block_index].end = self.blocks[block_index].begin + size;
        self.blocks[block_index].stable = false;

        trace!(
            "Split block {:?} into blocks {:?} and {:?}",
            block_index,
            block_index,
            BlockIndex::new(last_block)
        );
        Some(BlockIndex::new(last_block))
    }

    /// Returns the number of blocks in the partition.
    pub fn num_of_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Returns an iterator over the elements of a given block.
    pub fn iter_block(&self, block_index: BlockIndex) -> SimpleBlockIter<'_> {
        SimpleBlockIter {
            elements: &self.elements,
            index: self.blocks[block_index].begin,
            end: self.blocks[block_index].end,
        }
    }

    /// Returns an iterator over all blocks in the partition.
    pub fn iter(&self) -> impl Iterator<Item = &SimpleBlock> {
        self.blocks.iter()
    }

    /// Returns an iterator over all blocks in the partition.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut SimpleBlock> {
        self.blocks.iter_mut()
    }
}

impl fmt::Display for SimpleBlockPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format = self
            .blocks
            .iter()
            .map(|block| format!("{{{}}}", block.iter(&self.elements).format(", ")))
            .format(", ");

        write!(f, "{{{}}}", format)
    }
}

impl Partition for SimpleBlockPartition {
    fn block_number(&self, state_index: StateIndex) -> BlockIndex {
        // Note that this is O(n) in the number of blocks. This could be improved
        // by storing a mapping from state index to block index. However, this
        // is only used in the comparison functions, so it is not a big issue.
        for (block_index, block) in self.blocks.iter().enumerate() {
            for element in block.iter(&self.elements) {
                if element == state_index {
                    return BlockIndex::new(block_index);
                }
            }
        }

        panic!("State index {:?} not found in partition {:?}", state_index, self);
    }

    fn num_of_blocks(&self) -> usize {
        self.blocks.len()
    }

    fn len(&self) -> usize {
        self.elements.len()
    }
}

/// A [super::Block] that stores a subset of the elements in a partition, but
/// with individual stable elements.
///
/// # Details
///
/// It uses `start` and `end` to indicate a range start..end of elements in the
/// partition. The stable flag indicates whether the block is stable.
#[derive(Clone, Copy, Debug)]
pub struct SimpleBlock {
    begin: usize,
    end: usize,
    stable: bool,
}

impl SimpleBlock {
    /// Creates a new block that is not marked.
    pub fn new(begin: usize, end: usize) -> SimpleBlock {
        debug_assert!(begin < end, "The range of this block is incorrect");

        SimpleBlock {
            begin,
            end,
            stable: false,
        }
    }

    /// Returns an iterator over the elements in this block.
    pub fn iter<'a>(&self, elements: &'a Vec<StateIndex>) -> SimpleBlockIter<'a> {
        SimpleBlockIter {
            elements,
            index: self.begin,
            end: self.end,
        }
    }

    /// Returns the number of elements in the block.
    pub fn len(&self) -> usize {
        self.assert_consistent();

        self.end - self.begin
    }

    /// Returns true iff the block is empty.
    pub fn is_empty(&self) -> bool {
        self.assert_consistent();

        self.begin == self.end
    }

    /// Returns true iff the block is stable.
    pub fn is_stable(&self) -> bool {
        self.stable
    }

    /// Marks the block as stable.
    pub fn mark_stable(&mut self) {
        self.stable = true
    }

    /// Returns true iff the block is consistent.
    fn assert_consistent(self) {
        debug_assert!(self.begin < self.end, "The range of block {self:?} is incorrect");
    }
}

pub struct SimpleBlockIter<'a> {
    elements: &'a Vec<StateIndex>,
    index: usize,
    end: usize,
}

impl Iterator for SimpleBlockIter<'_> {
    type Item = StateIndex;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.end {
            let element = self.elements[self.index];
            self.index += 1;
            Some(element)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_block_partition() {
        let mut partition = SimpleBlockPartition::new(10);

        assert_eq!(partition.num_of_blocks(), 1);

        let initial_block = BlockIndex::new(0);
        assert_eq!(partition.block(initial_block).len(), 10);

        let block_index = partition
            .split_block(BlockIndex::new(0), |state| *state < *StateIndex::new(5))
            .unwrap();

        assert_eq!(partition.num_of_blocks(), 2);
        assert_eq!(partition.block(initial_block).len(), 5);
        assert_eq!(partition.block(block_index).len(), 5);
    }
}
