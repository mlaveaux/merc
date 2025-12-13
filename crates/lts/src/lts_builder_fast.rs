use std::collections::HashMap;
use std::fmt;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;

/// This is the same as [`crate::LtsBuilder`], but optimized for speed rather than memory usage.
/// So it does not use the byte compression for the transitions since somehow permuting and
/// sorting these take a long time (probably due to cache misses).
///
/// Perhaps that implementation can be made more efficient in the future, but for now
/// this works well enough.
pub struct LtsBuilderFast {
    transitions: Vec<(StateIndex, LabelIndex, StateIndex)>,

    // This is used to keep track of the label to index mapping.
    labels_index: HashMap<String, LabelIndex>,
    labels: Vec<String>,

    /// The number of states (derived from the transitions).
    num_of_states: usize,
}

impl LtsBuilderFast {
    /// Initializes a new empty builder.
    pub fn new(labels: Vec<String>, hidden_labels: Vec<String>) -> Self {
        Self::with_capacity(labels, hidden_labels, 0)
    }

    /// Initializes the builder with pre-allocated capacity for states and transitions.
    pub fn with_capacity(mut labels: Vec<String>, hidden_labels: Vec<String>, num_of_transitions: usize) -> Self {
        // Remove duplicates from the labels.
        labels.sort();
        labels.dedup();

        // Introduce the fixed 0 indexed tau label.
        if let Some(tau_pos) = labels.iter().position(|l| l == "tau") {
            labels.swap(0, tau_pos);
        } else {
            labels.insert(0, "tau".to_string());
        }

        // Ensure that all hidden labels are mapped to the tau action.
        let mut labels_index = HashMap::new();
        labels_index.insert("tau".to_string(), LabelIndex::new(0));
        for label in hidden_labels.iter() {
            labels_index.insert(label.clone(), LabelIndex::new(0)); // Map hidden labels to tau
        }

        Self {
            transitions: Vec::with_capacity(num_of_transitions),
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

        self.transitions.push((from, label_index, to));

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

        self.transitions.push((from, label, to));

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
        self.transitions.len()
    }

    /// Returns the number of states that the builder currently found.
    pub fn num_of_states(&self) -> usize {
        self.num_of_states
    }

    /// Sets the number of states to at least the given number. All states without transitions
    /// will simply become deadlock states.
    pub fn require_num_of_states(&mut self, num_states: usize) {
        if num_states > self.num_of_states {
            self.num_of_states = num_states;
        }
    }

    /// Removes duplicated transitions from the added transitions.
    fn remove_duplicates(&mut self) {
        self.transitions.sort();
        self.transitions.dedup();
    }

    /// Returns an iterator over all transitions as (from, label, to) tuples.
    pub fn iter(&self) -> impl Iterator<Item = (StateIndex, LabelIndex, StateIndex)> {
        self.transitions.iter().cloned()
    }
}

impl fmt::Debug for LtsBuilderFast {
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

    use itertools::Itertools;
    use rand::Rng;

    use merc_utilities::random_test;

    #[test]
    fn test_random_remove_duplicates() {
        random_test(100, |rng| {
            let mut builder = LtsBuilderFast::new(vec!["a".to_string(), "b".to_string(), "c".to_string()], Vec::new());

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
