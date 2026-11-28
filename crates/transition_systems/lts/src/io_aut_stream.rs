#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::f64::consts::LOG10_2;
use std::hash::Hash;
use std::io::BufWriter;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::marker::PhantomData;

use merc_utilities::MercError;

use crate::AUT_TAU_LABEL;
use crate::LtsBuilder;
use crate::MCRL2_TAU_LABEL;
use crate::StateIndex;
use crate::TransitionLabel;

/// A stream writer for the AUT format, which allows writing an LTS to a file
/// without keeping the entire LTS in memory. The header of the AUT file is
/// written at the end, after all transitions have been added, to ensure that
/// the correct number of states and transitions is included in the header.
pub struct AutStream<W: Write, L> {
    writer: BufWriter<W>,

    /// The dialect to use when writing tau labels.
    format: AutFormat,

    /// Keep track of the number of transitions added.
    number_of_transitions: usize,

    /// Keep track of the number of states added.
    number_of_states: usize,

    /// Number of bytes reserved for the header line (including its trailing
    /// newline). The real header is padded to exactly this width in
    /// [`AutStream::finish`] so the placeholder is fully overwritten.
    header_len: usize,

    /// Records the label type without affecting the auto traits of the stream:
    /// the labels themselves are never stored, so an `AutStream` stays `Send`
    /// and `Sync` (so it can be wrapped in a [`crate::MutexLtsBuilder`] and shared
    /// across worker threads) even when `L` is neither — for example mCRL2
    /// multi-action labels backed by a thread-local `ATerm`.
    _marker: PhantomData<fn() -> L>,
}

impl<W: Write, L> AutStream<W, L> {
    /// Creates a new AUT stream writer in the standard Aldebaran format
    /// (`i` is used as the tau label).
    ///
    /// Note that the writer is buffered internally using a `BufWriter`. Writing
    /// the placeholder header can fail, so this returns a [`Result`].
    pub fn new(writer: W) -> Result<Self, MercError> {
        Self::with_format(writer, AutFormat::Aut)
    }

    /// Creates a new AUT stream writer in the mCRL2 dialect
    /// (`tau` is used as the tau label).
    pub fn new_mcrl2(writer: W) -> Result<Self, MercError> {
        Self::with_format(writer, AutFormat::AutMcrl2)
    }

    /// Creates a new AUT stream writer using the given format.
    pub fn with_format(writer: W, format: AutFormat) -> Result<Self, MercError> {
        let mut writer = BufWriter::new(writer);
        // Write a placeholder for the header, which will be filled in later.
        // Reserve enough space for the header using the number of bits of
        // usize. This avoids overwriting transition bytes when the final header
        // is longer.
        let max_usize_digits = (usize::BITS as f64 * LOG10_2).ceil() as usize;
        let header_len = format!("des ({0:<1$}, {0:<1$}, {0:<1$})\n", " ", max_usize_digits).len();
        writer.write_all(" ".repeat(header_len).as_bytes())?;

        Ok(Self {
            writer,
            format,
            number_of_transitions: 0,
            number_of_states: 0,
            header_len,
            _marker: PhantomData,
        })
    }
}

impl<W: Write + Seek, L: TransitionLabel> LtsBuilder<L> for AutStream<W, L> {
    type LTS = ();

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        L: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = L> + Eq + Hash,
    {
        self.number_of_transitions += 1;
        self.number_of_states = self.number_of_states.max(from.value() + 1).max(to.value() + 1);

        let owned = label.to_owned();
        if owned.is_tau_label() {
            writeln!(self.writer, "({}, \"{}\", {})", from, self.format.tau_label_str(), to)?;
        } else {
            writeln!(self.writer, "({}, \"{}\", {})", from, owned, to)?;
        }
        Ok(())
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        // Flush to ensure all buffered transitions are written
        self.writer.flush()?;

        // Seek to the start and overwrite the header. The real header is padded
        // with trailing spaces to fill the entire reserved region (its trailing
        // newline lands as the last reserved byte), so no leftover placeholder
        // spaces remain to form a spurious, invalid transition line on read —
        // which matters in particular when there are no transitions at all.
        self.writer.seek(SeekFrom::Start(0))?;
        let header = format!(
            "des ({}, {}, {})",
            initial_state, self.number_of_transitions, self.number_of_states
        );
        writeln!(self.writer, "{:<1$}", header, self.header_len - 1)?;

        // Flush the updated header
        self.writer.flush()?;
        Ok(())
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

/// The dialect of the AUT format, which controls the textual label used for
/// internal (tau) transitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum AutFormat {
    /// The standard Aldebaran format, which uses `i` for tau transitions.
    Aut,
    /// The mCRL2 dialect of the Aldebaran format, which uses `tau` for tau transitions.
    AutMcrl2,
}

impl AutFormat {
    /// Returns the string that represents the tau label in this format.
    fn tau_label_str(self) -> &'static str {
        match self {
            AutFormat::Aut => AUT_TAU_LABEL,
            AutFormat::AutMcrl2 => MCRL2_TAU_LABEL,
        }
    }
}
