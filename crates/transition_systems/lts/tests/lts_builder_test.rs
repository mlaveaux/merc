use std::collections::BTreeSet;

use rand::RngExt;

use merc_lts::LTS;
use merc_lts::LtsBuilder;
use merc_lts::LtsBuilderMem;
use merc_lts::StateIndex;
use merc_utilities::random_test;

/// Finishing with deduplication yields exactly the set of added transitions.
#[test]
fn test_random_builder_remove_duplicates() {
    random_test(100, |rng| {
        let labels = vec!["tau".to_string(), "a".to_string(), "b".to_string()];
        let mut builder = LtsBuilderMem::new(labels.clone(), Vec::new());
        let mut expected = BTreeSet::new();

        for _ in 0..rng.random_range(0..30) {
            let from = rng.random_range(0..10);
            let label = rng.random_range(0..labels.len());
            let to = rng.random_range(0..10);
            builder
                .add_transition(StateIndex::new(from), &labels[label], StateIndex::new(to))
                .unwrap();
            expected.insert((from, labels[label].clone(), to));
        }
        builder.require_num_of_states(10);

        let lts = &builder.finish(StateIndex::new(0), true);

        let actual: Vec<_> = lts
            .iter_states()
            .flat_map(|state| {
                lts.outgoing_transitions(state)
                    .map(move |t| (state.value(), lts.labels()[t.label.value()].clone(), t.to.value()))
            })
            .collect();
        assert_eq!(actual.len(), expected.len(), "no transition may remain duplicated");
        assert_eq!(actual.into_iter().collect::<BTreeSet<_>>(), expected);
        assert_eq!(lts.num_of_states(), 10);
    });
}
