use std::collections::HashMap;
use std::fmt;

use itertools::Itertools;
use merc_utilities::ByteCompressedVec;
use merc_utilities::CompressedEntry;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;

/// This struct helps in building a labelled transition system by accumulating
/// transitions efficiently.
///
/// # Details
///
/// When labels are added via `add_transition`, they are mapped to `LabelIndex`
/// values internally. The mapping is maintained in a `HashMap<String,
/// LabelIndex>`, and new labels are assigned the next available index.
/// Alternatively, labels can be added directly using `add_transition_index` an
///
pub struct LtsBuilder {
    transition_from: ByteCompressedVec<StateIndex>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    // This is used to keep track of the label to index mapping.
    labels_index: HashMap<String, LabelIndex>,
    labels: Vec<String>,

    /// The number of states (derived from the transitions).
    num_of_states: usize,
}

impl LtsBuilder {
    /// Initializes a new empty builder.
    pub fn new(labels: Vec<String>, hidden_labels: Vec<String>) -> Self {
        Self::with_capacity(labels, hidden_labels, 0, 0, 0)
    }

    /// Initializes the builder with pre-allocated capacity for states and transitions.
    pub fn with_capacity(
        mut labels: Vec<String>,
        hidden_labels: Vec<String>,
        num_of_states: usize,
        num_of_labels: usize,
        num_of_transitions: usize,
    ) -> Self {
        // Introduce the fixed 0 indexed tau label.
        if let Some(tau_pos) = labels.iter().position(|l| l == "tau") {
            labels.swap(0, tau_pos);
        } else {
            labels.insert(0, "tau".to_string());
        }

        // Remove duplicates from the labels.
        labels.sort();
        labels.dedup();

        // Ensure that all hidden labels are mapped to the tau action.
        let mut labels_index = HashMap::new();
        labels_index.insert("tau".to_string(), LabelIndex::new(0));
        for label in hidden_labels.iter() {
            labels_index.insert(label.clone(), LabelIndex::new(0)); // Map hidden labels to tau
        }

        Self {
            transition_from: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
            transition_labels: ByteCompressedVec::with_capacity(num_of_transitions, num_of_labels.bytes_required()),
            transition_to: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
            labels_index,
            labels,
            num_of_states: 0,
        }
    }

    /// Adds a transition to the builder.
    pub fn add_transition(&mut self, from: StateIndex, label: &str, to: StateIndex) {
        let label_index = if let Some(&index) = self.labels_index.get(label) {
            index
        } else {
            let index = LabelIndex::new(self.labels.len());
            self.labels_index.insert(label.to_string(), index);
            self.labels.push(label.to_string());
            index
        };

        self.transition_from.push(from);
        self.transition_labels.push(label_index);
        self.transition_to.push(to);

        // Update the number of states.
        self.num_of_states = self.num_of_states.max(from.value() + 1).max(to.value() + 1);
    }

    /// Adds a transition to the builder.
    pub fn add_transition_index(&mut self, from: StateIndex, label: LabelIndex, to: StateIndex) {
        debug_assert!(
            (label.value() < self.labels.len()),
            "Label index {:?} out of bounds (num labels: {})",
            label,
            self.labels.len()
        );

        self.transition_from.push(from);
        self.transition_labels.push(label);
        self.transition_to.push(to);

        // Update the number of states.
        self.num_of_states = self.num_of_states.max(from.value() + 1).max(to.value() + 1);
    }

    /// Finalizes the builder and returns the constructed labelled transition system.
    pub fn finish(&mut self, initial_state: StateIndex, remove_duplicates: bool) -> LabelledTransitionSystem {
        if remove_duplicates {
            self.remove_duplicates();
        }

        LabelledTransitionSystem::new(
            initial_state,
            Some(self.num_of_states),
            || self.iter(),
            self.labels.clone(),
        )
    }

    /// Returns the number of transitions added to the builder.
    pub fn num_of_transitions(&self) -> usize {
        self.transition_from.len()
    }

    /// Removes duplicated transitions from the added transitions.
    fn remove_duplicates(&mut self) {
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
            let mut builder = LtsBuilder::new(vec!["a".to_string(), "b".to_string(), "c".to_string()], Vec::new());

            for _ in 0..rng.random_range(0..10) {
                let from = StateIndex::new(rng.random_range(0..10));
                let label = LabelIndex::new(rng.random_range(0..2));
                let to = StateIndex::new(rng.random_range(0..10));
                builder.add_transition_index(from, label, to);
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
