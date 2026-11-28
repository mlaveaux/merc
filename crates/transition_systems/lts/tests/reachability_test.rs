use std::collections::BTreeMap;

use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_lts::random_lts;
use merc_lts::reachability;
use merc_lts::reachable_lts;
use merc_utilities::random_test;

/// Returns, for every state, whether it is reachable from `initial` using only transitions
/// whose label satisfies `filter`, computed by a naive fixpoint.
fn naive_reachable<L: LTS>(lts: &L, initial: StateIndex, filter: impl Fn(&L::Label) -> bool) -> Vec<bool> {
    let mut reachable = vec![false; lts.num_of_states()];
    reachable[initial] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for state in lts.iter_states() {
            if !reachable[state] {
                continue;
            }
            for t in lts.outgoing_transitions(state) {
                if filter(&lts.labels()[t.label.value()]) && !reachable[t.to] {
                    reachable[t.to] = true;
                    changed = true;
                }
            }
        }
    }
    reachable
}

/// Counts how often every label occurs on the outgoing transitions of the states in `include`.
fn label_counts<L: LTS>(lts: &L, include: impl Fn(StateIndex) -> bool) -> BTreeMap<L::Label, usize> {
    let mut counts = BTreeMap::new();
    for state in lts.iter_states().filter(|s| include(*s)) {
        for t in lts.outgoing_transitions(state) {
            *counts.entry(lts.labels()[t.label.value()].clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// `reachability` visits exactly the states reachable via accepted transitions, each once.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_reachability() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 50, 3);
        let initial = lts.initial_state_index();

        let expected = naive_reachable(&lts, initial, |label| label != "a");

        let mut visited = vec![0; lts.num_of_states()];
        reachability(
            &lts,
            initial,
            |label| lts.labels()[label.value()] != "a",
            |state| visited[state] += 1,
        );

        for state in lts.iter_states() {
            assert_eq!(visited[state], usize::from(expected[state]), "state {state}");
        }
    });
}

/// `reachable_lts` keeps exactly the reachable states and their transitions, with the
/// initial state first and every remaining state reachable.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_reachable_lts() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 50, 3);
        let reachable = naive_reachable(&lts, lts.initial_state_index(), |_| true);

        let result = reachable_lts(&lts);

        let expected_labels = label_counts(&lts, |state| reachable[state]);
        let actual_labels = label_counts(&result, |_| true);

        assert_eq!(result.num_of_states(), reachable.iter().filter(|r| **r).count());
        assert_eq!(result.initial_state_index(), StateIndex::new(0));
        assert_eq!(actual_labels, expected_labels);
        assert!(
            naive_reachable(&result, result.initial_state_index(), |_| true)
                .iter()
                .all(|r| *r),
            "every state of the result must be reachable"
        );
    });
}
