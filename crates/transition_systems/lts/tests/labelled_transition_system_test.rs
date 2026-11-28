use std::collections::BTreeSet;

use rand::seq::SliceRandom;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use merc_lts::random_lts;
use merc_utilities::random_test;

/// Returns the outgoing transitions of `state` as `(label, to)` pairs, with `to` shifted by `offset`.
fn outgoing<L: LTS>(lts: &L, state: StateIndex, offset: usize) -> BTreeSet<(L::Label, usize)> {
    lts.outgoing_transitions(state)
        .map(|t| (lts.labels()[t.label.value()].clone(), t.to.value() + offset))
        .collect()
}

/// The merged LTS contains the left LTS unchanged, followed by the right LTS with its states
/// offset by the number of states of the left LTS.
#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_random_merge_disjoint() {
    random_test(100, |rng| {
        let left = random_lts::<String, _>(rng, 100, 5);
        let right = random_lts::<String, _>(rng, 100, 8);

        let (merged, right_initial) = left.clone().merge_disjoint(&right);

        let offset = left.num_of_states();
        assert_eq!(merged.num_of_states(), left.num_of_states() + right.num_of_states());
        assert_eq!(
            merged.num_of_transitions(),
            left.num_of_transitions() + right.num_of_transitions()
        );
        assert_eq!(merged.initial_state_index(), left.initial_state_index());
        assert_eq!(right_initial.value(), right.initial_state_index().value() + offset);

        for state in left.iter_states() {
            assert_eq!(outgoing(&merged, state, 0), outgoing(&left, state, 0));
        }
        for state in right.iter_states() {
            assert_eq!(
                outgoing(&merged, StateIndex::new(state.value() + offset), 0),
                outgoing(&right, state, offset)
            );
        }
    });
}

/// Renumbering the states moves every transition and the initial state along the permutation.
#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_random_new_from_permutation() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 100, 3);

        let mut permutation: Vec<StateIndex> = lts.iter_states().collect();
        permutation.shuffle(rng);

        let permuted = LabelledTransitionSystem::new_from_permutation(lts.clone(), |i| permutation[i]);

        assert_eq!(permuted.num_of_states(), lts.num_of_states());
        assert_eq!(permuted.num_of_transitions(), lts.num_of_transitions());
        assert_eq!(permuted.initial_state_index(), permutation[lts.initial_state_index()]);
        for state in lts.iter_states() {
            let expected: BTreeSet<_> = lts
                .outgoing_transitions(state)
                .map(|t| (lts.labels()[t.label.value()].clone(), permutation[t.to].value()))
                .collect();
            assert_eq!(outgoing(&permuted, permutation[state], 0), expected);
        }
    });
}
