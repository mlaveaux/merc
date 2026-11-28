#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::hash::Hash;
use std::io::BufWriter;
use std::io::Write;

use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermWriter;
use merc_data::Mcrl2DataSpecification;
use merc_utilities::MercError;

use crate::LtsAction;
use crate::LtsBuilder;
use crate::LtsMultiAction;
use crate::StateIndex;
use crate::io_lts::initial_state_marker;
use crate::io_lts::lts_marker;
use crate::io_lts::transition_marker;

/// A stream writer for the mCRL2 binary `.lts` format, which allows writing an LTS to a file
/// without keeping the entire LTS in memory.
pub struct LtsStream<W: Write> {
    writer: BinaryATermWriter<BufWriter<W>>,

    /// Keep track of the number of transitions added.
    number_of_transitions: usize,

    /// Keep track of the number of states added.
    number_of_states: usize,
}

impl<W: Write> LtsStream<W> {
    /// Creates a new `.lts` stream writer, immediately writing the format's header: the marker,
    /// `data_spec`, and empty parameter/action-label lists (see [`crate::write_lts`], whose
    /// header this matches).
    ///
    /// Note that the writer is buffered internally using a `BufWriter`. Writing the header can
    /// fail, so this returns a [`Result`].
    pub fn new(writer: W, data_spec: &Mcrl2DataSpecification) -> Result<Self, MercError> {
        let mut writer = BinaryATermWriter::new(BufWriter::new(writer))?;

        writer.write_aterm(&lts_marker())?;
        data_spec.write(&mut writer)?;
        writer.write_aterm(&ATermList::<ATerm>::empty().into())?; // Empty parameters
        writer.write_aterm(&ATermList::<ATerm>::empty().into())?; // Empty action labels

        Ok(Self {
            writer,
            number_of_transitions: 0,
            number_of_states: 0,
        })
    }
}

impl<W: Write> LtsBuilder<LtsMultiAction<LtsAction>> for LtsStream<W> {
    type LTS = ();

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        LtsMultiAction<LtsAction>: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = LtsMultiAction<LtsAction>> + Eq + Hash,
    {
        self.number_of_transitions += 1;
        self.number_of_states = self.number_of_states.max(from.value() + 1).max(to.value() + 1);

        let label_term = label.to_owned().to_mcrl2_aterm()?;

        self.writer.write_aterm(&transition_marker())?;
        self.writer.write_aterm_int(*from)?;
        self.writer.write_aterm(&label_term)?;
        self.writer.write_aterm_int(*to)?;
        Ok(())
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        self.writer.write_aterm(&initial_state_marker())?;
        self.writer.write_aterm_int(1)?; // Length of initial state is 1.
        self.writer.write_aterm_int(*initial_state)?;

        // Flushes and writes the end-of-stream marker.
        ATermWrite::flush(&mut self.writer)
    }

    /// Returns the number of transitions added to the builder.
    fn num_of_transitions(&self) -> usize {
        self.number_of_transitions
    }

    /// Returns the number of states added to the builder.
    fn num_of_states(&self) -> usize {
        self.number_of_states
    }

    /// Sets the number of states to at least the given number. All states
    /// without transitions simply become deadlock states.
    fn require_num_of_states(&mut self, num_states: usize) {
        if num_states > self.number_of_states {
            self.number_of_states = num_states;
        }
    }
}
