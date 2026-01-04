#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;
use merc_collections::CompressedVecMetrics;
use merc_collections::bytevec;
use merc_io::LargeFormatter;
use merc_utilities::TagIndex;

use crate::LTS;
use crate::LabelIndex;
use crate::LabelTag;
use crate::StateIndex;
use crate::Transition;
use crate::TransitionLabel;

/// Represents a labelled transition system consisting of states with directed
/// labelled transitions between them.
///
/// # Details
///
/// Uses byte compressed vectors to store the states and their outgoing
/// transitions efficiently in memory.
#[derive(PartialEq, Eq, Clone)]
pub struct LabelledTransitionSystem<Label> {
    /// Encodes the states and their outgoing transitions.
    states: ByteCompressedVec<usize>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    /// Keeps track of the labels for every index, and which of them are hidden.
    labels: Vec<Label>,

    /// The index of the initial state.
    initial_state: StateIndex,
}

impl<Label: TransitionLabel> LabelledTransitionSystem<Label> {
    /// Creates a new a labelled transition system with the given transitions,
    /// labels, and hidden labels.
    ///
    /// The initial state is the state with the given index. `num_of_states`` is
    /// the number of states in the LTS, if known. If it is not known, pass
    /// `None`. However, in that case the number of states will be determined
    /// based on the maximum state index in the transitions. And all states that
    /// do not have any outgoing transitions will simply be created as deadlock
    /// states.
    pub fn new<I, F>(
        initial_state: StateIndex,
        num_of_states: Option<usize>,
        mut transition_iter: F,
        labels: Vec<Label>,
    ) -> LabelledTransitionSystem<Label>
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

    /// Constructs a LTS by the the a successor function for every state.
    pub fn with_successors<F, I>(
        initial_state: StateIndex,
        num_of_states: usize,
        labels: Vec<Label>,
        mut successors: F,
    ) -> Self
    where
        F: FnMut(StateIndex) -> I,
        I: Iterator<Item = (LabelIndex, StateIndex)>,
    {
        assert!(
            *labels
                .first()
                .expect("At least one label (the hidden label) must be provided")
                == Label::tau_label(),
            "The first label must be the hidden label."
        );

        let mut states = ByteCompressedVec::new();
        states.resize_with(num_of_states, Default::default);

        let mut transition_labels = ByteCompressedVec::with_capacity(num_of_states, 16usize.bytes_required());
        let mut transition_to = ByteCompressedVec::with_capacity(num_of_states, num_of_states.bytes_required());

        for state_index in 0..num_of_states {
            let state_index = StateIndex::new(state_index);
            states.update(*state_index, |entry| {
                *entry = transition_labels.len();
            });

            for (label, to) in successors(state_index) {
                transition_labels.push(label);
                transition_to.push(to);
            }
        }

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
    fn merge_disjoint_impl(mut self, other: &impl LTS<Label = Label>) -> (Self, StateIndex) {
        // Determine the combination of action labels
        let mut all_labels = self.labels().to_vec();
        for label in other.labels() {
            if !all_labels.contains(label) {
                all_labels.push(label.clone());
            }
        }

        let label_indices: HashMap<Label, TagIndex<usize, LabelTag>> = HashMap::from_iter(
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
    pub fn new_from_permutation<P>(lts: Self, permutation: P) -> Self
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

    /// Consumes the LTS and relabels its transition labels according to the given mapping.
    pub fn relabel<L: TransitionLabel>(self, labelling: impl Fn(Label) -> L) -> LabelledTransitionSystem<L> {
        let new_labels: Vec<L> = self.labels.iter().cloned().map(labelling).collect();

        LabelledTransitionSystem {
            initial_state: self.initial_state,
            labels: new_labels,
            states: self.states,
            transition_labels: self.transition_labels,
            transition_to: self.transition_to,
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

impl<L: TransitionLabel> LTS for LabelledTransitionSystem<L> {
    type Label = L;

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

    fn iter_states(&self) -> impl Iterator<Item = StateIndex> + '_ {
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

    fn labels(&self) -> &[Self::Label] {
        &self.labels[0..]
    }

    fn is_hidden_label(&self, label_index: LabelIndex) -> bool {
        label_index.value() == 0
    }

    fn merge_disjoint<T: LTS<Label = Self::Label>>(self, other: &T) -> (Self, StateIndex) {
        self.merge_disjoint_impl(other)
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

/// Checks that two LTSs are equivalent, for testing purposes.
#[cfg(test)]
pub fn check_equivalent<L: LTS>(lts: &L, lts_read: &L) {    
    println!("LTS labels: {:?}", lts.labels());
    println!("Read LTS labels: {:?}", lts_read.labels());

    // If labels are not used, the number of labels may be less. So find a remapping of old labels to new labels.
    let mapping = lts
        .labels()
        .iter()
        .enumerate()
        .map(|(_i, label)| lts_read.labels().iter().position(|l| l == label))
        .collect::<Vec<_>>();

    // Print the mapping
    for (i, m) in mapping.iter().enumerate() {
        println!("Label {} mapped to {:?}", i, m);
    }

    assert_eq!(lts.num_of_states(), lts_read.num_of_states());
    assert_eq!(lts.num_of_transitions(), lts_read.num_of_transitions());

    // Check that all the outgoing transitions are the same.
    for state_index in lts.iter_states() {
        let transitions: Vec<_> = lts.outgoing_transitions(state_index).collect();
        let transitions_read: Vec<_> = lts_read.outgoing_transitions(state_index).collect();

        // Check that transitions are the same, modulo label remapping.
        transitions.iter().for_each(|t| {
            let mapped_label = mapping[t.label.value()].expect(&format!("Label {} should be found", t.label));
            assert!(
                transitions_read
                    .iter()
                    .any(|tr| tr.to == t.to && tr.label.value() == mapped_label)
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_utilities::random_test;

    use crate::random_lts;
    use crate::write_aut;

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_labelled_transition_system_merge() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_labelled_transition_system_merge");

            let left = random_lts(rng, 5, 5, 10);
            let right = random_lts(rng, 5, 10, 10);

            files.dump("left.aut", |f| write_aut(f, &left)).unwrap();
            files.dump("right.aut", |f| write_aut(f, &right)).unwrap();

            let (merged, _offset) = left.clone().merge_disjoint_impl(&right);

            files.dump("merged.aut", |f| write_aut(f, &merged)).unwrap();
        })
    }
}
