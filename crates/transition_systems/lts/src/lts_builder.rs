#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::cell::RefCell;
use std::fmt;
use std::hash::Hash;
use std::sync::Mutex;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;
use merc_utilities::MercError;
use rustc_hash::FxHashMap;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;
use crate::TransitionLabel;

/// A trait for building labelled transition systems incrementally.
///
/// # Details
///
/// Depending on the implementation this can be done in a memory efficient way,
/// or in a way that is optimized for speed. Alternatively, the resulting LTS is
/// immediately written to disk. The builder accumulates transitions using
/// `add_transition`, and once all transitions have been added, the labelled
/// transition system can be constructed with `finish`. An initial state can
/// also be specified during finalization.
pub trait LtsBuilder<L: TransitionLabel> {
    /// The result type of the builder once finalized.
    type LTS;

    /// Adds a transition to the builder. For efficiency reasons, we can use
    /// another type `Q` for the label.
    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash;

    /// Finalizes the builder and returns the constructed labelled transition system.
    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError>;

    /// Returns the number of transitions added to the builder.
    fn num_of_transitions(&self) -> usize;

    /// Returns the number of states added to the builder.
    fn num_of_states(&self) -> usize;

    /// Ensures the builder accounts for at least `num_states` states, so that
    /// states without incident transitions (such as an isolated initial state)
    /// are still reflected in the result.
    fn require_num_of_states(&mut self, num_states: usize);
}

/// Delegates to `B`'s own implementation, so a `&mut B` can stand in wherever an owned builder is
/// expected, without having to take ownership of it.
impl<L: TransitionLabel, B: LtsBuilder<L> + ?Sized> LtsBuilder<L> for &mut B {
    type LTS = B::LTS;

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        (**self).add_transition(from, label, to)
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        (**self).finish(initial_state)
    }

    fn num_of_transitions(&self) -> usize {
        (**self).num_of_transitions()
    }

    fn num_of_states(&self) -> usize {
        (**self).num_of_states()
    }

    fn require_num_of_states(&mut self, num_states: usize) {
        (**self).require_num_of_states(num_states)
    }
}

/// A builder that discards all transitions, producing no output. This is useful
/// when an LTS only needs to be explored but the result is not required.
impl<L: TransitionLabel> LtsBuilder<L> for () {
    type LTS = ();

    fn add_transition<Q>(&mut self, _from: StateIndex, _label: &Q, _to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        Ok(())
    }

    fn finish(&mut self, _initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        Ok(())
    }

    fn num_of_transitions(&self) -> usize {
        0
    }

    fn num_of_states(&self) -> usize {
        0
    }

    fn require_num_of_states(&mut self, _num_states: usize) {}
}

/// A builder that additionally accepts transitions through a shared `&self`
/// reference, synchronising internally.
///
/// # Details
///
/// This is used by the parallel explorer, where several worker threads stream
/// transitions into a single builder at once. Implementors take care of their
/// own synchronisation, so callers need no external lock. The builder is
/// finalised through the [`LtsBuilder`] supertrait once exploration completes.
pub trait ConcurrentLtsBuilder<L: TransitionLabel>: LtsBuilder<L> + Sync {
    /// Adds a transition through a shared reference. See
    /// [`LtsBuilder::add_transition`].
    fn add_transition_shared<Q>(&self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash;
}

/// The discarding builder also discards concurrent transitions.
impl<L: TransitionLabel> ConcurrentLtsBuilder<L> for () {
    fn add_transition_shared<Q>(&self, _from: StateIndex, _label: &Q, _to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        Ok(())
    }
}

/// Adapts any [`LtsBuilder`] into a [`ConcurrentLtsBuilder`] by guarding it with
/// a `Mutex`.
///
/// Concurrent (`&self`) transitions simply lock the mutex and delegate to the
/// inner builder's [`LtsBuilder::add_transition`]; the single-threaded
/// (`&mut self`) operations access the builder without locking. This serialises
/// all writes, which is enough for builders that are cheap to write to (such as
/// [`crate::AutStream`]) while the expensive exploration happens outside the
/// lock.
pub struct MutexLtsBuilder<B> {
    inner: Mutex<B>,
}

impl<B> MutexLtsBuilder<B> {
    /// Wraps `builder` so it can be shared across worker threads.
    pub fn new(builder: B) -> MutexLtsBuilder<B> {
        MutexLtsBuilder {
            inner: Mutex::new(builder),
        }
    }

    /// Unwraps and returns the inner builder.
    pub fn into_inner(self) -> B {
        self.inner.into_inner().expect("MutexLtsBuilder mutex poisoned")
    }
}

impl<L: TransitionLabel, B: LtsBuilder<L>> LtsBuilder<L> for MutexLtsBuilder<B> {
    type LTS = B::LTS;

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        // We hold `&mut self`, so the builder is exclusively ours and needs no lock.
        self.inner
            .get_mut()
            .expect("MutexLtsBuilder mutex poisoned")
            .add_transition(from, label, to)
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        self.inner
            .get_mut()
            .expect("MutexLtsBuilder mutex poisoned")
            .finish(initial_state)
    }

    fn num_of_transitions(&self) -> usize {
        self.inner
            .lock()
            .expect("MutexLtsBuilder mutex poisoned")
            .num_of_transitions()
    }

    fn num_of_states(&self) -> usize {
        self.inner
            .lock()
            .expect("MutexLtsBuilder mutex poisoned")
            .num_of_states()
    }

    fn require_num_of_states(&mut self, num_states: usize) {
        self.inner
            .get_mut()
            .expect("MutexLtsBuilder mutex poisoned")
            .require_num_of_states(num_states)
    }
}

impl<L: TransitionLabel, B: LtsBuilder<L> + Send> ConcurrentLtsBuilder<L> for MutexLtsBuilder<B> {
    fn add_transition_shared<Q>(&self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        self.inner
            .lock()
            .expect("MutexLtsBuilder mutex poisoned")
            .add_transition(from, label, to)
    }
}

/// A label as numbered by an [`LtsBuilderMem`], which only that builder hands out.
///
/// The builder numbers labels itself, since the hidden action is always label zero and hidden
/// labels map onto it, so indices into the caller's own labels would be wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuilderIndex(LabelIndex);

/// This struct helps in building a labelled transition system by accumulating
/// transitions in a memory efficient way.
///
/// # Details
///
/// Transitions can be added with `add_transition`, and once all transitions
/// have been added, the labelled transition system can be constructed with
/// `finish`. An initial state can also be specified during finalization.
pub struct LtsBuilderMem<L> {
    transition_from: ByteCompressedVec<StateIndex>,
    transition_labels: ByteCompressedVec<LabelIndex>,
    transition_to: ByteCompressedVec<StateIndex>,

    // This is used to keep track of the label to index mapping.
    labels_index: FxHashMap<L, LabelIndex>,
    labels: Vec<L>,

    /// The hidden labels that should be mapped to the hidden action.
    hidden_labels: Vec<String>,

    /// The number of states (derived from the transitions).
    num_of_states: usize,
}

impl<L: TransitionLabel> LtsBuilderMem<L> {
    /// Returns the label of this builder for `label`, which is added if it is new. Hidden labels
    /// map to the hidden action.
    pub fn label_index<Q>(&mut self, label: &Q) -> BuilderIndex
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        if let Some(&index) = self.labels_index.get(label) {
            return BuilderIndex(index);
        }

        let label = label.to_owned();
        let index = if self.hidden_labels.iter().any(|l| label.matches_label(l)) {
            LabelIndex::new(0)
        } else {
            let index = LabelIndex::new(self.labels.len());
            self.labels.push(label.clone());
            index
        };
        self.labels_index.insert(label, index);
        BuilderIndex(index)
    }

    /// Returns the labels of this builder for `labels`, in the same order, so that the transitions
    /// of an existing LTS can be added by their label index without looking up the label each time.
    pub fn label_indices(&mut self, labels: &[L]) -> Vec<BuilderIndex> {
        labels.iter().map(|label| self.label_index(label)).collect()
    }

    /// Adds a transition with a label that [`Self::label_index`] returned, which avoids looking up
    /// the label for every transition.
    pub fn add_transition_index(&mut self, from: StateIndex, label: BuilderIndex, to: StateIndex) {
        debug_assert!(
            label.0.value() < self.labels.len(),
            "The label belongs to another builder"
        );

        self.transition_from.push(from);
        self.transition_labels.push(label.0);
        self.transition_to.push(to);

        self.num_of_states = self.num_of_states.max(from.value() + 1).max(to.value() + 1);
    }

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
        let mut labels_index = FxHashMap::default();
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
            hidden_labels,
            num_of_states: 0,
        }
    }

    /// Returns an iterator over all transitions as (from, label, to) tuples.
    fn iter(&self) -> impl Iterator<Item = (StateIndex, LabelIndex, StateIndex)> {
        self.transition_from
            .iter()
            .zip(self.transition_labels.iter())
            .zip(self.transition_to.iter())
            .map(|((from, label), to)| (from, label, to))
    }

    /// Returns the approximate number of bytes used to store the accumulated
    /// transitions.
    pub fn memory_usage(&self) -> usize {
        self.transition_from.metrics().actual_memory
            + self.transition_labels.metrics().actual_memory
            + self.transition_to.metrics().actual_memory
    }

    /// Finalizes the builder and returns the constructed labelled transition system.
    ///
    /// If `remove_duplicates` is true, duplicate `(from, label, to)` transitions are
    /// removed first.
    pub fn finish(&mut self, initial_state: StateIndex, remove_duplicates: bool) -> LabelledTransitionSystem<L> {
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

    /// Removes duplicate `(from, label, to)` transitions in place, deduplicating via a
    /// bucket (counting) sort on `from` followed by [`merc_collections::dedup_grouped`]
    /// (see its docs for the within-bucket algorithm).
    ///
    /// The counting sort itself goes through [`merc_collections::scatter_into_buckets`],
    /// which scatters `(label, to)` directly into the (byte-compressed) grouped columns via
    /// a write callback, rather than building an explicit `position -> original_index`
    /// permutation for the caller to gather through: for a large LTS, a full-width `usize`
    /// permutation (8 bytes per transition, regardless of how few bits
    /// `StateIndex`/`LabelIndex` actually need) would be the dominant term in this
    /// function's peak memory footprint, on top of the old and new transition columns it
    /// would sit alongside; scattering directly needs only the returned `bucket_ends`, of
    /// length `num_states` -- typically far smaller than `num_transitions`.
    fn remove_duplicates(&mut self) {
        let num_states = self.num_of_states;
        let num_transitions = self.transition_from.len();

        if num_transitions == 0 {
            return;
        }

        // Sized for their final byte width up front.
        let mut grouped_labels = ByteCompressedVec::with_capacity(num_transitions, self.labels.len().bytes_required());
        grouped_labels.resize_zeroed(num_transitions, self.labels.len().bytes_required());
        let mut grouped_to = ByteCompressedVec::with_capacity(num_transitions, num_states.bytes_required());
        grouped_to.resize_zeroed(num_transitions, num_states.bytes_required());
        let bucket_ends = merc_collections::scatter_into_buckets(
            num_states,
            num_transitions,
            |i| self.transition_from.index(i).value(),
            |position, i| {
                grouped_labels.set(position, self.transition_labels.index(i));
                grouped_to.set(position, self.transition_to.index(i));
            },
        );

        // Free the old columns now that they're copied into `grouped_labels`/`grouped_to`,
        // rather than holding both alive at once during the compaction below.
        self.transition_from = ByteCompressedVec::new();
        self.transition_labels = ByteCompressedVec::new();
        self.transition_to = ByteCompressedVec::new();

        // `key_of` (reading) and the `Keep` arm (writing, to compact in place) both
        // need access to `grouped_labels`/`grouped_to`, so they're wrapped in a
        // `RefCell`: the two never actually borrow at the same time, just at a point
        // the borrow checker can't see through two separate closures.
        let mut new_transition_from = ByteCompressedVec::with_capacity(num_transitions, num_states.bytes_required());
        let mut write = 0usize;
        let grouped_labels = RefCell::new(grouped_labels);
        let grouped_to = RefCell::new(grouped_to);

        merc_collections::dedup_grouped(
            &bucket_ends,
            |position| {
                (
                    grouped_labels.borrow().index(position),
                    grouped_to.borrow().index(position),
                )
            },
            |position, state, outcome| {
                if let merc_collections::DedupOutcome::Keep { .. } = outcome {
                    if write != position {
                        let label = grouped_labels.borrow().index(position);
                        let to = grouped_to.borrow().index(position);
                        grouped_labels.borrow_mut().set(write, label);
                        grouped_to.borrow_mut().set(write, to);
                    }
                    new_transition_from.push(StateIndex::new(state));
                    write += 1;
                }
            },
        );

        let mut grouped_labels = grouped_labels.into_inner();
        let mut grouped_to = grouped_to.into_inner();

        grouped_labels.resize_with(write, || unreachable!("compaction never grows the vector"));
        grouped_to.resize_with(write, || unreachable!("compaction never grows the vector"));

        // `resize_with` doesn't release excess capacity on its own, so shrink
        // explicitly to actually give the memory back.
        new_transition_from.shrink_to_fit();
        grouped_labels.shrink_to_fit();
        grouped_to.shrink_to_fit();

        self.transition_from = new_transition_from;
        self.transition_labels = grouped_labels;
        self.transition_to = grouped_to;
    }
}

impl<L: TransitionLabel> LtsBuilder<L> for LtsBuilderMem<L> {
    type LTS = LabelledTransitionSystem<L>;

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        let label_index = self.label_index(label);
        self.add_transition_index(from, label_index, to);
        Ok(())
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        Ok(self.finish(initial_state, false))
    }

    fn num_of_transitions(&self) -> usize {
        self.transition_from.len()
    }

    fn num_of_states(&self) -> usize {
        self.num_of_states
    }

    fn require_num_of_states(&mut self, num_of_states: usize) {
        if num_of_states > self.num_of_states {
            self.num_of_states = num_of_states;
        }
    }
}

impl<Label: TransitionLabel> fmt::Debug for LtsBuilderMem<Label> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Transitions:")?;
        for (from, label, to) in self.iter() {
            writeln!(f, "    {:?} --[{:?}]-> {:?}", from, label, to)?;
        }
        Ok(())
    }
}
