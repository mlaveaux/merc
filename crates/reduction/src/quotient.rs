#![forbid(unsafe_code)]

use log::trace;
use merc_collections::BlockIndex;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilder;
use merc_lts::LtsBuilderMem;
use merc_lts::StateIndex;
use merc_lts::reachability;

use crate::BlockPartition;
use crate::Partition;
use crate::diverges;

/// Returns a new LTS based on the given partition.
///
/// Computes the existential quotient of the given LTS based on the given
/// partition:
///
/// > \[p\] -a-> \[q\] iff there exist states s in p and t in q such that s -a-> t
///
/// If `eliminate_inert_taus` is true then non self-loop tau steps \[p\] -tau-> \[p\] are eliminated.
/// If `eliminate_tau_loops` is true then tau self-loops s -tau-> s are eliminated.
/// The two parameters are independent: each controls a disjoint set of transitions.
pub fn quotient_lts_naive<L: LTS, P: Partition>(
    lts: &L,
    partition: &P,
    eliminate_inert_taus: bool,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    // Introduce the transitions based on the block numbers. `lts.num_of_transitions()`
    // is an exact upper bound on the quotient's transition count (quotienting only
    // ever drops or merges transitions), and `partition.num_of_blocks()` is its exact
    // state count.
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        partition.num_of_blocks(),
        lts.num_of_transitions(),
    );

    for state_index in lts.iter_states() {
        for transition in lts.outgoing_transitions(state_index) {
            let block = partition.block_number(state_index);
            let to_block = partition.block_number(transition.to);

            // Eliminate non-self-loop inert taus and (independently) tau self-loops.
            if !(eliminate_inert_taus
                && lts.is_hidden_label(transition.label)
                && block == to_block
                && state_index != transition.to)
                && !(eliminate_tau_loops && lts.is_hidden_label(transition.label) && state_index == transition.to)
            {
                debug_assert!(
                    partition.block_number(state_index) < partition.num_of_blocks(),
                    "Quotienting assumes that the block numbers do not exceed the number of blocks"
                );

                builder
                    .add_transition(
                        StateIndex::new(block.value()),
                        &lts.labels()[transition.label],
                        StateIndex::new(to_block.value()),
                    )
                    .expect("Adding transitions does not fail");
            }
        }
    }

    builder.require_num_of_states(partition.num_of_blocks());
    builder.finish(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        true,
    )
}

/// Returns a weak bisimulation quotient that additionally removes transitions
/// subsumed by a one-hidden-step alternative.
///
/// If `eliminate_tau_loops` is true then tau self-loops are eliminated.
pub(crate) fn quotient_lts_weak<L: LTS, P: Partition>(
    lts: &L,
    partition: &P,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    let quotient = quotient_lts_naive(lts, partition, true, eliminate_tau_loops);
    remove_redundant_transitions(&quotient)
}

/// Weak bisimulation quotient that removes redundant transitions.
fn remove_redundant_transitions<L: LTS>(lts: &L) -> LabelledTransitionSystem<L::Label> {
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        lts.num_of_states(),
        lts.num_of_transitions(),
    );
    builder.require_num_of_states(lts.num_of_states());

    for from in lts.iter_states() {
        for transition in lts.outgoing_transitions(from) {
            if !is_redundant_transition(lts, from, transition.label, transition.to) {
                builder
                    .add_transition(from, &lts.labels()[transition.label], transition.to)
                    .expect("Adding transitions does not fail");
            } else {
                trace!(
                    "Removing redundant transition: {} -[{}]-> {}",
                    from,
                    lts.labels()[transition.label],
                    transition.to
                );
            }
        }
    }

    builder.finish(lts.initial_state_index(), true)
}

/// Returns true when transition `from -label-> target` is redundant.
///
/// A transition `s -a-> u` is redundant iff there exist states `t` and `v` such that:
/// - `s -tau*-> t`
/// - `t -a-> v` (for hidden `a`, any hidden transition)
/// - `v -tau*-> u`
///
/// The exact transition `s -a-> u` itself is not considered a witness.
fn is_redundant_transition<L: LTS>(lts: &L, from: StateIndex, label: LabelIndex, target: StateIndex) -> bool {
    let mut redundant = false;

    reachability(
        lts,
        from,
        |l| lts.is_hidden_label(l),
        |middle| {
            if redundant {
                return;
            }

            for transition in lts.outgoing_transitions(middle) {
                let same_action = if lts.is_hidden_label(label) {
                    lts.is_hidden_label(transition.label)
                } else {
                    transition.label == label
                };

                if !same_action {
                    continue;
                }

                // Skip the exact transition being tested.
                if middle == from && transition.label == label && transition.to == target {
                    continue;
                }

                // Skip tau self-loops on the middle state, as they do not contribute to the redundancy.
                if lts.is_hidden_label(transition.label) && middle == transition.to {
                    continue;
                }

                reachability(
                    lts,
                    transition.to,
                    |l| lts.is_hidden_label(l),
                    |reached| {
                        if reached == target {
                            redundant = true;
                        }
                    },
                );

                if redundant {
                    break;
                }
            }
        },
    );

    redundant
}

/// Optimised implementation for block partitions.
///
/// Chooses a single state in the block as representative. If `BRANCHING` then
/// the chosen state is a bottom state. For `BRANCHING` we only consider bottom
/// states as representatives.
///
/// If `eliminate_tau_loops` is true then tau self-loops are eliminated.
pub fn quotient_lts_block<L: LTS, const BRANCHING: bool>(
    lts: &L,
    partition: &BlockPartition,
    eliminate_tau_loops: bool,
) -> LabelledTransitionSystem<L::Label> {
    let mut builder = LtsBuilderMem::with_capacity(
        lts.labels().into(),
        Vec::new(),
        lts.num_of_labels(),
        partition.num_of_blocks(),
        lts.num_of_transitions(),
    );

    // Reused across blocks to find bottom states when BRANCHING.
    let mut visited = vec![false; lts.num_of_states()];
    // Only touched states are reset to avoid clearing the entire visited vector.
    let mut touched = Vec::new();

    for block in (0..partition.num_of_blocks()).map(BlockIndex::new) {
        // Pick any state in the block
        let mut candidate = if let Some(state) = partition.iter_block(block).next() {
            state
        } else {
            panic!("Blocks in the partition should not be empty {}", block);
        };

        if BRANCHING {
            // traverse any outgoing transition to find a bottom state.
            'outer: loop {
                if visited[candidate] {
                    // No bottom state exists in this block. Stop early to avoid looping forever.
                    debug_assert!(
                        !diverges(lts, candidate),
                        "The states of the given LTS should be non-divergent."
                    );
                    break;
                }
                visited[candidate] = true;
                touched.push(candidate);

                if let Some(trans) = lts.outgoing_transitions(candidate).find(|trans| {
                    lts.is_hidden_label(trans.label)
                        && candidate != trans.to // Ignore self loops for the bottom state search.
                        && partition.block_number(trans.to) == block
                }) {
                    candidate = trans.to;
                    continue 'outer;
                }

                // No outgoing tau transition to the same block, so we found a bottom state.
                break;
            }

            // Reset only the entries touched by this walk.
            for state in touched.drain(..) {
                visited[state] = false;
            }
        }

        // Add all transitions from the representative state (or the bottom state if BRANCHING) to the quotient LTS.
        for transition in lts.outgoing_transitions(candidate) {
            if BRANCHING {
                debug_assert!(
                    !(lts.is_hidden_label(transition.label)
                        && candidate != transition.to
                        && partition.block_number(transition.to) == block),
                    "The representative {} is not bottom state",
                    candidate
                );
            }

            if !(eliminate_tau_loops && lts.is_hidden_label(transition.label) && candidate == transition.to) {
                builder
                    .add_transition(
                        StateIndex::new(*block),
                        &lts.labels()[transition.label],
                        StateIndex::new(*partition.block_number(transition.to)),
                    )
                    .expect("Adding transitions does not fail");
            }
        }
    }

    builder.require_num_of_states(partition.num_of_blocks());
    builder.finish(
        StateIndex::new(partition.block_number(lts.initial_state_index()).value()),
        true,
    )
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_lts::LTS;
    use merc_lts::LabelIndex;
    use merc_lts::LabelledTransitionSystem;
    use merc_lts::StateIndex;
    use merc_lts::TransitionLabel;
    use merc_lts::random_lts;
    use merc_lts::write_aut;
    use merc_utilities::Timing;
    use merc_utilities::random_test;
    use rand::rngs::StdRng;

    use merc_collections::BlockIndex;

    use crate::BlockPartition;
    use crate::BlockPartitionBuilder;
    use crate::Equivalence;
    use crate::Partition;
    use crate::compare_lts;
    use crate::quotient_lts_block;
    use crate::reduce_lts;

    /// `quotient_lts_block::<_, true>` assumes every block has a genuine bottom state, which
    /// holds when the partition comes from the crate's own `reduce_lts` (it always collapses
    /// tau-SCCs first) but not in general -- the function is `pub` and takes a caller-supplied
    /// `BlockPartition` with no such precondition documented. This builds a block with a
    /// tau-cycle (1 -tau-> 2 -tau-> 1, reached via 0 -tau-> 1) and no bottom state; on current
    /// code the bottom-state search's cycle check is a `debug_assert!`, so this panics in debug
    /// builds (`#[ignore]`d below) and would silently pick a non-bottom representative in
    /// release builds, dropping that block's `x`-transition from the quotient.
    #[test]
    #[ignore = "known bug: quotient_lts_block panics (debug) / silently drops transitions (release) on a block with no bottom state, see review/phase-3-algorithmic-core.md"]
    fn test_quotient_lts_block_branching_uses_true_bottom_state() {
        // Label 0 = tau, label 1 = "x".
        // 0 -tau-> 1, 1 -tau-> 2, 2 -tau-> 1, 2 -x-> 3.
        let transitions = [(0, 0, 1), (1, 0, 2), (2, 0, 1), (2, 1, 3)]
            .map(|(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)));

        let lts = LabelledTransitionSystem::new(
            StateIndex::new(0),
            None,
            || transitions.iter().cloned(),
            vec![String::tau_label(), "x".to_string()],
        );

        // Hand-build a partition with block {0, 1, 2} and block {3}. This
        // bypasses signature refinement entirely, so the "no true bottom
        // state" case can be constructed directly rather than relying on it
        // arising from an actual (and, per above, impossible) run of
        // branching bisimulation reduction.
        let mut partition = BlockPartition::new(4);
        let mut builder = BlockPartitionBuilder::default();
        let _ = partition.partition_marked_with(BlockIndex::new(0), &mut builder, |state, _| {
            if state.value() == 3 {
                BlockIndex::new(1)
            } else {
                BlockIndex::new(0)
            }
        });

        let cyclic_block = partition.block_number(StateIndex::new(1));
        assert_eq!(
            partition.block_number(StateIndex::new(2)),
            cyclic_block,
            "test setup: states 1 and 2 should be in the same block"
        );
        assert_eq!(
            partition.block_number(StateIndex::new(0)),
            cyclic_block,
            "test setup: state 0 should be in the same block"
        );

        let quotient = quotient_lts_block::<_, true>(&lts, &partition, false);

        let x_label = LabelIndex::new(1);
        let cyclic_block_state = StateIndex::new(*cyclic_block);
        let has_x_transition = quotient
            .outgoing_transitions(cyclic_block_state)
            .any(|t| t.label == x_label);

        assert!(
            has_x_transition,
            "quotient_lts_block dropped the x-transition that is only directly available from \
             state 2; it picked a non-bottom state as the block's representative because the \
             block's tau-subgraph contains a cycle (states 1 and 2), which the bottom-state \
             search does not detect as \"no bottom state exists\" other than via a debug_assert \
             that panics in debug builds and is compiled out in release builds"
        );
    }

    /// Generates a random LTS, reduces it under `equivalence`, and asserts
    /// that the original and reduced LTS are equivalent.
    fn check_quotient_equivalence(rng: &mut StdRng, equivalence: Equivalence, test_name: &str) {
        let timing = Timing::new();
        let files = DumpFiles::new(test_name);

        let lts = random_lts::<String, _>(rng, 100, 3);

        files.dump("input.aut", |w| write_aut(w, &lts)).unwrap();

        let reduced = reduce_lts(lts.clone(), equivalence, false, &timing);
        files.dump("quotient.aut", |w| write_aut(w, &reduced)).unwrap();

        assert!(
            compare_lts(equivalence, lts, reduced, false, false, &timing).0,
            "Quotient is not equivalent under {equivalence:?}",
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_strong_bisim_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(rng, Equivalence::StrongBisim, "test_random_strong_bisim_quotient");
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_strong_bisim_naive_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::StrongBisimNaive,
                "test_random_strong_bisim_naive_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_branching_bisim_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(rng, Equivalence::BranchingBisim, "test_random_branching_bisim_quotient");
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_branching_bisim_naive_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::BranchingBisimNaive,
                "test_random_branching_bisim_naive_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(rng, Equivalence::WeakBisim, "test_random_weak_bisim_quotient");
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_parallel_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimParallel,
                "test_random_weak_bisim_parallel_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_sigref_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimSigref,
                "test_random_weak_bisim_sigref_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_sigref_naive_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimSigrefNaive,
                "test_random_weak_bisim_sigref_naive_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_divergence_preserving_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimDivergencePreserving,
                "test_random_weak_bisim_divergence_preserving_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_parallel_divergence_preserving_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimParallelDivergencePreserving,
                "test_random_weak_bisim_parallel_divergence_preserving_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_sigref_divergence_preserving_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimSigrefDivergencePreserving,
                "test_random_weak_bisim_sigref_divergence_preserving_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_weak_bisim_sigref_naive_divergence_preserving_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::WeakBisimSigrefNaiveDivergencePreserving,
                "test_random_weak_bisim_sigref_naive_divergence_preserving_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_branching_bisim_divergence_preserving_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::BranchingBisimDivergencePreserving,
                "test_random_branching_bisim_divergence_preserving_quotient",
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_branching_bisim_divergence_preserving_naive_quotient() {
        random_test(100, |rng| {
            check_quotient_equivalence(
                rng,
                Equivalence::BranchingBisimDivergencePreservingNaive,
                "test_random_branching_bisim_divergence_preserving_naive_quotient",
            );
        });
    }
}
