use merc_collections::ByteCompressedVec;
use merc_collections::bytevec;

use crate::LTS;
use crate::LabelIndex;
use crate::StateIndex;
use crate::Transition;

/// Stores the incoming transitions for a given labelled transition system.
pub struct IncomingTransitions {
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_from: ByteCompressedVec<StateIndex>,
    state2incoming: ByteCompressedVec<usize>,
}

impl IncomingTransitions {
    pub fn new(lts: &impl LTS) -> Self {
        let mut transition_labels = bytevec![LabelIndex::new(0); lts.num_of_transitions()];
        let mut transition_from = bytevec![StateIndex::new(0); lts.num_of_transitions()];
        let mut state2incoming = bytevec![0usize; lts.num_of_states()];

        // Count the number of incoming transitions for each state
        for state_index in lts.iter_states() {
            for transition in lts.outgoing_transitions(state_index) {
                state2incoming.update(transition.to.value(), |start| *start += 1);
            }
        }

        // Compute the start offsets (prefix sum)
        state2incoming.fold(0, |offset, start| {
            let new_offset = offset + *start;
            *start = offset;
            new_offset
        });

        // Place the transitions
        for state_index in lts.iter_states() {
            for transition in lts.outgoing_transitions(state_index) {
                state2incoming.update(transition.to.value(), |start| {
                    transition_labels.set(*start, transition.label);
                    transition_from.set(*start, state_index);
                    *start += 1;
                });
            }
        }

        state2incoming.fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        // Add sentinel state
        state2incoming.push(transition_labels.len());

        // Sort the incoming transitions such that silent transitions come first.
        //
        // TODO: This could be more efficient by simply grouping them instead of sorting, perhaps some group using a predicate.
        let mut pairs = Vec::new();
        for state_index in 0..lts.num_of_states() {
            let start = state2incoming.index(state_index);
            let end = state2incoming.index(state_index + 1);

            // Extract, sort, and put back
            pairs.clear();
            pairs.extend((start..end).map(|i| (transition_labels.index(i), transition_from.index(i))));
            pairs.sort_unstable_by_key(|(label, _)| *label);

            for (i, (label, from)) in pairs.iter().enumerate() {
                transition_labels.set(start + i, *label);
                transition_from.set(start + i, *from);
            }
        }

        Self {
            transition_labels,
            transition_from,
            state2incoming,
        }
    }

    /// Returns an iterator over the incoming transitions for the given state.
    pub fn incoming_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let start = self.state2incoming.index(state_index.value());
        let end = self.state2incoming.index(state_index.value() + 1);
        (start..end).map(move |i| Transition::new(self.transition_labels.index(i), self.transition_from.index(i)))
    }

    // Return an iterator over the incoming silent transitions for the given state.
    pub fn incoming_silent_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let start = self.state2incoming.index(state_index.value());
        let end = self.state2incoming.index(state_index.value() + 1);
        (start..end)
            .map(move |i| Transition::new(self.transition_labels.index(i), self.transition_from.index(i)))
            .take_while(|transition| transition.label == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use merc_io::DumpFiles;
    use merc_utilities::random_test;

    use crate::random_lts;

    #[test]
    fn test_random_incoming_transitions() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_incoming_transitions");

            let lts = random_lts(rng, 10, 3, 3);
            files.dump("input.aut", |f| crate::write_aut(f, &lts)).unwrap();
            let incoming = IncomingTransitions::new(&lts);

            // Check that for every outgoing transition there is an incoming transition.
            for state_index in lts.iter_states() {
                for transition in lts.outgoing_transitions(state_index) {
                    let found = incoming
                        .incoming_transitions(transition.to)
                        .any(|incoming| incoming.label == transition.label && incoming.to == state_index);
                    assert!(
                        found,
                        "Outgoing transition ({state_index}, {transition:?}) should have an incoming transition"
                    );
                }
            }

            // Check that all incoming transitions belong to some outgoing transition.
            for state_index in lts.iter_states() {
                for transition in incoming.incoming_transitions(state_index) {
                    let found = lts
                        .outgoing_transitions(transition.to)
                        .any(|outgoing| outgoing.label == transition.label && outgoing.to == state_index);
                    assert!(
                        found,
                        "Incoming transition ({transition:?}, {state_index}) should have an outgoing transition"
                    );
                }
            }
        });
    }
}
