#![forbid(unsafe_code)]

use std::fmt;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;
use merc_collections::CompressedVecMetrics;
use merc_collections::bytevec;
use merc_io::LargeFormatter;
use merc_utilities::MercError;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

use crate::LTS;
use crate::LabelIndex;
use crate::StateIndex;
use crate::StateLabels;
use crate::Transition;
use crate::TransitionLabel;

/// Represents a labelled transition system consisting of states with directed
/// labelled transitions between them.
///
/// # Details
///
/// Uses byte compressed vectors to store the states and their outgoing
/// transitions efficiently in memory. The states are unlabelled by default,
/// see [StateLabels] for the state labels type `S`.
#[derive(PartialEq, Eq, Clone)]
pub struct LabelledTransitionSystem<Label, S = ()> {
    /// Encodes the states and their outgoing transitions.
    states: ByteCompressedVec<usize>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    /// Keeps track of the labels for every index, and which of them are hidden.
    labels: Vec<Label>,

    /// The index of the initial state.
    initial_state: StateIndex,

    /// The labels of the states, if any.
    state_labels: S,
}

impl<Label: TransitionLabel> LabelledTransitionSystem<Label> {
    /// Creates a new labelled transition system from a transition iterator factory.
    ///
    /// `num_of_states`, when known, fixes the total state count so that states
    /// with no outgoing transitions still exist as deadlocks; when `None` it is
    /// inferred from the highest state index seen in the transitions. `labels[0]`
    /// must be the tau label.
    ///
    /// # Panics
    ///
    /// `transition_iter` is called twice and must produce the same sequence of
    /// transitions both times. Panics (in debug mode) if a transition index is
    /// out of bounds for a given `num_of_states`, or if `labels` does not start
    /// with the tau label.
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

        // Add the sentinel state.
        states.push(transition_labels.len());

        LabelledTransitionSystem::from_raw_parts(initial_state, states, transition_labels, transition_to, labels)
    }

    /// Constructs a LTS by a successor function for every state.
    ///
    /// `labels[0]` must be the tau label, and `successors` is called once per
    /// state in `0..num_of_states`.
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

        Self::from_raw_parts(initial_state, states, transition_labels, transition_to, labels)
    }

    /// Creates a labelled transition system from another one, given the permutation of state indices.
    ///
    /// The permutation maps old state indices to new state indices, i.e.,
    /// `permutation(old) = new`. The transition arrays are rebuilt so that
    /// transitions are contiguous per new state index, and all transition
    /// targets are updated to reference the new state indices.
    ///
    /// # Panics
    ///
    /// `permutation` must be a bijection on `0..lts.num_of_states()`; otherwise the result
    /// violates the internal LTS invariants and panics (in debug mode) inside `assert_valid`.
    pub fn new_from_permutation<P>(lts: Self, permutation: P) -> Self
    where
        P: Fn(StateIndex) -> StateIndex + Copy,
    {
        // Build the inverse permutation: inverse[new_index] = old_index
        let mut inverse = vec![StateIndex::new(0); lts.num_of_states()];
        for state_index in lts.iter_states() {
            inverse[*permutation(state_index)] = state_index;
        }

        // Rebuild transition arrays in the order of the new state indices.
        let mut states = ByteCompressedVec::new();
        let mut transition_labels = ByteCompressedVec::new();
        let mut transition_to = ByteCompressedVec::new();

        for old_index in &inverse {
            states.push(transition_labels.len());

            let start = lts.states.index(**old_index);
            let end = lts.states.index(**old_index + 1);

            for i in start..end {
                transition_labels.push(lts.transition_labels.index(i));
                transition_to.push(permutation(lts.transition_to.index(i)));
            }
        }

        // Add the sentinel state.
        states.push(transition_labels.len());

        Self::from_raw_parts(
            permutation(lts.initial_state),
            states,
            transition_labels,
            transition_to,
            lts.labels,
        )
    }

    /// Constructs a [LabelledTransitionSystem] directly from its raw internal arrays.
    ///
    /// The `states` array must contain one entry per state holding the start offset of that
    /// state's transitions in the transition arrays, plus a sentinel entry at the end equal
    /// to the total number of transitions. `transition_labels` and `transition_to` must have
    /// equal length and all indices they contain must be in bounds.
    ///
    /// # Panics
    ///
    /// Panics (in debug mode) if the invariants of the internal representation are violated.
    pub fn from_raw_parts(
        initial_state: StateIndex,
        states: ByteCompressedVec<usize>,
        transition_labels: ByteCompressedVec<LabelIndex>,
        transition_to: ByteCompressedVec<StateIndex>,
        labels: Vec<Label>,
    ) -> Self {
        let lts = LabelledTransitionSystem {
            initial_state,
            states,
            transition_labels,
            transition_to,
            labels,
            state_labels: (),
        };
        lts.assert_valid();
        lts
    }
}

impl<Label: TransitionLabel, S: StateLabels> LabelledTransitionSystem<Label, S> {
    /// Consumes the LTS and replaces its state labels by `state_labels`.
    ///
    /// # Panics
    ///
    /// Panics if `state_labels` is neither empty nor has a label for every state.
    pub fn with_state_labels<T: StateLabels>(self, state_labels: T) -> LabelledTransitionSystem<Label, T> {
        let lts = LabelledTransitionSystem {
            initial_state: self.initial_state,
            states: self.states,
            transition_labels: self.transition_labels,
            transition_to: self.transition_to,
            labels: self.labels,
            state_labels,
        };
        lts.assert_valid();
        lts
    }

    /// Consumes the current LTS and merges it with another one, returning the merged LTS.
    ///
    /// # Details
    ///
    /// Internally this works by offsetting the state indices of the other LTS by the number of states
    /// in the current LTS, and combining the action labels. The offset is returned such that
    /// can find the states of the other LTS in the merged LTS as the initial state of the other LTS.
    /// The state labels are dropped.
    fn merge_disjoint_impl<L: LTS<Label = Label>>(
        mut self,
        other: &L,
    ) -> (LabelledTransitionSystem<Label>, StateIndex) {
        // Determine the combination of action labels, keeping a map from label to
        // its index so membership checks stay O(1) instead of scanning the vector.
        let mut all_labels = self.labels().to_vec();
        let mut label_indices: FxHashMap<Label, LabelIndex> = all_labels
            .iter()
            .enumerate()
            .map(|(i, label)| (label.clone(), LabelIndex::new(i)))
            .collect();

        for label in other.labels() {
            if !label_indices.contains_key(label) {
                label_indices.insert(label.clone(), LabelIndex::new(all_labels.len()));
                all_labels.push(label.clone());
            }
        }

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
            LabelledTransitionSystem::from_raw_parts(
                self.initial_state,
                self.states,
                self.transition_labels,
                self.transition_to,
                all_labels,
            ),
            StateIndex::new(offset + other.initial_state_index().value()),
        )
    }

    /// Consumes the LTS and relabels its transition labels according to the
    /// given mapping.
    ///
    /// Note that this only relabels the visible labels, since the hidden label
    /// must be kept consistent. The state labels are kept.
    pub fn relabel<L, F>(self, labelling: F) -> Result<LabelledTransitionSystem<L, S>, MercError>
    where
        F: Fn(Label) -> Result<L, MercError>,
        L: TransitionLabel,
    {
        let new_labels: Vec<L> = self
            .labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                if index == 0 {
                    Ok(L::tau_label())
                } else {
                    labelling(label)
                }
            })
            .collect::<Result<_, _>>()?;

        Ok(LabelledTransitionSystem::from_raw_parts(
            self.initial_state,
            self.states,
            self.transition_labels,
            self.transition_to,
            new_labels,
        )
        .with_state_labels(self.state_labels))
    }

    /// A [Self::relabel] variant that also applies to the tau label. This is
    /// useful when the tau label cannot be constructed from `L::tau_label()`.
    pub fn relabel_all<L, F>(self, labelling: F) -> Result<LabelledTransitionSystem<L, S>, MercError>
    where
        F: Fn(Label) -> Result<L, MercError>,
        L: TransitionLabel,
    {
        let new_labels: Vec<L> = self.labels.into_iter().map(labelling).collect::<Result<_, _>>()?;

        Ok(LabelledTransitionSystem::from_raw_parts(
            self.initial_state,
            self.states,
            self.transition_labels,
            self.transition_to,
            new_labels,
        )
        .with_state_labels(self.state_labels))
    }

    /// Checks that the internal representation satisfies all structural invariants.
    pub fn assert_valid(&self) {
        assert!(
            self.labels
                .first()
                .expect("At least one label (the hidden label) must be provided")
                .is_tau_label(),
            "The first label must be the hidden label."
        );

        assert!(
            self.state_labels.is_empty() || self.state_labels.len() == self.num_of_states(),
            "There must be either no state labels or one for each of the {} states, found {}.",
            self.num_of_states(),
            self.state_labels.len()
        );

        // The remaining checks are debug assertions, but their loops would still visit every
        // state and transition in a release build.
        if !cfg!(debug_assertions) {
            return;
        }

        let num_states = self.num_of_states();
        let num_transitions = self.num_of_transitions();

        debug_assert!(
            !self.states.is_empty(),
            "states array must have at least one entry (the sentinel)"
        );

        debug_assert!(
            self.initial_state.value() < num_states,
            "initial_state {:?} is out of bounds (num_states: {})",
            self.initial_state,
            num_states
        );

        debug_assert_eq!(
            self.states.index(num_states),
            num_transitions,
            "sentinel value must equal the number of transitions"
        );

        debug_assert_eq!(
            self.transition_labels.len(),
            self.transition_to.len(),
            "transition_labels and transition_to must have equal length"
        );

        for i in 0..num_states {
            debug_assert!(
                self.states.index(i) <= self.states.index(i + 1),
                "state {i} has offset {} which is greater than successor offset {}",
                self.states.index(i),
                self.states.index(i + 1)
            );
        }

        for i in 0..num_transitions {
            let label = self.transition_labels.index(i);
            debug_assert!(
                label.value() < self.labels.len(),
                "transition {i} references label index {} which is out of bounds (num_labels: {})",
                label.value(),
                self.labels.len()
            );

            let to = self.transition_to.index(i);
            debug_assert!(
                to.value() < num_states,
                "transition {i} references target state {} which is out of bounds (num_states: {})",
                to.value(),
                num_states
            );
        }

        for state in 0..num_states {
            let start = self.states.index(state);
            let end = self.states.index(state + 1);
            let mut seen = FxHashSet::with_capacity_and_hasher(end.saturating_sub(start), Default::default());

            for i in start..end {
                let edge = (
                    self.transition_labels.index(i).value(),
                    self.transition_to.index(i).value(),
                );

                debug_assert!(
                    seen.insert(edge),
                    "state {state} has duplicate outgoing transition with label {} to state {}",
                    edge.0,
                    edge.1
                );
            }
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

impl<L: TransitionLabel, S: StateLabels> LTS for LabelledTransitionSystem<L, S> {
    type Label = L;
    type StateLabel<'a>
        = S::Label<'a>
    where
        Self: 'a;

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
        &self.labels
    }

    fn is_hidden_label(&self, label_index: LabelIndex) -> bool {
        label_index.value() == 0
    }

    fn state_label(&self, state_index: StateIndex) -> Option<S::Label<'_>> {
        self.state_labels.get(state_index)
    }

    fn merge_disjoint<T: LTS<Label = Self::Label>>(self, other: &T) -> (LabelledTransitionSystem<L>, StateIndex) {
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
