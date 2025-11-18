use std::fmt;

use itertools::Itertools;
use merc_utilities::ByteCompressedVec;
use merc_utilities::CompressedEntry;

use crate::LabelIndex;
use crate::StateIndex;

/// This struct helps in building a labelled transition system by accumulating transitions efficiently.
pub struct LtsBuilder {
    transition_from: ByteCompressedVec<StateIndex>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,
}

impl LtsBuilder {
    pub fn new() -> Self {
        Self {
            transition_from: ByteCompressedVec::new(),
            transition_labels: ByteCompressedVec::new(),
            transition_to: ByteCompressedVec::new(),
        }
    }

    /// Initializes the builder with pre-allocated capacity for states and transitions.
    pub fn with_capacity(num_of_states: usize, num_of_labels: usize, num_of_transitions: usize) -> Self {
        Self {
            transition_from: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
            transition_labels: ByteCompressedVec::with_capacity(num_of_transitions, num_of_labels.bytes_required()),
            transition_to: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
        }
    }

    /// Adds a transition to the builder.
    pub fn add_transition(&mut self, from: StateIndex, label: LabelIndex, to: StateIndex) {
        self.transition_from.push(from);
        self.transition_labels.push(label);
        self.transition_to.push(to);
    }

    /// Removes duplicated transitions from the added transitions.
    pub fn remove_duplicates(&mut self) {
        debug_assert!(
            self.transition_from.len() == self.transition_labels.len()
                && self.transition_from.len() == self.transition_to.len(),
            "All transition arrays must have the same length"
        );

        // Sort the three arrays based on (from, label, to)
        let mut indices: Vec<usize> = (0..self.transition_from.len()).collect();
        indices.sort_unstable_by_key(|&i| {
            (
                self.transition_from.index(i),
                self.transition_labels.index(i),
                self.transition_to.index(i),
            )
        });

        // Put the arrays in the sorted order
        self.transition_from.permute_indices(|i: usize| indices[i]);
        self.transition_labels.permute_indices(|i: usize| indices[i]);
        self.transition_to.permute_indices(|i: usize| indices[i]);
    }

    /// Returns an iterator over all transitions as (from, label, to) tuples.
    pub fn iter(&self) -> impl Iterator<Item = (StateIndex, LabelIndex, StateIndex)> {
        self.transition_from
            .iter()
            .zip(self.transition_labels.iter())
            .zip(self.transition_to.iter())
            .map(|((from, label), to)| (from, label, to))
            .dedup()
    }

    /// Returns the number of transitions added to the builder.
    pub fn num_of_transitions(&self) -> usize {
        self.transition_from.len()
    }
}

impl Default for LtsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LtsBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Transitions:")?;
        for (from, label, to) in self.iter() {
            writeln!(f, "    {:?} --[{:?}]-> {:?}", from, label, to)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::Rng;

    use merc_utilities::random_test;

    #[test]
    fn test_random_remove_duplicates() {
        random_test(100, |rng| {
            let mut builder = LtsBuilder::new();

            for _ in 0..rng.random_range(0..10) {
                let from = StateIndex::new(rng.random_range(0..10));
                let label = LabelIndex::new(rng.random_range(0..2));
                let to = StateIndex::new(rng.random_range(0..10));
                builder.add_transition(from, label, to);
            }

            builder.remove_duplicates();

            let transitions = builder.iter().collect::<Vec<_>>();
            debug_assert!(
                transitions.iter().all_unique(),
                "Transitions should be unique after removing duplicates"
            );
        });
    }
}
