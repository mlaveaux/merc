#![forbid(unsafe_code)]

use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use log::info;
use merc_io::LargeFormatter;
use regex::Regex;
use streaming_iterator::StreamingIterator;
use thiserror::Error;

use merc_io::LineIterator;
use merc_io::TimeProgress;
use merc_utilities::MercError;
use merc_utilities::debug_trace;

use crate::LTS;
use crate::LabelledTransitionSystem;
use crate::LtsBuilder;
use crate::LtsBuilderMem;
use crate::StateIndex;
use crate::TransitionLabel;

#[derive(Error, Debug)]
pub(crate) enum IOError {
    #[error("Invalid .aut header {0}")]
    InvalidHeader(&'static str),

    #[error("Invalid transition {0}")]
    InvalidTransition(String),
}

/// The label used for internal transitions in the mCRL2 format.
pub(crate) const MCRL2_TAU_LABEL: &str = "tau";

/// The label used for internal transitions in the Aldebaran format.
pub(crate) const AUT_TAU_LABEL: &str = "i";

/// Loads a labelled transition system in the [Aldebaran
/// format](https://cadp.inria.fr/man/aldebaran.html) from the given reader.
///
/// Note that the reader is buffered internally using a `BufReader`.
///
/// The format consists of a header line `des (<initial>: Nat,
/// <num_of_transitions>: Nat, <num_of_states>: Nat)` followed by one line per
/// transition, either `(<from>: Nat, "<label>": Str, <to>: Nat)` or
/// `(<from>: Nat, <label>: Str, <to>: Nat)`.
///
/// # Errors
///
/// Returns an error if the header or a transition line does not match this format.
pub fn read_aut<R: Read>(reader: R) -> Result<LabelledTransitionSystem<String>, MercError> {
    read_aut_impl(reader, AUT_TAU_LABEL)
}

/// The same as [read_aut], but with the `tau` as the internal transition, instead of `i`.
pub fn read_mcrl2_aut<R: Read>(reader: R) -> Result<LabelledTransitionSystem<String>, MercError> {
    read_aut_impl(reader, MCRL2_TAU_LABEL)
}

/// The implementation of [read_aut] and [read_mcrl2_aut].
///
/// `tau_label` specifies which label is considered the internal transition.
fn read_aut_impl<R: Read>(reader: R, tau_label: &str) -> Result<LabelledTransitionSystem<String>, MercError> {
    info!("Reading LTS in .aut format...");

    let mut lines = LineIterator::new(reader);
    lines.advance();
    let header = lines
        .get()
        .ok_or(IOError::InvalidHeader("The first line should be the header"))?;

    // Regex for des (<initial>: Nat, <num_of_transitions>: Nat, <num_of_states>: Nat)
    let header_regex = Regex::new(r#"des\s*\(\s*([0-9]*)\s*,\s*([0-9]*)\s*,\s*([0-9]*)\s*\)\s*"#)
        .expect("Regex compilation should not fail");

    let (_, [initial_txt, num_of_transitions_txt, num_of_states_txt]) = header_regex
        .captures(header)
        .ok_or(IOError::InvalidHeader(
            "does not match des (<init>, <num_of_transitions>, <num_of_states>)",
        ))?
        .extract();

    let initial_state = StateIndex::new(initial_txt.parse()?);
    let num_of_transitions: usize = num_of_transitions_txt.parse()?;
    let num_of_states: usize = num_of_states_txt.parse()?;

    let mut builder = LtsBuilderMem::with_capacity(
        Vec::new(),
        vec![tau_label.to_string()],
        16,
        num_of_states,
        num_of_transitions,
    );
    builder.require_num_of_states(num_of_states);

    let progress = TimeProgress::new(
        move |read: usize| {
            info!(
                "Read {} transitions {}%...",
                LargeFormatter(read),
                read * 100 / num_of_transitions.max(1)
            )
        },
        1,
    );

    while let Some(line) = lines.next() {
        let (from_txt, label_txt, to_txt) =
            read_transition(line).ok_or_else(|| IOError::InvalidTransition(line.clone()))?;

        // Parse the from and to states, with the given label.
        let from = StateIndex::new(from_txt.parse()?);
        let to = StateIndex::new(to_txt.parse()?);

        debug_trace!("Read transition {from} --[{label_txt}]-> {to}");
        builder.add_transition(from, label_txt, to)?;

        progress.print(builder.num_of_transitions());
    }

    info!("Finished reading LTS");

    // Untrusted input can always contain duplicates.
    Ok(builder.finish(initial_state, true))
}

/// Write a labelled transition system in plain text in Aldebaran format to the
/// given writer, see [read_aut].
///
/// Note that the writer is buffered internally using a `BufWriter`.
pub fn write_aut<W: Write, L: LTS>(writer: &mut W, lts: &L) -> Result<(), MercError> {
    write_aut_impl(writer, lts, AUT_TAU_LABEL)
}

/// The same as [write_aut], but with the `tau` as the internal transition, instead of `i`.
pub fn write_mcrl2_aut<W: Write, L: LTS>(writer: &mut W, lts: &L) -> Result<(), MercError> {
    write_aut_impl(writer, lts, MCRL2_TAU_LABEL)
}

/// The implementation of [write_aut] and [write_mcrl2_aut].
fn write_aut_impl<W: Write, L: LTS>(writer: &mut W, lts: &L, tau_label: &str) -> Result<(), MercError> {
    info!("Writing LTS in .aut format...");

    let mut writer = BufWriter::new(writer);
    writeln!(
        writer,
        "des ({}, {}, {})",
        lts.initial_state_index(),
        lts.num_of_transitions(),
        lts.num_of_states()
    )?;

    let num_of_transitions = lts.num_of_transitions();
    let progress = TimeProgress::new(
        move |written: usize| {
            info!(
                "Wrote {} transitions {}%...",
                LargeFormatter(written),
                written * 100 / num_of_transitions
            )
        },
        1,
    );
    let mut transitions_written = 0usize;
    for state_index in lts.iter_states() {
        for transition in lts.outgoing_transitions(state_index) {
            if lts.is_hidden_label(transition.label) {
                writeln!(writer, "({}, \"{}\", {})", state_index, tau_label, transition.to)?;
            } else {
                writeln!(
                    writer,
                    "({}, \"{}\", {})",
                    state_index,
                    lts.labels()[transition.label.value()],
                    transition.to
                )?;
            }

            progress.print(transitions_written);
            transitions_written += 1;
        }
    }

    info!("Finished writing LTS.");
    Ok(())
}

/// Parses a transition line of the form `(<from>: Nat, "<label>": Str, <to>: Nat)` or
/// `(<from>: Nat, <label>: Str, <to>: Nat)`, returning `None` if it does not match.
fn read_transition(input: &str) -> Option<(&str, &str, &str)> {
    let start_paren = input.find('(')?;
    let start_comma = input.find(',')?;

    // Find the comma in the second part
    let start_second_comma = input.rfind(',')?;
    let end_paren = input.rfind(')')?;

    let from = input.get(start_paren + 1..start_comma)?.trim();
    let label = input.get(start_comma + 1..start_second_comma)?.trim();
    let to = input.get(start_second_comma + 1..end_paren)?.trim();
    // Handle the special case where it has quotes.
    if label.starts_with('"') && label.ends_with('"') {
        return Some((from, &label[1..label.len() - 1], to));
    }

    Some((from, label, to))
}

/// A trait for labels that can be used in transitions.
impl TransitionLabel for String {
    fn is_tau_label(&self) -> bool {
        self == "τ"
    }

    fn tau_label() -> Self {
        "τ".to_string()
    }

    fn matches_label(&self, label: &str) -> bool {
        self == label
    }

    fn from_index(i: usize) -> Self {
        char::from_digit(i as u32 + 10, 36)
            .expect("Radix is less than 37, so should not panic")
            .to_string()
    }
}
