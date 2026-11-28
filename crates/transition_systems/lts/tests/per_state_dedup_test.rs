use std::collections::BTreeSet;

use rand::RngExt;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsBuilder;
use merc_lts::LtsBuilderMem;
use merc_lts::PerStateDedup;
use merc_lts::StateIndex;
use merc_utilities::random_test;

/// Collects `(from, label, to)` for every transition in `lts`, keeping duplicates.
fn all_transitions(lts: &LabelledTransitionSystem<String>) -> Vec<(usize, String, usize)> {
    lts.iter_states()
        .flat_map(|state| {
            lts.outgoing_transitions(state)
                .map(move |t| (state.value(), lts.labels()[t.label.value()].clone(), t.to.value()))
        })
        .collect()
}

#[test]
fn test_per_state_dedup_removes_within_state_duplicates() {
    let labels = vec!["a".to_string(), "b".to_string()];
    let mut inner = LtsBuilderMem::new(labels.clone(), Vec::new());
    let mut dedup = PerStateDedup::new();

    // State 0 has a duplicate (0, b, 1) and a distinct (0, b, 2); state 1 has the same
    // (label, to) pair as state 0, which must be kept since its source differs.
    for (from, to) in [(0, 1), (0, 1), (0, 2), (1, 1)] {
        dedup
            .add(
                StateIndex::new(from),
                &labels[1],
                StateIndex::new(to),
                |from, label, to| inner.add_transition(from, label, to),
            )
            .unwrap();
    }
    dedup
        .flush(|from, label, to| inner.add_transition(from, label, to))
        .unwrap();

    let mut result = all_transitions(&inner.finish(StateIndex::new(0), false));
    result.sort();
    assert_eq!(
        result,
        vec![
            (0, "b".to_string(), 1),
            (0, "b".to_string(), 2),
            (1, "b".to_string(), 1)
        ]
    );
}

/// Forwarding through `PerStateDedup` without a final deduplication yields exactly the set of
/// added transitions.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_per_state_dedup() {
    random_test(100, |rng| {
        let labels = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut inner = LtsBuilderMem::new(labels.clone(), Vec::new());
        let mut dedup = PerStateDedup::new();
        let mut expected = BTreeSet::new();

        // Every state's transitions are added consecutively, as `PerStateDedup` requires.
        for from in 0..10 {
            for _ in 0..rng.random_range(0..8) {
                let label = rng.random_range(0..labels.len());
                let to = rng.random_range(0..10);
                dedup
                    .add(
                        StateIndex::new(from),
                        &labels[label],
                        StateIndex::new(to),
                        |from, label, to| inner.add_transition(from, label, to),
                    )
                    .unwrap();
                expected.insert((from, labels[label].clone(), to));
            }
        }
        dedup
            .flush(|from, label, to| inner.add_transition(from, label, to))
            .unwrap();

        let result = all_transitions(&inner.finish(StateIndex::new(0), false));
        assert_eq!(result.len(), expected.len(), "no transition may remain duplicated");
        assert_eq!(result.into_iter().collect::<BTreeSet<_>>(), expected);
    });
}
