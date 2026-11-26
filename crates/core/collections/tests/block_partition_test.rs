use std::collections::HashSet;

use rand::RngExt;
use rand::seq::IteratorRandom;

use merc_collections::BlockIndex;
use merc_collections::BlockPartition;
use merc_collections::IndexedPartition;
use merc_utilities::random_test;

/// The elements of every block, each block sorted.
fn blocks(partition: &BlockPartition) -> Vec<Vec<usize>> {
    (0..partition.num_of_blocks())
        .map(|block| {
            let mut elements: Vec<usize> = partition.iter_block(BlockIndex::new(block)).collect();
            assert_eq!(elements.len(), partition.block(BlockIndex::new(block)).len());
            elements.sort_unstable();
            elements
        })
        .collect()
}

#[test]
fn test_simple_block_partition() {
    let mut partition: BlockPartition = BlockPartition::new(10);
    assert_eq!(blocks(&partition), [(0..10).collect::<Vec<_>>()]);

    let initial_block = BlockIndex::new(0);
    let block_index = partition.split_block(initial_block, |element| element < 5).unwrap();
    assert_eq!(blocks(&partition), [vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]]);
    assert_eq!(block_index, BlockIndex::new(1));

    // A predicate that holds for all or none of the elements does not split.
    assert_eq!(partition.split_block(block_index, |element| element > 4), None);
    assert_eq!(partition.split_block(block_index, |_| false), None);
    assert_eq!(partition.num_of_blocks(), 2);
    assert_eq!(partition.len(), 10);
}

/// Random splits checked against a model that stores the blocks as sets.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_split_block() {
    random_test(100, |rng| {
        let num_of_elements = rng.random_range(1..50);
        let mut partition: BlockPartition = BlockPartition::new(num_of_elements);
        let mut model: Vec<Vec<usize>> = vec![(0..num_of_elements).collect()];

        for _ in 0..20 {
            let block = rng.random_range(0..model.len());
            let selected: HashSet<usize> = (0..num_of_elements).filter(|_| rng.random_bool(0.5)).collect();

            let (kept, moved): (Vec<usize>, Vec<usize>) =
                model[block].iter().partition(|element| selected.contains(element));
            let result = partition.split_block(BlockIndex::new(block), |element| selected.contains(&element));
            if kept.is_empty() || moved.is_empty() {
                assert_eq!(result, None);
            } else {
                assert_eq!(result, Some(BlockIndex::new(model.len())));
                model[block] = kept;
                model.push(moved);
            }
            assert_eq!(blocks(&partition), model);
        }
    })
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_from_indexed_partition() {
    random_test(100, |rng| {
        let subset = (0..100).sample(rng, 25);
        let mut partition = IndexedPartition::with_subset(100, subset.iter().copied());
        for element in subset {
            partition.set_block(element, BlockIndex::new(rng.random_range(0..10)));
        }

        let block_partition: BlockPartition = BlockPartition::from_indexed_partition(&partition);

        // Every non-empty block of the indexed partition is exactly one block of the result.
        let mut expected: Vec<Vec<usize>> = (0..10)
            .map(|block| {
                partition
                    .iter_elements()
                    .filter(|&(_, element_block)| element_block == BlockIndex::new(block))
                    .map(|(element, _)| element)
                    .collect::<Vec<_>>()
            })
            .filter(|block| !block.is_empty())
            .collect();
        let mut actual = blocks(&block_partition);
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        assert_eq!(block_partition.len(), partition.iter_elements().count());
    })
}
