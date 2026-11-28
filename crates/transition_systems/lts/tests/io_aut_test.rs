use std::collections::BTreeSet;
use std::io::Cursor;

use merc_lts::AutStream;
use merc_lts::LTS;
use merc_lts::LtsBuilder;
use merc_lts::StateIndex;
use merc_lts::random_lts;
use merc_lts::read_aut;
use merc_lts::write_aut;
use merc_utilities::random_test;
use test_log::test;

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

/// Asserts that both LTSs have the same initial state, states and transitions.
fn assert_same_lts<L: LTS, R: LTS<Label = L::Label>>(expected: &L, actual: &R) {
    assert_eq!(expected.initial_state_index(), actual.initial_state_index());
    assert_eq!(expected.num_of_states(), actual.num_of_states());
    assert_eq!(expected.num_of_transitions(), actual.num_of_transitions());
    assert_eq!(transitions(expected), transitions(actual));
}

#[test]
fn test_reading_aut() {
    let file = include_str!("../../../../examples/lts/abp.aut");

    let lts = read_aut(file.as_bytes()).unwrap();

    assert_eq!(lts.initial_state_index().value(), 0);
    assert_eq!(lts.num_of_states(), 74);
    assert_eq!(lts.num_of_transitions(), 92);
    assert_eq!(lts.outgoing_transitions(lts.initial_state_index()).count(), 2);
}

#[test]
fn test_reading_aut_small() {
    let file = "des (1, 3, 3)\n(0,\"a\",1)\n(1,\"tau\",2)\n(1,\"b\",0)\n";

    let lts = read_aut(file.as_bytes()).unwrap();

    assert_eq!(lts.initial_state_index().value(), 1);
    assert_eq!(lts.num_of_states(), 3);
    let expected: BTreeSet<_> = [(0, "a", 1), (1, "tau", 2), (1, "b", 0)]
        .into_iter()
        .map(|(from, label, to)| (from, label.to_string(), to))
        .collect();
    assert_eq!(transitions(&lts), expected);
}

#[test]
fn test_reading_aut_with_duplicate_transitions() {
    // The Aldebaran format is a plain transition relation and may contain duplicate
    // `(from, label, to)` entries, e.g. mCRL2's unreduced `ltscombine` output.
    let file = "des (0, 3, 2)\n(0,\"a\",1)\n(0,\"a\",1)\n(1,\"b\",0)\n";

    let lts = read_aut(file.as_bytes()).unwrap();

    assert_eq!(lts.initial_state_index().value(), 0);
    assert_eq!(
        lts.num_of_transitions(),
        2,
        "the duplicate (0, a, 1) transition must be merged"
    );
}

#[test]
fn test_lts_failure() {
    let wrong_header = "
        des (0,2,
            (0,\"r1(d1)\",1)
            (0,\"r1(d2)\",2)
        ";

    assert!(read_aut(wrong_header.as_bytes()).is_err());

    let wrong_transition = "
        des (0,2,3)
            (0,\"r1(d1),1)
            (0,\"r1(d2)\",2)
        ";

    assert!(read_aut(wrong_transition.as_bytes()).is_err());
}

#[test]
fn test_writing_lts() {
    let file = include_str!("../../../../examples/lts/abp.aut");
    let lts_original = read_aut(file.as_bytes()).unwrap();

    let mut buffer: Vec<u8> = Vec::new();
    write_aut(&mut buffer, &lts_original).unwrap();

    let lts = read_aut(&buffer[0..]).unwrap();
    assert_same_lts(&lts_original, &lts);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under Miri
fn test_random_aut_io() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 1000, 3);

        let mut buffer: Vec<u8> = Vec::new();
        write_aut(&mut buffer, &lts).unwrap();

        let result_lts = read_aut(&buffer[0..]).unwrap();
        assert_same_lts(&lts, &result_lts);
    })
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under Miri
fn test_random_aut_stream_io() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 1000, 3);

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut stream = AutStream::new(&mut buffer).unwrap();
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
        let result_lts = read_aut(&mut buffer).unwrap();
        assert_same_lts(&lts, &result_lts);
    })
}

/// An LTS with no transitions (a single deadlock state) must round-trip: the header
/// placeholder must be fully overwritten so no line of padding is left for the reader.
#[test]
fn test_aut_stream_no_transitions() {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut stream: AutStream<_, String> = AutStream::new(&mut buffer).unwrap();
        stream.require_num_of_states(1);
        stream.finish(StateIndex::new(0)).unwrap();
    }

    buffer.set_position(0);
    let result_lts = read_aut(&mut buffer).unwrap();

    assert_eq!(result_lts.num_of_states(), 1);
    assert_eq!(result_lts.num_of_transitions(), 0);
}
