//! Authors: Jan Friso Groote, Maurice Laveaux, Wieger Wesselink and Tim A.C. Willemse
//! This file contains an implementation of
//! M. Laveaux, J.F. Groote and T.A.C. Willemse
//! Correct and Efficient Antichain Algorithms for Refinement Checking. Logical Methods in Computer Science 17(1) 2021
//!
//! There are six algorithms. One for trace inclusion, one for failures inclusion and one for failures-divergence
//! inclusion. All algorithms come in a variant with and without internal steps. It is possible to generate a counter
//! transition system in case the inclusion is answered by no.

use log::trace;
use merc_collections::VecSet;
use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_reduction::Equivalence;
use merc_reduction::Partition;
use merc_reduction::quotient_lts_block;
use merc_reduction::reduce_lts;
use merc_reduction::strong_bisim_sigref;
use merc_utilities::Timing;

use crate::Antichain;
use crate::RefinementType;

/// Sets the exploration strategy for the failures refinement algorithm.
pub enum ExplorationStrategy {
    BFS,
    DFS,
}

/// This function checks using algorithms in the paper mentioned above
/// whether transition system l1 is included in transition system l2, in the
/// sense of trace inclusions, failures inclusion and divergence failures
/// inclusion.
pub fn is_failures_refinement<L: LTS, const COUNTER_EXAMPLE: bool>(
    impl_lts: L,
    spec_lts: L,
    refinement: RefinementType,
    _strategy: ExplorationStrategy,
    preprocess: bool,
    timing: &mut Timing,
) -> bool {
    let reduction = match refinement {
        RefinementType::Trace => Equivalence::StrongBisim,
    };

    // For the preprocessing/quotienting step it makes sense to merge both LTSs
    // together in case that some states are equivalent. So we do this in all branches.
    let (merged_lts, initial_spec) = if preprocess {
        if COUNTER_EXAMPLE {
            // If a counter example is to be generated, we only reduce the
            // specification LTS such that the trace remains valid.
            let reduced_spec = reduce_lts(spec_lts, reduction, timing);
            impl_lts.merge_disjoint(&reduced_spec)
        } else {
            let (merged_lts, initial_spec) = impl_lts.merge_disjoint(&spec_lts);

            // Reduce all states in the merged LTS.
            match reduction {
                Equivalence::StrongBisim => {
                    let (preprocess_lts, partition) = strong_bisim_sigref(merged_lts, timing);

                    let initial_spec = partition.block_number(initial_spec);
                    let reduced_lts = quotient_lts_block::<_, false>(&preprocess_lts, &partition);

                    // After partitioning the block becomes the state in the reduced_lts.
                    (reduced_lts, StateIndex::new(*initial_spec))
                }
                _ => unimplemented!(),
            }
        }
    } else {
        impl_lts.merge_disjoint(&spec_lts)
    };

    let mut working = vec![(merged_lts.initial_state_index(), VecSet::singleton(initial_spec))];

    // The antichain data structure is used for storing explored states. However, as opposed to a discovered set it
    // allows for pruning additional pairs based on the `antichain` property.
    let mut antichain = Antichain::new();

    while let Some((impl_state, spec)) = working.pop() {
        trace!("Checking ({:?}, {:?})", impl_state, spec);
        // pop (impl,spec) from working;

        for impl_transition in merged_lts.outgoing_transitions(impl_state) {
            // spec' := {s' | exists s in spec. s-e->s'};
            let mut spec_prime = VecSet::new();
            for s in &spec {
                for spec_transition in merged_lts.outgoing_transitions(*s) {
                    if impl_transition.label == spec_transition.label {
                        spec_prime.insert(spec_transition.to);
                    }
                }
            }

            trace!("spec' = {:?}", spec_prime);
            if spec_prime.is_empty() {
                // if spec' = {} then
                return false; //    return false;
            }

            if antichain.insert(impl_transition.to, spec_prime.clone()) {
                // if antichain_insert(impl,spec') then
                working.push((impl_transition.to, spec_prime));
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_lts::random_lts;
    use merc_lts::write_aut;
    use merc_reduction::Equivalence;
    use merc_reduction::reduce_lts;
    use merc_utilities::Timing;
    use merc_utilities::random_test;

    use crate::ExplorationStrategy;
    use crate::RefinementType;
    use crate::is_failures_refinement;

    #[test]
    #[cfg_attr(miri, ignore)] // Tests are too slow under miri.
    fn test_random_trace_refinement() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_trace_refinement");

            let spec_lts = random_lts(rng, 10, 20, 5);

            let mut timing = Timing::default();
            let impl_lts = reduce_lts(spec_lts.clone(), Equivalence::StrongBisim, &mut timing);

            files.dump("spec.aut", |w| write_aut(w, &spec_lts)).unwrap();
            files.dump("impl.aut", |w| write_aut(w, &impl_lts)).unwrap();

            assert!(
                is_failures_refinement::<_, false>(
                    impl_lts,
                    spec_lts,
                    RefinementType::Trace,
                    ExplorationStrategy::BFS,
                    false,
                    &mut timing
                ),
                "Strong bisimulation implies trace refinement."
            );
        });
    }
}
