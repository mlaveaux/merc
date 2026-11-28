use std::collections::BTreeSet;
use std::io::Cursor;

use merc_data::Mcrl2DataSpecification;
use merc_lts::LTS;
use merc_lts::LtsAction;
use merc_lts::LtsBuilder;
use merc_lts::LtsMultiAction;
use merc_lts::LtsStream;
use merc_lts::StateIndex;
use merc_lts::random_lts;
use merc_lts::read_lts;
use merc_lts::write_lts;
use merc_utilities::random_test;

/// Returns the transitions of `lts` as `(from, label, to)` triples, so that two LTSs can be
/// compared independently of how they number their labels.
fn transitions<L: LTS>(lts: &L) -> BTreeSet<(usize, L::Label, usize)> {
    lts.iter_states()
        .flat_map(|state| {
            lts.outgoing_transitions(state)
                .map(move |t| (state.value(), lts.labels()[t.label.value()].clone(), t.to.value()))
        })
        .collect()
}

/// Asserts that both LTSs have the same initial state and transitions.
///
/// Without state labels the `.lts` format does not store the number of states, so trailing
/// states that no transition mentions may be missing from `actual`.
fn assert_same_lts<L: LTS, R: LTS<Label = L::Label>>(expected: &L, actual: &R) {
    assert_eq!(expected.initial_state_index(), actual.initial_state_index());
    assert!(actual.num_of_states() <= expected.num_of_states());
    let mentioned = transitions(expected)
        .into_iter()
        .map(|(from, _, to)| from.max(to))
        .chain([expected.initial_state_index().value()])
        .max()
        .unwrap();
    assert!(mentioned < actual.num_of_states(), "a mentioned state is missing");
    assert_eq!(expected.num_of_transitions(), actual.num_of_transitions());
    assert_eq!(transitions(expected), transitions(actual));
}

#[test]
#[cfg_attr(miri, ignore)] // Tests are too slow under miri.
fn test_read_lts() {
    let (lts, _data_spec) = read_lts(include_bytes!("../../../../examples/lts/abp.lts").as_ref(), true).unwrap();

    assert_eq!(lts.num_of_states(), 74);
    assert_eq!(lts.num_of_transitions(), 92);
    assert_eq!(*lts.initial_state_index(), 0);
    assert_eq!(lts.state_labels().count(), 74);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_lts_state_labels_io() {
    let (lts, data_spec) = read_lts(include_bytes!("../../../../examples/lts/abp.lts").as_ref(), true).unwrap();

    let mut buffer: Vec<u8> = Vec::new();
    write_lts(&mut buffer, &lts, &data_spec).unwrap();
    let (result_lts, _data_spec) = read_lts(&buffer[..], true).unwrap();

    assert_same_lts(&lts, &result_lts);
    assert!(lts.state_labels().eq(result_lts.state_labels()));
    assert_eq!(lts.state_labels().count(), lts.num_of_states());

    // Dropping the state labels on reading also works.
    let (result_lts, _data_spec) = read_lts(&buffer[..], false).unwrap();
    assert_eq!(result_lts.state_labels().count(), 0);
    assert_same_lts(&lts, &result_lts);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_random_lts_io() {
    random_test(100, |rng| {
        let lts = random_lts::<LtsMultiAction<LtsAction>, _>(rng, 1000, 3);

        let mut buffer: Vec<u8> = Vec::new();
        write_lts(&mut buffer, &lts, &Mcrl2DataSpecification::default()).unwrap();

        let (result_lts, _data_spec) = read_lts(&buffer[0..], false).unwrap();
        assert_same_lts(&lts, &result_lts);
    })
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under Miri
fn test_random_lts_stream_io() {
    random_test(100, |rng| {
        let lts = random_lts::<LtsMultiAction<LtsAction>, _>(rng, 1000, 3);
        let data_spec = Mcrl2DataSpecification::default();

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut stream = LtsStream::new(&mut buffer, &data_spec).unwrap();
            for state_index in lts.iter_states() {
                for transition in lts.outgoing_transitions(state_index) {
                    stream
                        .add_transition(state_index, &lts.labels()[transition.label.value()], transition.to)
                        .unwrap();
                }
            }

            stream.require_num_of_states(lts.num_of_states());
            stream.finish(lts.initial_state_index()).unwrap();
        }

        buffer.set_position(0);
        let (result_lts, _data_spec) = read_lts(&mut buffer, false).unwrap();
        assert_same_lts(&lts, &result_lts);
    })
}

/// An LTS with no transitions (a single deadlock state) must round-trip: the reader must
/// accept a stream with only the header, the initial state and the end-of-stream marker.
#[test]
fn test_lts_stream_no_transitions() {
    let data_spec = Mcrl2DataSpecification::default();
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut stream: LtsStream<_> = LtsStream::new(&mut buffer, &data_spec).unwrap();
        stream.require_num_of_states(1);
        stream.finish(StateIndex::new(0)).unwrap();
    }

    buffer.set_position(0);
    let (result_lts, _data_spec) = read_lts(&mut buffer, false).unwrap();

    assert_eq!(result_lts.num_of_states(), 1);
    assert_eq!(result_lts.num_of_transitions(), 0);
}
