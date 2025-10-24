use std::fmt;

use log::debug;
use mcrl3_utilities::ByteCompressedVec;
use mcrl3_utilities::CompressedEntry;
use mcrl3_utilities::TagIndex;
use mcrl3_utilities::bytevec;

/// A unique type for the labels.
pub struct LabelTag;

/// A unique type for the labels.
pub struct StateTag;

/// The index type for a label.
pub type LabelIndex = TagIndex<usize, LabelTag>;

/// The index for a state.
pub type StateIndex = TagIndex<usize, StateTag>;

/// Represents a labelled transition system consisting of states with directed
/// labelled edges.
#[derive(PartialEq, Eq, Clone)]
pub struct LabelledTransitionSystem {
    /// Encodes the states and their outgoing transitions.
    states: ByteCompressedVec<State>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    /// Keeps track of the labels for every index, and which of them are hidden.
    labels: Vec<String>,
    hidden_labels: Vec<String>,

    /// The index of the initial state.
    initial_state: StateIndex,
}

impl LabelledTransitionSystem {
    /// Creates a new a labelled transition system with the given transitions, labels, and hidden labels.
    ///
    /// The initial state is the state with the given index.
    /// num_of_states is the number of states in the LTS, if known.
    pub fn new<I, F>(
        initial_state: StateIndex,
        num_of_states: Option<usize>,
        mut transition_iter: F,
        mut labels: Vec<String>,
        hidden_labels: Vec<String>,
    ) -> LabelledTransitionSystem
    where
        F: FnMut() -> I,
        I: Iterator<Item = (StateIndex, LabelIndex, StateIndex)>,
    {
        let mut states = ByteCompressedVec::new();
        if let Some(num_of_states) = num_of_states {
            states.resize_with(num_of_states, Default::default);
        }

        // Count the number of transitions for every state
        let mut num_of_transitions = 0;
        for (from, _, to) in transition_iter() {
            // Ensure that the states vector is large enough.
            while states.len() <= *from.max(to) {
                states.push(State::default());
            }

            states.update(*from, |entry| entry.outgoing_start += 1);
            num_of_transitions += 1;
        }

        // Track the number of transitions before every state.
        states.fold(0, |count, state| {
            let result = count + state.outgoing_start;
            *state = State::new(count);
            result
        });

        // Place the transitions, and increment the end for every state.
        let mut transition_labels = bytevec![LabelIndex::new(0); num_of_transitions];
        let mut transition_to = bytevec![StateIndex::new(0); num_of_transitions];
        for (from, label, to) in transition_iter() {
            states.update(*from, |entry| {
                transition_labels.set(entry.outgoing_start, label);
                transition_to.set(entry.outgoing_start, to);
                entry.outgoing_start += 1
            });
        }

        // Reset the offset.
        states.fold(0, |previous, state| {
            let result = state.outgoing_start;
            state.outgoing_start = previous;
            result
        });

        // Add the sentinel state.
        states.push(State::new(transition_labels.len()));

        // Keep track of which label indexes are hidden labels.
        let mut hidden_indices: Vec<usize> = Vec::new();
        for label in &hidden_labels {
            if let Some(index) = labels.iter().position(|other| other == label) {
                hidden_indices.push(index);
            }
        }
        hidden_indices.sort();

        // Make an implicit tau label the first label.
        let introduced_tau = if hidden_indices.contains(&0) {
            labels[0] = "tau".to_string();
            false
        } else {
            labels.insert(0, "tau".to_string());
            true
        };

        // Remap all hidden actions to zero.
        transition_labels.map(|label| {
            if hidden_indices.binary_search(&label.value()).is_ok() {
                *label = LabelIndex::new(0);
            } else if introduced_tau {
                // Remap all labels to not be the zero hidden action.
                *label = LabelIndex::new(label.value() + 1);
            }
        });

        LabelledTransitionSystem {
            initial_state,
            labels,
            hidden_labels,
            states,
            transition_labels,
            transition_to,
        }
    }

    /// Creates a labelled transition system from another one, given the permutation of state indices
    ///
    pub fn new_from_permutation<P>(lts: LabelledTransitionSystem, permutation: P) -> Self
    where
        P: Fn(StateIndex) -> StateIndex + Copy,
    {
        let mut states = bytevec![State::default(); lts.num_of_states()];

        for state_index in lts.iter_states() {
            // Keep the transitions the same move the state indices around
            let new_state_index = permutation(state_index);
            let state = lts.states.index(*state_index);
            states.update(*new_state_index, |entry| {
                *entry = state.clone();
            });
        }

        // Add the sentinel state.
        states.push(State::new(lts.num_of_transitions()));

        LabelledTransitionSystem {
            initial_state: permutation(lts.initial_state),
            labels: lts.labels,
            hidden_labels: lts.hidden_labels,
            states,
            transition_labels: lts.transition_labels,
            transition_to: lts.transition_to,
        }
    }

    /// Returns the index of the initial state
    pub fn initial_state_index(&self) -> StateIndex {
        self.initial_state
    }

    /// Returns the set of outgoing transitions for the given state.
    pub fn outgoing_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_ {
        let state = &self.states.index(*state_index);
        let next_state = &self.states.index(*state_index + 1);
        let start = state.outgoing_start;
        let end = next_state.outgoing_start;

        (start..end).map(move |i| Transition {
            label: self.transition_labels.index(i),
            to: self.transition_to.index(i),
        })
    }

    /// Iterate over all state_index in the labelled transition system
    pub fn iter_states(&self) -> impl Iterator<Item = StateIndex> + use<> {
        (0..self.num_of_states()).map(StateIndex::new)
    }

    /// Returns the number of states.
    pub fn num_of_states(&self) -> usize {
        // Remove the sentinel state.
        self.states.len() - 1
    }

    /// Returns the number of labels.
    pub fn num_of_labels(&self) -> usize {
        self.labels.len()
    }

    /// Returns the number of transitions.
    pub fn num_of_transitions(&self) -> usize {
        self.transition_labels.len()
    }

    /// Returns the list of labels.
    pub fn labels(&self) -> &[String] {
        &self.labels[0..]
    }

    /// Returns the list of hidden labels.
    pub fn hidden_labels(&self) -> &[String] {
        &self.hidden_labels[0..]
    }

    /// Returns true iff the given label index is a hidden label.
    pub fn is_hidden_label(&self, label_index: LabelIndex) -> bool {
        label_index.value() == 0
    }
}

/// A single state in the LTS, containing a vector of outgoing edges.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
struct State {
    outgoing_start: usize,
}

impl State {
    /// Constructs a new state
    fn new(outgoing_start: usize) -> Self {
        Self { outgoing_start }
    }
}

impl CompressedEntry for State {
    fn to_bytes(&self, bytes: &mut [u8]) {
        self.outgoing_start.to_bytes(bytes);
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            outgoing_start: usize::from_bytes(bytes),
        }
    }

    fn bytes_required(&self) -> usize {
        self.outgoing_start.bytes_required()
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

impl fmt::Display for LabelledTransitionSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print some information about the LTS.
        writeln!(f, "Number of states: {}", self.num_of_states())?;
        writeln!(f, "Number of action labels: {}", self.labels.len())?;
        writeln!(f, "Number of transitions: {}", self.num_of_transitions())?;
        writeln!(f, "States {}", self.states.metrics());
        writeln!(f, "Transition labels {}", self.transition_labels.metrics());
        write!(f, "Transition to {}", self.transition_to.metrics())
    }
}

impl fmt::Debug for LabelledTransitionSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{self}")?;
        writeln!(f, "Initial state: {}", self.initial_state)?;
        writeln!(f, "Hidden labels: {:?}", self.hidden_labels)?;

        for state_index in self.iter_states() {
            for transition in self.outgoing_transitions(state_index) {
                let label_name = &self.labels[transition.label.value()];

                writeln!(f, "{state_index} --[{label_name}]-> {}", transition.to)?;
            }
        }

        Ok(())
    }
}
