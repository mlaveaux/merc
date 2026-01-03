//! Authors: Maurice Laveaux, Eduardo Costa Martins
//!
//! Implements the weak bisimulation algorithm by Eduardo Costa Martins.
#![forbid(unsafe_code)]

use bitvec::bitvec;
use bitvec::order::Lsb0;

use bitvec::vec::BitVec;
use log::info;
use log::trace;
use merc_io::TimeProgress;
use merc_lts::IncomingTransitions;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_utilities::Timing;

use crate::BlockIndex;
use crate::SimpleBlockPartition;
use crate::preprocess_branching;

/// Type alias because we use bitvec for marking states
type BitArray = BitVec<u64, Lsb0>;

/// Apply weak bisimulation reduction
pub fn weak_bisimulation<L: LTS>(
    lts: L,
    timing: &mut Timing,
) -> (LabelledTransitionSystem<L::Label>, SimpleBlockPartition) {
    let mut time_pre = timing.start("preprocessing");
    let tau_loop_free_lts = preprocess_branching(lts);
    time_pre.finish();

    let mut time_reduction = timing.start("reduction");
    let mut blocks = SimpleBlockPartition::new(tau_loop_free_lts.num_of_states());

    let mut act_mark = bitvec![u64, Lsb0; 0; tau_loop_free_lts.num_of_states()];
    let mut tau_mark = bitvec![u64, Lsb0; 0; tau_loop_free_lts.num_of_states()];

    let incoming = IncomingTransitions::new(&tau_loop_free_lts);

    let progress = TimeProgress::new(
        |num_of_blocks: usize| {
            info!("Found {} blocks...", num_of_blocks);
        },
        1,
    );

    loop {
        let mut stable = true;
        for block_index in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
            progress.print(blocks.num_of_blocks());
            if blocks.block(block_index).is_stable() {
                continue;
            }

            trace!("Stabilising block {:?}", block_index);
            stable = false;
            blocks.mark_block_stable(block_index);

            // tau is the first label.
            for label in tau_loop_free_lts
                .labels()
                .iter()
                .enumerate()
                .map(|(i, _)| LabelIndex::new(i))
            {
                compute_weak_act(
                    &mut act_mark,
                    &mut tau_mark,
                    &tau_loop_free_lts,
                    &blocks,
                    &incoming,
                    block_index,
                    label,
                );

                // Note that we cannot use the block references here, and instead uses indices, because stabilise
                // also modifies the blocks structure.
                for block_prime in (0usize..blocks.num_of_blocks()).map(BlockIndex::new) {
                    stabilise(block_prime, &mut act_mark, &mut blocks);
                }
            }
        }

        if stable {
            // Quit the outer loop.
            trace!("Partition is stable!");
            break;
        }
    }

    time_reduction.finish();
    (tau_loop_free_lts, blocks)
}

/// Sets s.act_mark to true iff exists t: S. s =\not{a}=> t
/// If a = tau, then also updates s.tau_mark
fn compute_weak_act(
    act_mark: &mut BitArray,
    tau_mark: &mut BitArray,
    lts: &impl LTS,
    blocks: &SimpleBlockPartition,
    incoming: &IncomingTransitions,
    block: BlockIndex,
    label: LabelIndex,
) {
    for s in lts.iter_states() {
        // s.act_mark := true iff s in B && a == tau
        act_mark.set(
            *s,
            lts.is_hidden_label(label) && blocks.iter_block(block).any(|state| state == s),
        );

        for transition in lts.outgoing_transitions(s) {
            if transition.label == label {
                // s.act_mark := true iff a != tau && tau_mark[t]
                if !lts.is_hidden_label(transition.label) && tau_mark[*transition.to] {
                    act_mark.set(*s, true);
                }
            }
        }
    }

    for t in lts.iter_states() {
        // t.tau_mark := t.act_mark if a == tau
        if lts.is_hidden_label(label) {
            tau_mark.set(*t, act_mark[*t]);
        }

        if act_mark[*t] {
            for transition in incoming.incoming_silent_transitions(t) {
                act_mark.set(*transition.to, true);
            }
        }
    }
}

/// Splits the given block according to the given marking.
fn stabilise(block: BlockIndex, act_mark: &mut BitArray, blocks: &mut SimpleBlockPartition) {
    blocks.split_block(block, |state| act_mark[*state]);
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_lts::LTS;
    use merc_lts::random_lts;
    use merc_lts::write_aut;
    use merc_utilities::Timing;
    use merc_utilities::random_test;

    use crate::Equivalence;
    use crate::compare_lts;
    use crate::reduce_lts;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_weak_bisimulation() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_weak_bisimulation");

            let lts = random_lts(rng, 2, 10, 3);
            let mut timing = Timing::new();
            files.dump("input.aut", |f| write_aut(f, &lts)).unwrap();

            let result = reduce_lts(lts.clone(), Equivalence::WeakBisim, &mut timing);
            let expected = reduce_lts(lts, Equivalence::WeakBisimSigref, &mut timing);

            assert_eq!(result.num_of_states(), expected.num_of_states());
            assert_eq!(result.num_of_transitions(), expected.num_of_transitions());

            files.dump("reduced.aut", |f| write_aut(f, &result)).unwrap();
            files.dump("expected.aut", |f| write_aut(f, &expected)).unwrap();

            assert!(compare_lts(Equivalence::StrongBisim, result, expected, &mut timing));
        })
    }
}
