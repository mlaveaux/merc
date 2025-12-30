#![forbid(unsafe_code)]

use log::trace;

use merc_collections::IndexedSet;

use crate::LTS;
use crate::LabelledTransitionSystem;
use crate::LtsBuilderFast;
use crate::StateIndex;
use crate::TransitionLabel;

/// Computes the synchronous product LTS of two given LTSs.
///
///  If `synchronized_labels` is `None`, then all common labels (except tau) are
/// considered synchronized. Otherwise, the provided labels are used for
/// synchronization.
pub fn product_lts<L: LTS, R: LTS<Label = L::Label>>(
    left: &L,
    right: &R,
    synchronized_labels: Option<Vec<L::Label>>,
) -> LabelledTransitionSystem<L::Label> {
    // Determine the combination of action labels
    let mut all_labels: IndexedSet<L::Label> = IndexedSet::new();

    for label in left.labels() {
        all_labels.insert(label.clone());
    }

    // Determine the synchronised labels
    let synchronised_labels = match synchronized_labels {
        Some(x) => x,
        None => {
            let mut new_synchronized_labels: Vec<L::Label> = Vec::new();
            for label in right.labels() {
                let (_index, inserted) = all_labels.insert(label.clone());

                if !inserted {
                    new_synchronized_labels.push(label.clone());
                }
            }

            // Tau can never be synchronised.
            new_synchronized_labels.retain(|l| !l.is_tau_label());
            new_synchronized_labels
        }
    };

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

                        lts_builder.add_transition(
                            StateIndex::new(*product_index),
                            &left.labels()[*left_transition.label],
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
                lts_builder.add_transition(
                    StateIndex::new(*product_index),
                    &left.labels()[*left_transition.label],
                    StateIndex::new(*left_index),
                );

                if inserted {
                    trace!("Adding ({}, {})", left_transition.to, right_state);
                    working.push((left_transition.to, right_state));
                }
            }
        }

        for right_transition in right.outgoing_transitions(right_state) {
            if synchronised_labels.contains(&right.labels()[*right_transition.label]) {
                // Already handled in the left transitions loop.
                continue;
            }

            // (left, right) -[a]-> (left, right') iff right -[a]-> right' and a is not a synchronous action.
            let (right_index, inserted) = discovered_states.insert((left_state, right_transition.to));
            lts_builder.add_transition(
                StateIndex::new(*product_index),
                &right.labels()[*right_transition.label],
                StateIndex::new(*right_index),
            );

            if inserted {
                // New state discovered.
                trace!("Adding ({}, {})", left_state, right_transition.to);
                working.push((left_state, right_transition.to));
            }
        }
    }

    if lts_builder.num_of_states() == 0 {
        // The product has no states, but an LTS requires at least one state (the initial state).
        lts_builder.require_num_of_states(1);
    }

    lts_builder.finish(StateIndex::new(0), true)
}

#[cfg(test)]
mod tests {
    use crate::random_lts;
    use crate::write_aut;

    use super::*;

    use merc_io::DumpFiles;
    use test_log::test;

    use merc_utilities::random_test;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_lts_product() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_lts_product");

            // This test only checks the assertions of an LTS internally.
            let left = random_lts(rng, 10, 3, 3);
            let right = random_lts(rng, 10, 3, 3);

            files.dump("left.aut", |f| write_aut(f, &left)).unwrap();
            files.dump("right.aut", |f| write_aut(f, &right)).unwrap();
            let product = product_lts(&left, &right, None);

            files.dump("product.aut", |f| write_aut(f, &product)).unwrap();
        });
    }
}
