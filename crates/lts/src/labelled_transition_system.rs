use std::collections::HashMap;
use std::fmt;

use merc_utilities::ByteCompressedVec;
use merc_utilities::CompressedEntry;
use merc_utilities::CompressedVecMetrics;
use merc_utilities::LargeFormatter;
use merc_utilities::TagIndex;
use merc_utilities::bytevec;

/// A unique type for the labels.
pub struct LabelTag;

/// A unique type for the states.
pub struct StateTag;

/// The index type for a label.
pub type LabelIndex = TagIndex<usize, LabelTag>;

/// The index for a state.
pub type StateIndex = TagIndex<usize, StateTag>;

pub trait LTS
where
    Self: Sized,
{
    /// Returns the index of the initial state
    fn initial_state_index(&self) -> StateIndex;

    /// Returns the set of outgoing transitions for the given state.
    fn outgoing_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_;

    /// Iterate over all state_index in the labelled transition system
    fn iter_states(&self) -> impl Iterator<Item = StateIndex> + use<Self>;

    /// Returns the number of states.
    fn num_of_states(&self) -> usize;

    /// Returns the number of labels.
    fn num_of_labels(&self) -> usize;

    /// Returns the number of transitions.
    fn num_of_transitions(&self) -> usize;

    /// Returns the list of labels.
    fn labels(&self) -> &[String];

    /// Returns true iff the given label index is a hidden label.
    fn is_hidden_label(&self, label_index: LabelIndex) -> bool;

    /// Consumes the current LTS and merges it with another one, returning the
    /// disjoint merged LTS and the initial state of the other LTS in the merged
    /// LTS.
    fn merge_disjoint(self, other: &Self) -> (Self, StateIndex);
}

/// Represents a labelled transition system consisting of states with directed
/// labelled transitions between them.
///
/// # Details
///
/// This LTS uses (dense) indices to refer to states and labels. The state indices
/// are represented as `StateIndex`, and the label indices as `LabelIndex`.
#[derive(PartialEq, Eq, Clone)]
pub struct LabelledTransitionSystem {
    /// Encodes the states and their outgoing transitions.
    states: ByteCompressedVec<usize>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    /// Keeps track of the labels for every index, and which of them are hidden.
    labels: Vec<String>,

    /// The index of the initial state.
    initial_state: StateIndex,
}

impl LabelledTransitionSystem {
    /// Creates a new a labelled transition system with the given transitions,
    /// labels, and hidden labels.
    ///
    /// The initial state is the state with the given index. num_of_states is
    /// the number of states in the LTS, if known. If it is not known, pass
    /// None. However, in that case the number of states will be determined
    /// based on the maximum state index in the transitions. And all states that
    /// do not have any outgoing transitions will simply be created as deadlock
    /// states.
    pub fn new<I, F>(
        initial_state: StateIndex,
        num_of_states: Option<usize>,
        mut transition_iter: F,
        labels: Vec<String>,
    ) -> LabelledTransitionSystem
    where
        F: FnMut() -> I,
        I: Iterator<Item = (StateIndex, LabelIndex, StateIndex)>,
    {
        let mut states = ByteCompressedVec::new();
        if let Some(num_of_states) = num_of_states {
            states.resize_with(num_of_states, Default::default);
            debug_assert!(
                initial_state.value() < num_of_states,
                "Initial vertex index {} out of bounds {num_of_states}",
                initial_state.value()
            );
        }

        // Count the number of transitions for every state
        let mut num_of_transitions = 0;
        for (from, _, to) in transition_iter() {
            // Ensure that the states vector is large enough.
            if states.len() <= *from.max(to) {
                states.resize_with(*from.max(to) + 1, || 0);
            }

            states.update(*from, |start| *start += 1);
            num_of_transitions += 1;

            if let Some(num_of_states) = num_of_states {
                debug_assert!(
                    *from < num_of_states && *to < num_of_states,
                    "State index out of bounds: from {:?}, to {:?}, num_of_states {}",
                    from,
                    to,
                    num_of_states
                );
            }
        }

        if initial_state.value() >= states.len() {
            // Ensure that the initial state is a valid state (and all states before it exist).
            states.resize_with(initial_state.value() + 1, Default::default);
        }

        // Track the number of transitions before every state.
        states.fold(0, |count, start| {
            let result = count + *start;
            *start = count;
            result
        });

        // Place the transitions, and increment the end for every state.
        let mut transition_labels = bytevec![LabelIndex::new(labels.len()); num_of_transitions];
        let mut transition_to = bytevec![StateIndex::new(states.len()); num_of_transitions];
        for (from, label, to) in transition_iter() {
            states.update(*from, |start| {
                transition_labels.set(*start, label);
                transition_to.set(*start, to);
                *start += 1
            });
        }

        // Reset the offset.
        states.fold(0, |previous, start| {
            let result = *start;
            *start = previous;
            result
        });

        // The minus one is because we added one extra state for the sentinel.
        debug_assert!(
            initial_state.value() < states.len(),
            "Initial state {:?} out of bounds (num states: {})",
            initial_state,
            states.len() - 1
        );

        // Add the sentinel state.
        states.push(transition_labels.len());

        LabelledTransitionSystem {
            initial_state,
            labels,
            states,
            transition_labels,
            transition_to,
        }
    }

    /// Consumes the current LTS and merges it with another one, returning the merged LTS.
    ///
    /// # Details
    ///
    /// Internally this works by offsetting the state indices of the other LTS by the number of states
    /// in the current LTS, and combining the action labels. The offset is returned such that
    /// can find the states of the other LTS in the merged LTS as the initial state of the other LTS.
    fn merge_disjoint_impl(mut self, other: &impl LTS) -> (LabelledTransitionSystem, StateIndex) {
        // Determine the combination of action labels
        let mut all_labels = self.labels().to_vec();
        for label in other.labels() {
            if !all_labels.contains(label) {
                all_labels.push(label.clone());
            }
        }

        let label_indices: HashMap<String, TagIndex<usize, LabelTag>> = HashMap::from_iter(
            all_labels
                .iter()
                .enumerate()
                .map(|(i, label)| (label.clone(), LabelIndex::new(i))),
        );

        let total_number_of_states = self.num_of_states() + other.num_of_states();

        // Reserve space for the right LTS.
        self.states
            .reserve(other.num_of_states(), total_number_of_states.bytes_required());
        self.transition_labels
            .reserve(other.num_of_transitions(), all_labels.len().bytes_required());
        self.transition_to
            .reserve(other.num_of_transitions(), total_number_of_states.bytes_required());

        let offset = self.num_of_states();

        // Remove the sentinel state temporarily. This breaks the state invariant, but we will add it back later.
        self.states.pop();

        // Add vertices for the other LTS that are offset by the number of states in self
        for state_index in other.iter_states() {
            // Add a new state for every state in the other LTS
            self.states.push(self.num_of_transitions());
            for transition in other.outgoing_transitions(state_index) {
                // Add the transitions of the other LTS, offsetting the state indices
                self.transition_to.push(StateIndex::new(transition.to.value() + offset));

                // Map the label to the new index in all_labels
                let label_name = &other.labels()[transition.label.value()];
                self.transition_labels
                    .push(*label_indices.get(label_name).expect("Label should exist in all_labels"));
            }
        }

        // Add back the sentinel state
        self.states.push(self.num_of_transitions());
        debug_assert_eq!(self.num_of_states(), total_number_of_states);

        (
            Self {
                initial_state: self.initial_state,
                labels: all_labels,
                states: self.states,
                transition_labels: self.transition_labels,
                transition_to: self.transition_to,
            },
            StateIndex::new(offset + other.initial_state_index().value()),
        )
    }

    /// Creates a labelled transition system from another one, given the permutation of state indices
    ///
    pub fn new_from_permutation<P>(lts: LabelledTransitionSystem, permutation: P) -> Self
    where
        P: Fn(StateIndex) -> StateIndex + Copy,
    {
        let mut states = bytevec![0; lts.num_of_states()];

        for state_index in lts.iter_states() {
            // Keep the transitions the same move the state indices around
            let new_state_index = permutation(state_index);
            let state = lts.states.index(*state_index);
            states.update(*new_state_index, |entry| {
                *entry = state;
            });
        }

        // Add the sentinel state.
        states.push(lts.num_of_transitions());

        LabelledTransitionSystem {
            initial_state: permutation(lts.initial_state),
            labels: lts.labels,
            states,
            transition_labels: lts.transition_labels,
            transition_to: lts.transition_to,
        }
    }

    /// Returns metrics about the LTS.
    pub fn metrics(&self) -> LtsMetrics {
        LtsMetrics {
            num_of_states: self.num_of_states(),
            num_of_labels: self.num_of_labels(),
            num_of_transitions: self.num_of_transitions(),
            state_metrics: self.states.metrics(),
            transition_labels_metrics: self.transition_labels.metrics(),
            transition_to_metrics: self.transition_to.metrics(),
        }
    }
}

impl LTS for LabelledTransitionSystem {
    fn initial_state_index(&self) -> StateIndex {
        self.initial_state
    }

    fn outgoing_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let start = self.states.index(*state_index);
        let end = self.states.index(*state_index + 1);

        (start..end).map(move |i| Transition {
            label: self.transition_labels.index(i),
            to: self.transition_to.index(i),
        })
    }

    fn iter_states(&self) -> impl Iterator<Item = StateIndex> + use<> {
        (0..self.num_of_states()).map(StateIndex::new)
    }

    fn num_of_states(&self) -> usize {
        // Remove the sentinel state.
        self.states.len() - 1
    }

    fn num_of_labels(&self) -> usize {
        self.labels.len()
    }

    fn num_of_transitions(&self) -> usize {
        self.transition_labels.len()
    }

    fn labels(&self) -> &[String] {
        &self.labels[0..]
    }

    fn is_hidden_label(&self, label_index: LabelIndex) -> bool {
        label_index.value() == 0
    }

    fn merge_disjoint(self, other: &Self) -> (Self, StateIndex) {
        self.merge_disjoint_impl(other)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transition {
    pub label: LabelIndex,
    pub to: StateIndex,
}

impl Transition {
    /// Constructs a new transition.
    pub fn new(label: LabelIndex, to: StateIndex) -> Self {
        Self { label, to }
    }
}

/// Metrics for a labelled transition system.
#[derive(Debug, Clone)]
pub struct LtsMetrics {
    /// The number of states in the LTS.
    pub num_of_states: usize,
    pub state_metrics: CompressedVecMetrics,
    /// The number of transitions in the LTS.
    pub num_of_transitions: usize,
    pub transition_labels_metrics: CompressedVecMetrics,
    pub transition_to_metrics: CompressedVecMetrics,
    /// The number of action labels in the LTS.
    pub num_of_labels: usize,
}

impl fmt::Display for LtsMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print some information about the LTS.
        writeln!(f, "Number of states: {}", LargeFormatter(self.num_of_states))?;
        writeln!(f, "Number of action labels: {}", LargeFormatter(self.num_of_labels))?;
        writeln!(
            f,
            "Number of transitions: {}\n",
            LargeFormatter(self.num_of_transitions)
        )?;
        writeln!(f, "Memory usage:")?;
        writeln!(f, "States {}", self.state_metrics)?;
        writeln!(f, "Transition labels {}", self.transition_labels_metrics)?;
        write!(f, "Transition to {}", self.transition_to_metrics)
    }
}

impl fmt::Debug for LabelledTransitionSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Initial state: {}", self.initial_state)?;

        for state_index in self.iter_states() {
            for transition in self.outgoing_transitions(state_index) {
                let label_name = &self.labels[transition.label.value()];

                writeln!(f, "{state_index} --[{label_name}]-> {}", transition.to)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::random_lts;

    #[test]
    fn test_labelled_transition_system_merge() {
        random_test(1, |rng| {
            let left = random_lts(rng, 5, 5, 10);
            let right = random_lts(rng, 5, 10, 10);

            println!("Left LTS:\n{:?}", left);
            println!("Right LTS:\n{:?}", right);
            let (merged, _offset) = left.clone().merge_disjoint_impl(&right);

            println!("Merged LTS:\n{:?}", merged);
        })
    }
}
