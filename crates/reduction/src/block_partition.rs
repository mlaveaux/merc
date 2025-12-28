#![forbid(unsafe_code)]

use std::fmt;

use itertools::Itertools;

use merc_lts::IncomingTransitions;
use merc_lts::StateIndex;

use super::Partition;
use crate::BlockIndex;

/// A partition that explicitly stores a list of blocks and their indexing into
/// the list of elements.
#[derive(Debug)]
pub struct BlockPartition {
    elements: Vec<StateIndex>,
    blocks: Vec<Block>,

    // These are only used to provide O(1) marking of elements.
    /// Stores the block index for each element.
    element_to_block: Vec<BlockIndex>,

    /// Stores the offset within the block for every element.
    element_offset: Vec<usize>,
}

impl BlockPartition {
    /// Create an initial partition where all the states are in a single block
    /// 0. And all the elements in the block are marked.
    pub fn new(num_of_elements: usize) -> BlockPartition {
        debug_assert!(num_of_elements > 0, "Cannot partition the empty set");

        let blocks = vec![Block::new(0, num_of_elements)];
        let elements = (0..num_of_elements).map(StateIndex::new).collect();
        let element_to_block = vec![BlockIndex::new(0); num_of_elements];
        let element_to_block_offset = (0..num_of_elements).collect();

        BlockPartition {
            elements,
            element_to_block,
            element_offset: element_to_block_offset,
            blocks,
        }
    }

    /// Partition the elements of the given block into multiple new blocks based
    /// on the given partitioner; which returns a number for each marked
    /// element. Elements with the same number belong to the same block, and the
    /// returned numbers should be dense.
    ///
    /// Returns an iterator over the new block indices, where the first element
    /// is the index of the block that was partitioned. And that block is the
    /// largest block.
    pub fn partition_marked_with<F>(
        &mut self,
        block_index: BlockIndex,
        builder: &mut BlockPartitionBuilder,
        mut partitioner: F,
    ) -> impl Iterator<Item = BlockIndex> + use<F>
    where
        F: FnMut(StateIndex, &BlockPartition) -> BlockIndex,
    {
        let block = self.blocks[block_index];
        debug_assert!(
            block.has_marked(),
            "Cannot partition marked elements of a block without marked elements"
        );

        if block.len() == 1 {
            // Block only has one element, so trivially partitioned.
            self.blocks[block_index].unmark_all();
            // Note that all the returned iterators MUST have the same type, but we cannot chain typed_index since Step is an unstable trait.
            return (block_index.value()..=block_index.value())
                .chain(0..0)
                .map(BlockIndex::new);
        }

        // Keeps track of the block index for every element in this block by index.
        builder.index_to_block.clear();
        builder.block_sizes.clear();
        builder.old_elements.clear();

        builder.index_to_block.resize(block.len_marked(), BlockIndex::new(0));

        // O(n log n) Loop through the marked elements in order (to maintain topological sorting)
        builder.old_elements.extend(block.iter_marked(&self.elements));
        builder.old_elements.sort_unstable();

        // O(n) Loop over marked elements to determine the number of the new block each element is in.
        for (element_index, &element) in builder.old_elements.iter().enumerate() {
            let number = partitioner(element, self);

            builder.index_to_block[element_index] = number;
            if number.value() + 1 > builder.block_sizes.len() {
                builder.block_sizes.resize(number.value() + 1, 0);
            }

            builder.block_sizes[number] += 1;
        }

        // Convert block sizes into block offsets.
        let end_of_blocks = self.blocks.len();
        let new_block_index = if block.has_unmarked() {
            self.blocks.len()
        } else {
            self.blocks.len() - 1
        };

        let _ = builder.block_sizes.iter_mut().fold(0usize, |current, size| {
            debug_assert!(*size > 0, "Partition is not dense, there are empty blocks");

            let current = if current == 0 {
                if block.has_unmarked() {
                    // Adapt the offsets of the current block to only include the unmarked elements.
                    self.blocks[block_index] = Block::new_unmarked(block.begin, block.marked_split);

                    // Introduce a new block for the zero block.
                    self.blocks
                        .push(Block::new_unmarked(block.marked_split, block.marked_split + *size));
                    block.marked_split
                } else {
                    // Use this as the zero block.
                    self.blocks[block_index] = Block::new_unmarked(block.begin, block.begin + *size);
                    block.begin
                }
            } else {
                // Introduce a new block for every other non-empty block.
                self.blocks.push(Block::new_unmarked(current, current + *size));
                current
            };

            let offset = current + *size;
            *size = current;
            offset
        });
        let block_offsets = &mut builder.block_sizes;

        for (index, offset_block_index) in builder.index_to_block.iter().enumerate() {
            // Swap the element to the correct position.
            let element = builder.old_elements[index];
            self.elements[block_offsets[*offset_block_index]] = builder.old_elements[index];
            self.element_offset[element] = block_offsets[*offset_block_index];
            self.element_to_block[element] = if *offset_block_index == 0 && !block.has_unmarked() {
                block_index
            } else {
                BlockIndex::new(new_block_index + offset_block_index.value())
            };

            // Update the offset for this block.
            block_offsets[*offset_block_index] += 1;
        }

        // Swap the first block and the maximum sized block.
        let max_block_index = (block_index.value()..=block_index.value())
            .chain(end_of_blocks..self.blocks.len())
            .map(BlockIndex::new)
            .max_by_key(|block_index| self.block(*block_index).len())
            .unwrap();
        self.swap_blocks(block_index, max_block_index);

        self.assert_consistent();

        (block_index.value()..=block_index.value())
            .chain(end_of_blocks..self.blocks.len())
            .map(BlockIndex::new)
    }

    /// Split the given block into two separate block based on the splitter
    /// predicate.
    pub fn split_marked(&mut self, block_index: usize, mut splitter: impl FnMut(StateIndex) -> bool) {
        let mut updated_block = self.blocks[block_index];
        let mut new_block: Option<Block> = None;

        // Loop over all elements, we use a while loop since the index stays the
        // same when a swap takes place.
        let mut element_index = updated_block.marked_split;
        while element_index < updated_block.end {
            let element = self.elements[element_index];
            if splitter(element) {
                match &mut new_block {
                    None => {
                        new_block = Some(Block::new_unmarked(updated_block.end - 1, updated_block.end));

                        // Swap the current element to the last place
                        self.swap_elements(element_index, updated_block.end - 1);
                        updated_block.end -= 1;
                    }
                    Some(new_block_index) => {
                        // Swap the current element to the beginning of the new block.
                        new_block_index.begin -= 1;
                        updated_block.end -= 1;

                        self.swap_elements(element_index, new_block_index.begin);
                    }
                }
            } else {
                // If no swap takes place consider the next index.
                element_index += 1;
            }
        }

        if let Some(new_block) = new_block {
            if (updated_block.end - updated_block.begin) != 0 {
                // A new block was introduced, so we need to update the current
                // block. Unless the current block is empty in which case
                // nothing changes.
                updated_block.unmark_all();
                self.blocks[block_index] = updated_block;

                // Introduce a new block for the split, containing only the new element.
                self.blocks.push(new_block);

                // Update the elements for the new block
                for element in new_block.iter(&self.elements) {
                    self.element_to_block[element] = BlockIndex::new(self.blocks.len() - 1);
                }
            }
        }

        println!("{self:?}");
        self.assert_consistent();
    }

    /// Makes the marked elements closed under the silent closure of incoming
    /// tau-transitions within the current block.
    pub fn mark_backward_closure(&mut self, block_index: BlockIndex, incoming_transitions: &IncomingTransitions) {
        let block = self.blocks[block_index];
        let mut it = block.end - 1;

        // First compute backwards silent transitive closure.
        while it >= self.blocks[block_index].marked_split && self.blocks[block_index].has_unmarked() {
            for transition in incoming_transitions.incoming_silent_transitions(self.elements[it]) {
                if self.block_number(transition.to) == block_index {
                    self.mark_element(transition.to);
                }
            }

            if it == 0 {
                break;
            }

            it -= 1;
        }
    }

    /// Swaps the given blocks given by the indices.
    pub fn swap_blocks(&mut self, left_index: BlockIndex, right_index: BlockIndex) {
        if left_index == right_index {
            // Nothing to do.
            return;
        }

        self.blocks.swap(left_index.value(), right_index.value());

        for element in self.block(left_index).iter(&self.elements) {
            self.element_to_block[element] = left_index;
        }

        for element in self.block(right_index).iter(&self.elements) {
            self.element_to_block[element] = right_index;
        }

        self.assert_consistent();
    }

    /// Marks the given element, such that it is returned by iter_marked.
    pub fn mark_element(&mut self, element: StateIndex) {
        let block_index = self.element_to_block[element];
        let offset = self.element_offset[element];
        let marked_split = self.blocks[block_index].marked_split;

        if offset < marked_split {
            // Element was not already marked.
            self.swap_elements(offset, marked_split - 1);
            self.blocks[block_index].marked_split -= 1;
        }

        self.blocks[block_index].assert_consistent();
    }

    /// Returns true iff the given element has already been marked.
    pub fn is_element_marked(&self, element: StateIndex) -> bool {
        let block_index = self.element_to_block[element];
        let offset = self.element_offset[element];
        let marked_split = self.blocks[block_index].marked_split;

        offset >= marked_split
    }

    /// Return a reference to the given block.
    pub fn block(&self, block_index: BlockIndex) -> &Block {
        &self.blocks[block_index]
    }

    /// Returns the number of blocks in the partition.
    pub fn num_of_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Returns an iterator over the elements of a given block.
    pub fn iter_block(&self, block_index: BlockIndex) -> BlockIter<'_> {
        BlockIter {
            elements: &self.elements,
            index: self.blocks[block_index].begin,
            end: self.blocks[block_index].end,
        }
    }

    /// Swaps the elements at the given indices and updates the element_to_block
    fn swap_elements(&mut self, left_index: usize, right_index: usize) {
        self.elements.swap(left_index, right_index);
        self.element_offset[self.elements[left_index]] = left_index;
        self.element_offset[self.elements[right_index]] = right_index;
    }

    /// Returns true iff the invariants of a partition hold
    fn assert_consistent(&self) -> bool {
        if cfg!(debug_assertions) {
            let mut marked = vec![false; self.elements.len()];

            for block in &self.blocks {
                for element in block.iter(&self.elements) {
                    debug_assert!(
                        !marked[element],
                        "Partition {self}, element {element} belongs to multiple blocks"
                    );
                    marked[element] = true;
                }

                block.assert_consistent();
            }

            // Check that every element belongs to a block.
            debug_assert!(
                !marked.contains(&false),
                "Partition {self} contains elements that do not belong to a block"
            );

            // Check that it belongs to the block indicated by element_to_block
            for (current_element, block_index) in self.element_to_block.iter().enumerate() {
                debug_assert!(
                    self.blocks[block_index.value()]
                        .iter(&self.elements)
                        .any(|element| element == current_element),
                    "Partition {self:?}, element {current_element} does not belong to block {block_index} as indicated by element_to_block"
                );

                let index = self.element_offset[current_element];
                debug_assert_eq!(
                    self.elements[index], current_element,
                    "Partition {self:?}, element {current_element} does not have the correct offset in the block"
                );
            }
        }

        true
    }
}

#[derive(Default)]
pub struct BlockPartitionBuilder {
    // Keeps track of the block index for every element in this block by index.
    index_to_block: Vec<BlockIndex>,

    /// Keeps track of the size of each block.
    block_sizes: Vec<usize>,

    /// Stores the old elements to perform the swaps safely.
    old_elements: Vec<StateIndex>,
}

impl Partition for BlockPartition {
    fn block_number(&self, element: StateIndex) -> BlockIndex {
        self.element_to_block[element.value()]
    }

    fn num_of_blocks(&self) -> usize {
        self.blocks.len()
    }

    fn len(&self) -> usize {
        self.elements.len()
    }
}

impl fmt::Display for BlockPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let blocks_str = self.blocks.iter().format_with(", ", |block, f| {
            let elements = block
                .iter_unmarked(&self.elements)
                .map(|e| (e, false))
                .chain(block.iter_marked(&self.elements).map(|e| (e, true)))
                .format_with(", ", |(e, marked), f| {
                    if marked {
                        f(&format_args!("{}*", e))
                    } else {
                        f(&format_args!("{}", e))
                    }
                });

            f(&format_args!("{{{}}}", elements))
        });

        write!(f, "{{{}}}", blocks_str)
    }
}

/// A block stores a subset of the elements in a partition.
///
/// # Details
///
/// A block uses `start`, `middle` and `end` indices to indicate a range
/// `start`..`end` of elements in the partition. The middle is used such that
/// `marked_split`..`end` are the marked elements. This is useful to be able to
/// split off new blocks cheaply.
///
/// Invariant: `start` <= `middle` <= `end` && `start` < `end`.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    begin: usize,
    marked_split: usize,
    end: usize,
}

impl Block {
    /// Creates a new block where every element is marked.
    pub fn new(begin: usize, end: usize) -> Block {
        debug_assert!(begin < end, "The range of this block is incorrect");

        Block {
            begin,
            marked_split: begin,
            end,
        }
    }

    pub fn new_unmarked(begin: usize, end: usize) -> Block {
        debug_assert!(begin < end, "The range {begin} to {end} of this block is incorrect");

        Block {
            begin,
            marked_split: end,
            end,
        }
    }

    /// Returns an iterator over the elements in this block.
    pub fn iter<'a>(&self, elements: &'a Vec<StateIndex>) -> BlockIter<'a> {
        BlockIter {
            elements,
            index: self.begin,
            end: self.end,
        }
    }

    /// Returns an iterator over the marked elements in this block.
    pub fn iter_marked<'a>(&self, elements: &'a Vec<StateIndex>) -> BlockIter<'a> {
        BlockIter {
            elements,
            index: self.marked_split,
            end: self.end,
        }
    }

    /// Returns an iterator over the unmarked elements in this block.
    pub fn iter_unmarked<'a>(&self, elements: &'a Vec<StateIndex>) -> BlockIter<'a> {
        BlockIter {
            elements,
            index: self.begin,
            end: self.marked_split,
        }
    }

    /// Returns true iff the block has marked elements.
    pub fn has_marked(&self) -> bool {
        self.assert_consistent();

        self.marked_split < self.end
    }

    /// Returns true iff the block has unmarked elements.
    pub fn has_unmarked(&self) -> bool {
        self.assert_consistent();

        self.begin < self.marked_split
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

    /// Returns the number of marked elements in the block.
    pub fn len_marked(&self) -> usize {
        self.assert_consistent();

        self.end - self.marked_split
    }

    /// Unmark all elements in the block.
    fn unmark_all(&mut self) {
        self.marked_split = self.end;
    }

    /// Returns true iff the block is consistent.
    fn assert_consistent(self) {
        debug_assert!(self.begin < self.end, "The range of block {self:?} is incorrect",);

        debug_assert!(
            self.begin <= self.marked_split,
            "The marked_split lies before the beginning of the block {self:?}"
        );

        debug_assert!(
            self.marked_split <= self.end,
            "The marked_split lies after the beginning of the block {self:?}"
        );
    }
}

pub struct BlockIter<'a> {
    elements: &'a Vec<StateIndex>,
    index: usize,
    end: usize,
}

impl Iterator for BlockIter<'_> {
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

    use test_log::test;

    #[test]
    fn test_block_partition_split() {
        let mut partition = BlockPartition::new(10);

        partition.split_marked(0, |element| element < 3);

        // The new block only has elements that satisfy the predicate.
        for element in partition.iter_block(BlockIndex::new(1)) {
            assert!(element < 3);
        }

        for element in partition.iter_block(BlockIndex::new(0)) {
            assert!(element >= 3);
        }

        for i in (0..10).map(StateIndex::new) {
            partition.mark_element(i);
        }

        partition.split_marked(0, |element| element < 7);
        for element in partition.iter_block(BlockIndex::new(2)) {
            assert!((3..7).contains(&element.value()));
        }

        for element in partition.iter_block(BlockIndex::new(0)) {
            assert!(element >= 7);
        }

        // Test the case where all elements belong to the split block.
        partition.split_marked(1, |element| element < 7);
    }

    #[test]
    fn test_block_partition_partitioning() {
        // Test the partitioning function for a random assignment of elements
        let mut partition = BlockPartition::new(10);
        let mut builder = BlockPartitionBuilder::default();

        let _ = partition.partition_marked_with(BlockIndex::new(0), &mut builder, |element, _| match element.value() {
            0..=1 => BlockIndex::new(0),
            2..=6 => BlockIndex::new(1),
            _ => BlockIndex::new(2),
        });

        partition.mark_element(StateIndex::new(7));
        partition.mark_element(StateIndex::new(8));
        let _ = partition.partition_marked_with(BlockIndex::new(2), &mut builder, |element, _| match element.value() {
            7 => BlockIndex::new(0),
            8 => BlockIndex::new(1),
            _ => BlockIndex::new(2),
        });
    }
}
