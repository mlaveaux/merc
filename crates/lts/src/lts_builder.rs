use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;
use crate::TransitionLabel;

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
pub struct LtsBuilder<L> {
    transition_from: ByteCompressedVec<StateIndex>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    // This is used to keep track of the label to index mapping.
    labels_index: HashMap<L, LabelIndex>,
    labels: Vec<L>,

    /// The number of states (derived from the transitions).
    num_of_states: usize,
}

impl<L: TransitionLabel> LtsBuilder<L> {
    /// Initializes a new empty builder.
    pub fn new(labels: Vec<L>, hidden_labels: Vec<String>) -> Self {
        Self::with_capacity(labels, hidden_labels, 0, 0, 0)
    }

    /// Initializes the builder with pre-allocated capacity for states and transitions. The number of labels
    /// can be used when labels are added dynamically.
    pub fn with_capacity(
        mut labels: Vec<L>,
        hidden_labels: Vec<String>,
        num_of_labels: usize,
        num_of_states: usize,
        num_of_transitions: usize,
    ) -> Self {
        // Remove duplicates from the labels.
        labels.sort();
        labels.dedup();

        // Introduce the fixed 0-indexed tau label.
        if let Some(tau_pos) = labels.iter().position(|l| l.is_tau_label()) {
            labels.swap(0, tau_pos);
        } else {
            labels.insert(0, L::tau_label());
        }

        // Ensure that all hidden labels are mapped to the tau action.
        let mut labels_index = HashMap::new();
        labels_index.insert(L::tau_label(), LabelIndex::new(0));
        for (index, label) in labels.iter().enumerate() {
            if hidden_labels.iter().any(|l| label.matches_label(l)) {
                labels_index.insert(label.clone(), LabelIndex::new(0)); // Map hidden labels to tau
            } else {
                labels_index.insert(label.clone(), LabelIndex::new(index));
            }
        }

        Self {
            transition_from: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
            transition_labels: ByteCompressedVec::with_capacity(
                num_of_transitions,
                num_of_labels.max(labels.len()).bytes_required(),
            ),
            transition_to: ByteCompressedVec::with_capacity(num_of_transitions, num_of_states.bytes_required()),
            labels_index,
            labels,
            num_of_states: 0,
        }
    }

    /// Adds a transition to the builder. For efficiently reasons, we can use
    /// another type `Q` for the label.
    pub fn add_transition<Q: ?Sized>(&mut self, from: StateIndex, label: &Q, to: StateIndex)
    where
        L: Borrow<Q>,
        Q: ToOwned<Owned = L> + Eq + Hash,
    {
        let label_index = if let Some(&index) = self.labels_index.get(label) {
            index
        } else {
            let index = LabelIndex::new(self.labels.len());
            self.labels_index.insert(label.to_owned(), index);
            self.labels.push(label.to_owned());
            index
        };

        self.transition_from.push(from);
        self.transition_labels.push(label_index);
        self.transition_to.push(to);

        // Update the number of states.
        self.num_of_states = self.num_of_states.max(from.value() + 1).max(to.value() + 1);
    }

    /// Finalizes the builder and returns the constructed labelled transition system.
    pub fn finish(&mut self, initial_state: StateIndex) -> LabelledTransitionSystem<L> {
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

    /// Returns the number of states added to the builder.
    pub fn num_of_states(&self) -> usize {
        self.num_of_states
    }

    /// Ensures that the builder has at least the given number of states.
    pub fn require_num_of_states(&mut self, num_of_states: usize) {
        if num_of_states > self.num_of_states {
            self.num_of_states = num_of_states;
        }
    }

    /// Returns an iterator over all transitions as (from, label, to) tuples.
    pub fn iter(&self) -> impl Iterator<Item = (StateIndex, LabelIndex, StateIndex)> {
        self.transition_from
            .iter()
            .zip(self.transition_labels.iter())
            .zip(self.transition_to.iter())
            .map(|((from, label), to)| (from, label, to))
    }
}

impl<Label: TransitionLabel> fmt::Debug for LtsBuilder<Label> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Transitions:")?;
        for (from, label, to) in self.iter() {
            writeln!(f, "    {:?} --[{:?}]-> {:?}", from, label, to)?;
        }
        Ok(())
    }
}
