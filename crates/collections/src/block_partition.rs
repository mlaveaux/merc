use std::fmt;

use itertools::Itertools;
use log::trace;

use crate::BlockIndex;
use crate::IndexedPartition;

/// A partition that explicitly stores a list of blocks and their indexing into
/// the list of elements.
#[derive(Debug)]
pub struct BlockPartition<A: Clone + fmt::Debug = ()> {
    elements: Vec<usize>,
    blocks: Vec<Block<A>>,
}

impl<A: Clone + fmt::Debug + Default> BlockPartition<A> {
    /// Creates a partition where all elements are in a single block.
    ///
    /// # Panics
    ///
    /// Panics if `num_of_elements == 0`.
    pub fn new(num_of_elements: usize) -> Self {
        debug_assert!(num_of_elements > 0, "Cannot partition the empty set");

        let blocks = vec![Block::new(0, num_of_elements)];
        let elements = (0..num_of_elements).collect();

        Self { elements, blocks }
    }
}

impl<A: Clone + fmt::Debug + Default> BlockPartition<A> {
    /// Create a block partition from an indexed partition.
    ///
    /// This function creates a new partition that is dense, i.e. it does not
    /// contain empty blocks, and the blocks are indexed from 0 to n-1 even if the
    /// indexed partition is sparse.
    pub fn from_indexed_partition(partition: &IndexedPartition) -> Self {
        let mut blocks = vec![Block::new_empty(); partition.num_of_blocks()];
        let num_of_elements = partition.iter_elements().count();

        // Figure out the number of elements per block.
        for (_, block_index) in partition.iter_elements() {
            blocks[block_index].end += 1;
        }

        // Compute the start index for each block.
        let mut start = 0;
        for block in &mut blocks {
            let end = block.end;
            block.begin = start;
            block.end = start; // This will be updated when adding elements.
            start += end;
        }

        // Create the elements vector.
        let mut elements = vec![0; num_of_elements];
        for (element_index, block_index) in partition.iter_elements() {
            // Add the element to the block, and update the end index.
            let block = &mut blocks[block_index];
            let pos = block.end;
            elements[pos] = element_index;
            block.end = pos + 1;
        }

        // Remove empty blocks.
        blocks.retain(|block| !block.is_empty());

        Self { elements, blocks }
    }

    /// Returns a reference to the given block.
    ///
    /// # Panics
    ///
    /// Panics if `block_index` is out of range.
    pub fn block(&self, block_index: BlockIndex) -> &Block<A> {
        &self.blocks[block_index]
    }

    /// Returns a mutable reference to the block's annotation.
    ///
    /// # Panics
    ///
    /// Panics if `block_index` is out of range.
    pub fn block_annotation(&mut self, block_index: BlockIndex) -> &mut A {
        self.blocks[block_index].annotation_mut()
    }

    /// Splits a block into two blocks according to the given predicate. If the
    /// predicate holds for all or none of the elements, no split occurs.
    ///
    /// # Panics
    ///
    /// Panics if `block_index` is out of range.
    pub fn split_block<F>(&mut self, block_index: BlockIndex, predicate: F) -> Option<BlockIndex>
    where
        F: Fn(usize) -> bool,
    {
        // Swaps elements in the block so that the elements for which the
        // predicate holds are at the beginning of the block.
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
        let new_block = Block::new(self.blocks[block_index].begin + size, self.blocks[block_index].end);
        let last_block = self.blocks.len();
        self.blocks.push(new_block);

        // Update the original block
        let new_end = self.blocks[block_index].begin + size;
        self.blocks[block_index].end = new_end;

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
    ///
    /// # Panics
    ///
    /// Panics if `block_index` is out of range.
    pub fn iter_block(&self, block_index: BlockIndex) -> BlockIter<'_> {
        BlockIter {
            elements: &self.elements,
            index: self.blocks[block_index].begin,
            end: self.blocks[block_index].end,
        }
    }

    /// Returns an iterator over all blocks in the partition.
    pub fn iter(&self) -> impl Iterator<Item = &Block<A>> {
        self.blocks.iter()
    }

    /// Returns an iterator over all blocks in the partition.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Block<A>> {
        self.blocks.iter_mut()
    }

    /// Returns the number of elements in the partition.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns true iff the partition is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl<B: Clone + fmt::Debug> fmt::Display for BlockPartition<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format = self
            .blocks
            .iter()
            .map(|block| format!("{{{}}}", block.iter(&self.elements).format(", ")))
            .format(", ");

        write!(f, "{{{}}}", format)
    }
}
/// A block that stores a subset of the elements in a partition as a `begin..end` range.
#[derive(Clone, Copy, Debug)]
pub struct Block<A: Clone + fmt::Debug> {
    begin: usize,
    end: usize,
    annotation: A,
}

impl<A: Clone + fmt::Debug + Default> Block<A> {
    pub fn new(begin: usize, end: usize) -> Self {
        debug_assert!(begin < end, "The range of this block is incorrect {begin}..{end}");
        Block {
            begin,
            end,
            annotation: Default::default(),
        }
    }

    /// Create an empty block at the given position.
    fn new_empty() -> Self {
        Block {
            begin: 0,
            end: 0,
            annotation: Default::default(),
        }
    }
}

impl<A: Clone + fmt::Debug> Block<A> {
    /// Returns an iterator over the elements in this block.
    pub fn iter<'a>(&self, elements: &'a [usize]) -> impl Iterator<Item = usize> + 'a {
        BlockIter {
            elements,
            index: self.begin,
            end: self.end,
        }
    }

    /// Returns the underlying annotation of this block.
    pub fn annotation(&self) -> &A {
        &self.annotation
    }

    /// Returns the underlying annotation of this block.
    pub fn annotation_mut(&mut self) -> &mut A {
        &mut self.annotation
    }

    /// Returns the number of elements in the block.
    pub fn len(&self) -> usize {
        self.assert_consistent();
        self.end - self.begin
    }

    /// Returns true iff the block is empty.
    pub fn is_empty(&self) -> bool {
        self.begin == self.end
    }

    /// Returns true iff the block is consistent.
    fn assert_consistent(&self) {
        debug_assert!(self.begin <= self.end, "The range of block {self:?} is incorrect");
    }
}

pub struct BlockIter<'a> {
    elements: &'a [usize],
    index: usize,
    end: usize,
}

impl Iterator for BlockIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.end {
            let element = self.elements[self.index];
            self.index += 1;
            Some(element)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BlockIter<'_> {}

#[cfg(kani)]
mod verification {
    use super::*;

    /// Small, fixed-size element domain the harness below explores
    /// exhaustively (`kani::any` enumerates every one of the `2^N` possible
    /// predicate masks, not just a random sample of them).
    const N: usize = 5;

    /// Proves `split_block`'s core partition-refinement invariant, for every
    /// one of the `2^N` possible predicates over a single-block partition of
    /// `N` elements:
    ///
    /// - if the predicate holds for all or none of the elements, no split
    ///   occurs and the partition is unchanged (matching the doc comment);
    /// - otherwise, every element left in the original block satisfies the
    ///   predicate, every element moved to the new block does not, and the
    ///   in-place swap that achieves this is a genuine partition: each of the
    ///   `N` original elements appears in exactly one of the two blocks
    ///   afterwards (never lost, never duplicated).
    #[kani::proof]
    #[kani::unwind(8)]
    fn split_block_refines_correctly_and_preserves_elements() {
        let mut partition: BlockPartition<()> = BlockPartition::new(N);
        let block_index = BlockIndex::new(0);

        // `mask[e]` is the predicate's verdict for element `e`; elements are
        // identity-numbered 0..N at this point, so this doubles as a lookup
        // table indexed directly by element value.
        let mask: [bool; N] = kani::any();
        let predicate = |element: usize| mask[element];

        let true_count = mask.iter().filter(|&&matched| matched).count();

        let result = partition.split_block(block_index, predicate);

        if true_count == 0 || true_count == N {
            assert!(
                result.is_none(),
                "a predicate matching all or none of the elements must not split"
            );
            assert_eq!(partition.num_of_blocks(), 1);
            assert_eq!(partition.block(block_index).len(), N);
        } else {
            let new_block = result.expect("a predicate matching some but not all elements must split");
            assert_eq!(partition.num_of_blocks(), 2);
            assert_eq!(partition.block(block_index).len(), true_count);
            assert_eq!(partition.block(new_block).len(), N - true_count);

            // Every element now in the original block satisfies the predicate...
            for element in partition.iter_block(block_index) {
                assert!(
                    mask[element],
                    "element {element} in the matching block fails the predicate"
                );
            }
            // ...and every element in the new block does not.
            for element in partition.iter_block(new_block) {
                assert!(
                    !mask[element],
                    "element {element} in the non-matching block satisfies the predicate"
                );
            }

            // The swap-based partition is a genuine permutation of 0..N:
            // every original element still appears, and appears exactly once,
            // across the two blocks combined.
            let mut seen = [false; N];
            for element in partition.iter_block(block_index).chain(partition.iter_block(new_block)) {
                assert!(
                    !seen[element],
                    "element {element} appears more than once after split_block"
                );
                seen[element] = true;
            }
            assert!(
                seen.iter().all(|&was_seen| was_seen),
                "an element was lost by split_block's in-place swap"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use log::trace;
    use merc_utilities::random_test;
    use rand::RngExt;
    use rand::seq::IteratorRandom;

    use crate::BlockIndex;
    use crate::IndexedPartition;

    use super::BlockPartition;

    #[test]
    fn test_simple_block_partition() {
        let mut partition: BlockPartition<()> = BlockPartition::new(10);

        assert_eq!(partition.num_of_blocks(), 1);

        let initial_block = BlockIndex::new(0);
        assert_eq!(partition.block(initial_block).len(), 10);

        let block_index = partition.split_block(BlockIndex::new(0), |state| state < 5).unwrap();

        assert_eq!(partition.num_of_blocks(), 2);
        assert_eq!(partition.block(initial_block).len(), 5);
        assert_eq!(partition.block(block_index).len(), 5);
    }

    #[test]
    fn test_random_from_indexed_partition() {
        random_test(100, |rng| {
            let subset = (0..100).sample(rng, 25);
            let mut partition = IndexedPartition::with_subset(100, subset.iter().copied());

            for element in subset {
                partition.set_block(element, BlockIndex::new(rng.random_range(0..10)));
            }
            trace!("Input partition {partition}");

            let block_partition: BlockPartition<()> = BlockPartition::from_indexed_partition(&partition);
            trace!("Output partition {block_partition}");

            // The block partition must contain exactly the same number of
            // elements as the indexed partition (regression: was sized by
            // num_of_blocks instead of num_of_elements).
            assert_eq!(
                block_partition.len(),
                partition.iter_elements().count(),
                "block_partition.len() does not match the number of elements in the indexed partition"
            );

            // Every element in the indexed partition must appear in the block
            // partition exactly once.
            let mut seen = [false; 100];
            for block in 0..block_partition.num_of_blocks() {
                for element in block_partition.iter_block(BlockIndex::new(block)) {
                    assert!(
                        !seen[element],
                        "Element {element} appears more than once in block partition"
                    );
                    seen[element] = true;
                }
            }
            for (element, _) in partition.iter_elements() {
                assert!(seen[element], "Element {element} is missing from block partition");
            }

            // Check that each block in the block partition contains only
            // elements from the same indexed-partition block.
            for block in 0..block_partition.num_of_blocks() {
                let mut elements = block_partition.iter_block(BlockIndex::new(block));
                let first = elements.next().unwrap();
                let expected_block = partition.block(first);

                for element in elements {
                    assert_eq!(
                        partition.block(element),
                        expected_block,
                        "Block {block} contains elements from different indexed-partition blocks"
                    );
                    assert_eq!(
                        partition.block(element),
                        expected_block,
                        "Block {block} contains elements from different indexed-partition blocks"
                    );
                }
            }
        })
    }
}
