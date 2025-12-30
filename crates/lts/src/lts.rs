#![forbid(unsafe_code)]

//! The labelled transition system (LTS) trait and associated types.

use std::fmt;
use std::hash::Hash;

use merc_utilities::TagIndex;

use crate::LabelledTransitionSystem;

/// A unique type for the labels.
pub struct LabelTag;

/// A unique type for the states.
pub struct StateTag;

/// The index type for a label.
pub type LabelIndex = TagIndex<usize, LabelTag>;

/// The index for a state.
pub type StateIndex = TagIndex<usize, StateTag>;

/// The trait for labelled transition systems.
///
/// Uses (strong) indices to refer to states and labels. The state indices are
/// represented as `StateIndex`, and the label indices as `LabelIndex`. The
/// labels themselves are given by type `Label`.
pub trait LTS
where
    Self: Sized,
{
    /// The associated type for transition labels.
    type Label: TransitionLabel;

    /// Returns the index of the initial state
    fn initial_state_index(&self) -> StateIndex;

    /// Returns the set of outgoing transitions for the given state.
    fn outgoing_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition> + '_;

    /// Iterate over all state_index in the labelled transition system
    fn iter_states(&self) -> impl Iterator<Item = StateIndex> + '_;

    /// Returns the number of states.
    fn num_of_states(&self) -> usize;

    /// Returns the number of labels.
    fn num_of_labels(&self) -> usize;

    /// Returns the number of transitions.
    fn num_of_transitions(&self) -> usize;

    /// Returns the list of labels.
    fn labels(&self) -> &[Self::Label];

    /// Returns true iff the given label index is a hidden label.
    fn is_hidden_label(&self, label_index: LabelIndex) -> bool;

    /// Consumes the current LTS and merges it with another one, returning the
    /// disjoint merged LTS and the initial state of the other LTS in the merged
    /// LTS.
    ///
    /// TODO: Can this be generalised to returning `Self`?
    fn merge_disjoint<L: LTS<Label = Self::Label>>(
        self,
        other: &L,
    ) -> (LabelledTransitionSystem<Self::Label>, StateIndex);
}

/// A common trait for all transition labels. For various algorithms on LTSs we
/// require that  they are orderable, comparable, and hashable. So we require that here
/// instead of specifying these bounds on usage.
pub trait TransitionLabel: Ord + Hash + Eq + Clone + fmt::Display + fmt::Debug {
    /// Returns the tau label for this transition label type.
    fn tau_label() -> Self;

    /// Returns true iff this label is the tau label.
    fn is_tau_label(&self) -> bool {
        self == &Self::tau_label()
    }

    /// Returns true iff this label matches the given string label.
    fn matches_label(&self, label: &str) -> bool;

    /// Used for generating labels for the random LTSs
    fn from_index(i: usize) -> Self;
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
