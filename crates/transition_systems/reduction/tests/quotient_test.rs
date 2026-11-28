use std::collections::BTreeSet;

use rand::RngExt;

use merc_collections::BlockIndex;
use merc_collections::IndexedPartition;
use merc_lts::LTS;
use merc_lts::random_lts;
use merc_reduction::quotient_lts_naive;
use merc_utilities::random_test;

/// The naive quotient has a state per block and a transition [p] -a-> [q] iff some s in p
/// has s -a-> t with t in q, minus the tau steps that the flags eliminate.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_quotient_lts_naive() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 20, 3);

        let num_of_blocks = rng.random_range(1..=lts.num_of_states());
        // Every block is non-empty: the first states fill the blocks in order.
        let blocks: Vec<BlockIndex> = (0..lts.num_of_states())
            .map(|s| {
                BlockIndex::new(if s < num_of_blocks {
                    s
                } else {
                    rng.random_range(0..num_of_blocks)
                })
            })
            .collect();
        let partition = IndexedPartition::with_partition(blocks.clone(), num_of_blocks);

        for eliminate_inert_taus in [false, true] {
            for eliminate_tau_loops in [false, true] {
                let quotient = &quotient_lts_naive(&lts, &partition, eliminate_inert_taus, eliminate_tau_loops);

                let mut expected = BTreeSet::new();
                for s in lts.iter_states() {
                    for t in lts.outgoing_transitions(s) {
                        let inert = lts.is_hidden_label(t.label) && blocks[s] == blocks[t.to];
                        if inert && ((s == t.to && eliminate_tau_loops) || (s != t.to && eliminate_inert_taus)) {
                            continue;
                        }
                        expected.insert((
                            blocks[s].value(),
                            lts.labels()[t.label.value()].clone(),
                            blocks[t.to].value(),
                        ));
                    }
                }

                let actual: BTreeSet<_> = quotient
                    .iter_states()
                    .flat_map(|p| {
                        quotient
                            .outgoing_transitions(p)
                            .map(move |t| (p.value(), quotient.labels()[t.label.value()].clone(), t.to.value()))
                    })
                    .collect();

                assert_eq!(quotient.num_of_states(), num_of_blocks);
                assert_eq!(
                    quotient.initial_state_index().value(),
                    blocks[lts.initial_state_index()].value()
                );
                assert_eq!(
                    actual, expected,
                    "inert {eliminate_inert_taus}, loops {eliminate_tau_loops}"
                );
                assert_eq!(quotient.num_of_transitions(), expected.len());
            }
        }
    });
}
