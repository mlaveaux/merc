#![forbid(unsafe_code)]

use std::marker::PhantomData;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;

use crate::LTS;
use crate::LabelIndex;
use crate::StateIndex;

/// Stores the incoming transitions for a given labelled transition system.
pub struct IncomingTransitions<'a> {
    /// A flat list of all incoming transition labels in the LTS. They are stored in two separate
    /// arrays since the compression is based on the highest value.
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_from: ByteCompressedVec<StateIndex>,

    /// A mapping from the state to the `transition_labels` and
    /// `transition_from` that stores its incoming transitions.
    state2incoming: ByteCompressedVec<usize>,

    /// Marker to tie the lifetime of the incoming transitions to the LTS.
    _marker: PhantomData<&'a ()>,
}

impl<'a> IncomingTransitions<'a> {
    pub fn new<L: LTS>(lts: &'a L) -> Self {
        // Sized for their final byte width up front.
        let mut transition_labels =
            ByteCompressedVec::with_capacity(lts.num_of_transitions(), lts.num_of_labels().bytes_required());
        transition_labels.resize_zeroed(lts.num_of_transitions(), lts.num_of_labels().bytes_required());
        let mut transition_from =
            ByteCompressedVec::with_capacity(lts.num_of_transitions(), lts.num_of_states().bytes_required());
        transition_from.resize_zeroed(lts.num_of_transitions(), lts.num_of_states().bytes_required());
        let mut state2incoming =
            ByteCompressedVec::with_capacity(lts.num_of_states(), lts.num_of_transitions().bytes_required());
        state2incoming.resize_zeroed(lts.num_of_states(), lts.num_of_transitions().bytes_required());

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
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over the incoming transitions for the given state.
    ///
    /// # Panics
    ///
    /// Panics if `state_index` is not less than the number of states in the underlying LTS.
    pub fn incoming_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = FromTransition> + '_ {
        let start = self.state2incoming.index(state_index.value());
        let end = self.state2incoming.index(state_index.value() + 1);
        (start..end).map(move |i| FromTransition::new(self.transition_labels.index(i), self.transition_from.index(i)))
    }

    /// Returns an iterator over the incoming silent (tau-labelled) transitions for the given state.
    ///
    /// # Panics
    ///
    /// Panics if `state_index` is not less than the number of states in the underlying LTS.
    pub fn incoming_silent_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = FromTransition> + '_ {
        let start = self.state2incoming.index(state_index.value());
        let end = self.state2incoming.index(state_index.value() + 1);
        (start..end)
            .map(move |i| FromTransition::new(self.transition_labels.index(i), self.transition_from.index(i)))
            .take_while(|transition| transition.label == 0)
    }
}

/// Represents an incoming transition in the LTS going to a known state.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FromTransition {
    pub label: LabelIndex,
    pub from: StateIndex,
}

impl FromTransition {
    /// Constructs a new transition.
    pub fn new(label: LabelIndex, from: StateIndex) -> Self {
        Self { label, from }
    }
}
