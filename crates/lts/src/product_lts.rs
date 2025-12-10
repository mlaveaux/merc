use log::trace;
use merc_utilities::IndexedSet;

use crate::{LTS, LabelIndex, LabelledTransitionSystem, LtsBuilderFast, StateIndex};


/// Computes the synchronous product LTS of two given LTSs.
///
/// This is useful for generating random LTSs by composing smaller random LTSs,
/// which is often a more realistic structure then fully random LTSs.
pub fn product_lts(left: &impl LTS, right: &impl LTS) -> LabelledTransitionSystem {
    // Determine the combination of action labels
    let mut all_labels: IndexedSet<String> = IndexedSet::new();

    for label in left.labels() {
        all_labels.insert(label.clone());
    }

    // Determine the synchronised labels
    let mut synchronised_labels: Vec<String> = Vec::new();
    for label in right.labels() {
        let (_index, inserted) = all_labels.insert(label.clone());

        if !inserted {
            synchronised_labels.push(label.clone());
        }
    }

    // Tau can never be synchronised.
    synchronised_labels.retain(|l| l != "tau");

    // For the product we do not know the number of states and transitions in advance.
    let mut lts_builder = LtsBuilderFast::new(all_labels.to_vec(), Vec::new());

    let mut discovered_states: IndexedSet<(StateIndex, StateIndex)> = IndexedSet::new();
    let mut working = vec![(left.initial_state_index(), right.initial_state_index())];
    let (_, _) = discovered_states.insert((left.initial_state_index(), right.initial_state_index()));

    while let Some((left_state, right_state)) = working.pop() {
        // Find the (left, right) in the set of states.
        let (product_index, inserted) = discovered_states.insert((left_state, right_state));
        debug_assert!(!inserted, "The product state must have already been added");

        trace!("Considering ({left_state}, {right_state})");

        // Add transitions for the left LTS
        for left_transition in left.outgoing_transitions(left_state) {
            if synchronised_labels.contains(&left.labels()[*left_transition.label]) {
                // Find the corresponding right state after this transition
                for right_transition in right.outgoing_transitions(right_state) {
                    if left.labels()[*left_transition.label] == right.labels()[*right_transition.label] {
                        // Labels match so introduce (left, right) -[a]-> (left', right') iff left -[a]-> left' and right -[a]-> right', and a is a synchronous action.
                        let (product_state, inserted) =
                            discovered_states.insert((left_transition.to, right_transition.to));

                        let label_index = LabelIndex::new(
                            *all_labels
                                .index(&left.labels()[*left_transition.label])
                                .expect("Label was already inserted"),
                        );
                        lts_builder.add_transition_index(
                            StateIndex::new(*product_index),
                            label_index,
                            StateIndex::new(*product_state),
                        );

                        if inserted {
                            trace!("Adding ({}, {})", left_transition.to, right_transition.to);
                            working.push((left_transition.to, right_transition.to));
                        }
                    }
                }
            } else {
                let (left_index, inserted) = discovered_states.insert((left_transition.to, right_state));

                // (left, right) -[a]-> (left', right) iff left -[a]-> left' and a is not a synchronous action.
                let label_index = LabelIndex::new(
                    *all_labels
                        .index(&left.labels()[*left_transition.label])
                        .expect("Label was already inserted"),
                );
                lts_builder.add_transition_index(
                    StateIndex::new(*product_index),
                    label_index,
                    StateIndex::new(*left_index),
                );

                if inserted {
                    trace!("Adding ({}, {})", left_transition.to, right_state);
                    working.push((left_transition.to, right_state));
                }
            }
        }

        for right_transition in right.outgoing_transitions(right_state) {
            // (left, right) -[a]-> (left', right) iff left -[a]->right and a is not a synchronous action.
            let (right_index, inserted) = discovered_states.insert((left_state, right_transition.to));

            let label_index = LabelIndex::new(
                *all_labels
                    .index(&right.labels()[*right_transition.label])
                    .expect("Label was already inserted"),
            );
            lts_builder.add_transition_index(
                StateIndex::new(*product_index),
                label_index,
                StateIndex::new(*right_index),
            );

            if inserted {
                // New state discovered.
                trace!("Adding ({}, {})", left_state, right_transition.to);
                working.push((left_state, right_transition.to));
            }
        }
    }

    lts_builder.finish(StateIndex::new(0), true)
}

#[cfg(test)]
mod tests {
    use crate::random_lts;

    use super::*;

    use log::trace;
    use test_log::test;

    use merc_utilities::random_test;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn random_lts_product_test() {
        random_test(100, |rng| {
            // This test only checks the assertions of an LTS internally.
            let left = random_lts(rng, 10, 3, 3);
            let right = random_lts(rng, 10, 3, 3);

            trace!("{left:?}");
            trace!("{right:?}");
            let _product = product_lts(&left, &right);
        });
    }
}
