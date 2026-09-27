use std::fmt;

use merc_utilities::TagIndex;

/// A zero sized tag for the block.
pub struct BlockTag {}

/// The index for blocks.
pub type BlockIndex = TagIndex<usize, BlockTag>;

/// Special value used for elements that do not occur in the partition.
pub(crate) const NOT_IN_PARTITION: usize = usize::MAX;

/// Defines a partition based on an explicit indexing of elements to their block
/// number.
///
/// Always stores one block number per element up to `max_value`, using a special value for
/// elements not in the partition - not memory efficient for sparse partitions.
#[derive(Clone, Debug)]
pub struct IndexedPartition {
    /// Stores a mapping from element index to block number.
    partition: Vec<BlockIndex>,

    /// Keeps track of the number of blocks defined.
    num_of_blocks: usize,
}

impl IndexedPartition {
    /// Create a new partition with 0 to `num_of_elements` in a single block.
    pub fn new(num_of_elements: usize) -> IndexedPartition {
        IndexedPartition {
            partition: vec![BlockIndex::new(0); num_of_elements],
            num_of_blocks: 1,
        }
    }

    /// Create a new partition where the given indices are in a single block.
    ///
    /// # Panics
    ///
    /// Panics if any of `included_indices` is `>= num_of_elements`.
    pub fn with_subset<I>(num_of_elements: usize, included_indices: I) -> IndexedPartition
    where
        I: IntoIterator<Item = usize>,
    {
        let mut partition = vec![BlockIndex::new(NOT_IN_PARTITION); num_of_elements];
        let mut has_included_indices = false;

        for index in included_indices {
            partition[index] = BlockIndex::new(0);
            has_included_indices = true;
        }

        IndexedPartition {
            partition,
            num_of_blocks: usize::from(has_included_indices),
        }
    }

    /// Create a new partition with the given partitioning.
    pub fn with_partition(partition: Vec<BlockIndex>, num_of_blocks: usize) -> IndexedPartition {
        debug_assert!(
            partition
                .iter()
                .all(|&block| block.value() < num_of_blocks || block.value() == NOT_IN_PARTITION),
            "Block numbers must be less than the number of blocks, or equal to NOT_IN_PARTITION"
        );

        IndexedPartition {
            partition,
            num_of_blocks,
        }
    }

    /// Iterates over the blocks in the partition, blocks can be repeated.
    pub fn iter(&self) -> impl Iterator<Item = BlockIndex> + '_ {
        self.iter_elements().map(|(_, block)| block)
    }

    /// Iterates over element-block pairs in the partition.
    pub fn iter_elements(&self) -> impl Iterator<Item = (usize, BlockIndex)> + '_ {
        self.partition
            .iter()
            .enumerate()
            .filter_map(|(element_index, &block)| (block.value() != NOT_IN_PARTITION).then_some((element_index, block)))
    }

    /// Sets the block number of the given element.
    ///
    /// Assumes block numbers are dense; otherwise `num_of_blocks` overestimates the number of
    /// blocks actually present.
    ///
    /// # Panics
    ///
    /// Panics if `element_index >= len()`.
    pub fn set_block(&mut self, element_index: usize, block_number: BlockIndex) {
        debug_assert!(
            block_number.value() != NOT_IN_PARTITION,
            "Block number cannot be NOT_IN_PARTITION"
        );

        self.num_of_blocks = self.num_of_blocks.max(block_number.value() + 1);
        self.partition[element_index] = block_number;
    }

    /// Returns the block number of the given element.
    ///
    /// # Panics
    ///
    /// Panics if `element_index >= len()`.
    pub fn block(&self, element_index: usize) -> BlockIndex {
        self.partition[element_index]
    }

    /// Returns the number of elements in the partition.
    pub fn len(&self) -> usize {
        self.partition.len()
    }

    /// Returns whether the partition is empty.
    pub fn is_empty(&self) -> bool {
        self.partition.is_empty()
    }

    /// Returns the number of blocks in the partition.
    pub fn num_of_blocks(&self) -> usize {
        self.num_of_blocks
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    /// Small, fixed element domain the harness below explores exhaustively:
    /// `kani::any` enumerates every combination of element/block-number pair
    /// bounded by `N`, for all 3 iterations of the loop, not a random sample.
    const N: usize = 4;

    /// Proves `set_block`/`num_of_blocks`'s bookkeeping against a plain
    /// reference model, for every combination of 3 `set_block` calls over
    /// `N` elements and block numbers bounded by `N`: `block(i)` always
    /// returns the most recently set block number for `i` (or block 0,
    /// `new`'s initial state, if `i` was never set), and `num_of_blocks()`
    /// always equals one plus the highest block number ever passed to
    /// `set_block` -- the "block numbers are dense" contract `set_block`'s
    /// own doc comment claims, which real callers depend on as an accurate
    /// upper bound (e.g. `BlockPartition::from_indexed_partition` sizes its
    /// block array directly from `num_of_blocks()`).
    #[kani::proof]
    #[kani::unwind(5)]
    fn set_block_and_num_of_blocks_match_reference_model() {
        let mut partition = IndexedPartition::new(N);
        let mut expected = [0usize; N];
        let mut expected_num_of_blocks = 1usize;

        for _ in 0..3 {
            let element: usize = kani::any();
            let block: usize = kani::any();
            kani::assume(element < N);
            kani::assume(block < N);

            partition.set_block(element, BlockIndex::new(block));
            expected[element] = block;
            expected_num_of_blocks = expected_num_of_blocks.max(block + 1);
        }

        assert_eq!(
            partition.num_of_blocks(),
            expected_num_of_blocks,
            "num_of_blocks() must equal one plus the highest block number ever set"
        );
        for i in 0..N {
            assert_eq!(
                partition.block(i).value(),
                expected[i],
                "block({i}) does not match the reference model after the set_block sequence"
            );
        }
    }

}

/// Reorders the blocks of the given partition according to the given permutation.
#[allow(dead_code)]
pub(crate) fn reorder_partition<P>(partition: IndexedPartition, permutation: P) -> IndexedPartition
where
    P: Fn(BlockIndex) -> BlockIndex,
{
    let mut new_partition = partition.clone();

    for (element_index, block) in partition.iter_elements() {
        new_partition.set_block(element_index, permutation(block));
    }

    new_partition
}

impl fmt::Display for IndexedPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{ ")?;

        let mut first = true;

        for block_index in self.iter() {
            // Print all elements with the same block number.
            let mut first_block = true;
            for (element_index, _) in self.iter_elements().filter(|&(_, value)| value == block_index) {
                if !first_block {
                    write!(f, ", ")?;
                } else {
                    if !first {
                        write!(f, ", ")?;
                    }

                    write!(f, "{{")?;
                }

                write!(f, "{element_index}")?;
                first_block = false;
            }

            if !first_block {
                write!(f, "}}")?;
                first = false;
            }
        }

        write!(f, " }}")
    }
}
