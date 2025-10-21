use mcrl3_utilities::ByteCompressedVec;
use mcrl3_utilities::CompressedEntry;
use mcrl3_utilities::bytevec;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;
use crate::Transition;

/// Stores the incoming transitions for a given labelled transition system.
pub struct IncomingTransitions {
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_from: ByteCompressedVec<StateIndex>,
    state2incoming: ByteCompressedVec<TransitionIndex>,
}

/// Stores the offsets at which the transitions for a state can be found.
///
/// The offsets [start, next_start) contain all incoming transitions, and [start, silent_end) contain only the silent transitions.
#[derive(Default, Clone, Debug)]
struct TransitionIndex {
    start: usize,
}

impl TransitionIndex {
    fn new(start: usize) -> TransitionIndex {
        TransitionIndex { start }
    }
}

impl IncomingTransitions {
    pub fn new(lts: &LabelledTransitionSystem) -> IncomingTransitions {
        let num_states = lts.num_of_states();
        let mut transition_labels = bytevec![LabelIndex::new(0); lts.num_of_transitions()];
        let mut transition_from = bytevec![StateIndex::new(0); lts.num_of_transitions()];
        let mut state2incoming = bytevec![TransitionIndex::default(); num_states];

        // Count the number of incoming transitions for each state
        for state_index in lts.iter_states() {
            for transition in lts.outgoing_transitions(state_index) {
                state2incoming.update(transition.to.value(), |incoming| incoming.start += 1);
            }
        }

        // Compute the start offsets (prefix sum)
        state2incoming.fold(0, |offset, incoming| {
            let new_offset = offset + incoming.start;
            *incoming = TransitionIndex::new(offset);
            new_offset
        });

        // Place the transitions
        for state_index in lts.iter_states() {
            for transition in lts.outgoing_transitions(state_index) {
                state2incoming.update(transition.to.value(), |incoming| {
                    transition_labels.set(incoming.start, transition.label);
                    transition_from.set(incoming.start, state_index);
                    incoming.start += 1;
                });
            }
        }

        state2incoming.fold(0, |previous, state| {
            let result = state.start;
            state.start = previous;
            result
        });

        // Add sentinel state
        state2incoming.push(TransitionIndex::new(transition_labels.len()));

        // Sort the incoming transitions such that silent transitions come first.
        for state_index in 0..num_states {
            let state = state2incoming.index(state_index);
            let next_state = state2incoming.index(state_index + 1);

            // Get the ranges to sort
            let start = state.start;
            let end = next_state.start;

            // Extract, sort, and put back
            let mut pairs: Vec<_> = (start..end)
                .map(|i| (transition_labels.index(i), transition_from.index(i)))
                .collect();
            pairs.sort_unstable_by_key(|(label, _)| *label);

            for (i, (label, from)) in pairs.into_iter().enumerate() {
                transition_labels.set(start + i, label);
                transition_from.set(start + i, from);
            }
        }

        IncomingTransitions {
            transition_labels,
            transition_from,
            state2incoming,
        }
    }

    /// Returns an iterator over the incoming transitions for the given state.
    pub fn incoming_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let state = self.state2incoming.index(state_index.value());
        let next_state = self.state2incoming.index(state_index.value() + 1);
        (state.start..next_state.start)
            .map(move |i| Transition::new(self.transition_labels.index(i), self.transition_from.index(i)))
    }

    // Return an iterator over the incoming silent transitions for the given state.
    pub fn incoming_silent_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let state = self.state2incoming.index(state_index.value());
        let next_state = self.state2incoming.index(state_index.value() + 1);
        (state.start..next_state.start)
            .map(move |i| Transition::new(self.transition_labels.index(i), self.transition_from.index(i)))
            .take_while(|transition| transition.label == 0)
    }
}

impl CompressedEntry for TransitionIndex {
    fn to_bytes(&self, bytes: &mut [u8]) {
        self.start.to_bytes(bytes);
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            start: usize::from_bytes(bytes),
        }
    }

    fn bytes_required(&self) -> usize {
        self.start.bytes_required()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use log::trace;
    use mcrl3_utilities::random_test;

    use crate::random_lts;

    #[test]
    fn test_random_incoming_transitions() {
        random_test(100, |rng| {
            let lts = random_lts(rng, 10, 3, 3);
            trace!("{:?}", lts);
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
