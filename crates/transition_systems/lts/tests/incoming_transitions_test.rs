use merc_lts::IncomingTransitions;
use merc_lts::LTS;
use merc_lts::random_lts;
use merc_utilities::random_test;

/// The incoming transitions are exactly the outgoing transitions, read backwards, without
/// duplicates or omissions.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_incoming_transitions() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 1000, 3);
        let incoming = IncomingTransitions::new(&lts);

        let mut outgoing_set = Vec::new();
        let mut incoming_set = Vec::new();
        for state in lts.iter_states() {
            for transition in lts.outgoing_transitions(state) {
                outgoing_set.push((state, transition.label, transition.to));
            }
            for transition in incoming.incoming_transitions(state) {
                incoming_set.push((transition.from, transition.label, state));
            }
        }

        outgoing_set.sort();
        incoming_set.sort();
        assert_eq!(outgoing_set, incoming_set);
    });
}
