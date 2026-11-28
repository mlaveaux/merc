#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::hash::Hash;

use merc_collections::IndexedSet;
use merc_collections::SetIndex;
use merc_utilities::MercError;

use crate::StateIndex;
use crate::TransitionLabel;

/// One state's outgoing transitions, being accumulated so they can be
/// deduplicated before being forwarded on.
///
/// # Details
///
/// Can be used in state based exploration algorithms, that only need to dedup
/// outgoing transitions of one state at the time. This is much cheaper than a
/// full deduplication afterwards.
pub struct PerStateDedup<L: TransitionLabel> {
    current_from: Option<StateIndex>,

    /// Interns every distinct label seen so far behind a small integer.
    /// Avoiding cloning the labels.
    labels: IndexedSet<L>,

    // Uses a small buffer rather than HashSet to preserve the order.
    seen: Vec<(SetIndex, StateIndex)>,
}

impl<L: TransitionLabel> Default for PerStateDedup<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: TransitionLabel> PerStateDedup<L> {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self {
            current_from: None,
            labels: IndexedSet::new(),
            seen: Vec::new(),
        }
    }

    /// Buffers `(label, to)` under `from`. If `from` differs from whichever state is currently
    /// buffered, that state's buffered transitions are flushed (deduplicated) through
    /// `flush_one` first.
    ///
    /// # Requirements
    ///
    /// Every transition for a given `from` state must be added consecutively; if transitions for
    /// two different states are interleaved, only duplicates *within* each contiguous run are
    /// caught.
    pub fn add<Q, F>(&mut self, from: StateIndex, label: &Q, to: StateIndex, flush_one: F) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
        F: FnMut(StateIndex, &L, StateIndex) -> Result<(), MercError>,
    {
        if self.current_from != Some(from) {
            self.flush(flush_one)?;
            self.current_from = Some(from);
        }

        // Looks the label up in the cache first, so a label already seen -- the common case --
        // never pays for `to_owned`; only a genuinely new label is cloned into the cache.
        let label_index = match self.labels.index(label) {
            Some(index) => index,
            None => self.labels.insert(label.to_owned()).0,
        };

        // Compares against the interned `label_index`/`to` first, so a duplicate -- the common
        // case for a label seen again from the same state -- is a cheap index comparison rather
        // than comparing `L` itself.
        let already_seen = self
            .seen
            .iter()
            .any(|&(seen_label, seen_to)| seen_to == to && seen_label == label_index);
        if !already_seen {
            self.seen.push((label_index, to));
        }
        Ok(())
    }

    /// Flushes any buffered transitions through `flush_one`, deduplicated, in the order they were
    /// first added.
    pub fn flush<F>(&mut self, mut flush_one: F) -> Result<(), MercError>
    where
        F: FnMut(StateIndex, &L, StateIndex) -> Result<(), MercError>,
    {
        if let Some(from) = self.current_from.take() {
            for (label_index, to) in self.seen.drain(..) {
                let label = self
                    .labels
                    .get(label_index)
                    .expect("label_index was returned by the label cache itself");
                flush_one(from, label, to)?;
            }
        }
        Ok(())
    }

    /// Returns the number of transitions currently buffered for the in-progress state.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Returns true iff nothing is currently buffered.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}
