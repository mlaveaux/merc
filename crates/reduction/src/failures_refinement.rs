//! Authors: Jan Friso Groote, Maurice Laveaux, Wieger Wesselink and Tim A.C. Willemse
//! This file contains an implementation of
//! M. Laveaux, J.F. Groote and T.A.C. Willemse
//! Correct and Efficient Antichain Algorithms for Refinement Checking. Logical Methods in Computer Science 17(1) 2021
//!
//! There are six algorithms. One for trace inclusion, one for failures inclusion and one for failures-divergence
//! inclusion. All algorithms come in a variant with and without internal steps. It is possible to generate a counter
//! transition system in case the inclusion is answered by no.

use merc_lts::LTS;
use merc_utilities::Timing;

use crate::{Equivalence, VecSet, reduce_lts};

/// Sets the exploration strategy for the failures refinement algorithm.
pub enum ExplorationStrategy {
    BFS,
    DFS,
}

/// Specifies the type of refinement to be checked.
pub enum RefinementType {
    FailuresDivergence,
}

/// This function checks using algorithms in the paper mentioned above
/// whether transition system l1 is included in transition system l2, in the
/// sense of trace inclusions, failures inclusion and divergence failures
/// inclusion.
pub fn failures_refinement<L: LTS, const COUNTER_EXAMPLE: bool>(
    impl_lts: L,
    spec_lts: L,
    refinement: RefinementType,
    strategy: ExplorationStrategy,
    preprocess: bool,
    timing: &mut Timing,
) -> bool {


    // For the preprocessing/quotienting step it makes sense to merge both LTSs
    // together in case that some states are equivalent. So we do this is all branches.
    let (merged_lts, initial_spec) = if preprocess {
        if COUNTER_EXAMPLE {
            // If a counter example is to be generated, we only reduce the
            // specification LTS such that the trace remains valid.
            // let reduced_spec = reduce_lts(spec_lts, Equivalence::BranchingBisim, timing);
            // impl_lts.merge_disjoint(&reduced_spec)
            unimplemented!("Adjust initial_spec after reduction");
        } else {
            let (merged_lts, initial_spec) = impl_lts.merge_disjoint(&spec_lts);

            // Reduce all states in the merged LTS.
            // TODO: How to deal with the initial state of the spec LTS?
            let reduced_lts = reduce_lts(merged_lts, Equivalence::BranchingBisim, timing);
            unimplemented!("Adjust initial_spec after reduction");
            // (reduced_lts, initial_spec)
        }
    } else {
        impl_lts.merge_disjoint(&spec_lts)
    };

    let mut working = vec![(merged_lts.initial_state_index(), vec![initial_spec])];

    while let Some((impl_state, spec)) = working.pop() {
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

            if spec_prime.is_empty() { // if spec' = {} then
                return false;  //    return false;
            }
        }
    }

    false
}

/// Stores cached information about the LTSs to speed up refinement checks.
struct LtsCache {}
