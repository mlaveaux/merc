#![forbid(unsafe_code)]

use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use log::debug;
use log::info;
use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermRef;
use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermReader;
use merc_aterm::BinaryATermWriter;
use merc_aterm::GarbageCollectionPause;
use merc_aterm::Protected;
use merc_aterm::Symbol;
use merc_aterm::Term;
use merc_aterm::is_list_term;
use merc_data::Mcrl2DataSpecification;
use merc_io::LargeFormatter;
use merc_io::TimeProgress;
use merc_utilities::MercError;
use rustc_hash::FxHashMap;

use crate::BuilderIndex;
use crate::LTS;
use crate::LabelledTransitionSystem;
use crate::LtsAction;
use crate::LtsBuilder;
use crate::LtsBuilderMem;
use crate::LtsMultiAction;
use crate::StateIndex;
use crate::StateLabels;

/// The state labels of the mCRL2 `.lts` format, which are lists of state vectors.
///
/// The terms are kept in a single [Protected] container instead of one [ATerm]
/// per state, since every protected term costs an entry in the protection set.
#[derive(Default, PartialEq, Eq)]
pub struct LtsStateLabels {
    labels: Protected<Vec<ATermRef<'static>>>,
}

impl Clone for LtsStateLabels {
    fn clone(&self) -> Self {
        let source = self.labels.read();
        let mut labels: Protected<Vec<ATermRef<'static>>> = Protected::new(Vec::with_capacity(source.len()));
        labels.write(|labels| labels.extend(source.iter().map(|label| label.copy())));
        LtsStateLabels { labels }
    }
}

impl StateLabels for LtsStateLabels {
    type Label<'a> = ATermRef<'a>;

    fn get(&self, state_index: StateIndex) -> Option<ATermRef<'_>> {
        self.labels.read().get(state_index.value()).map(|label| label.copy())
    }

    fn len(&self) -> usize {
        self.labels.read().len()
    }
}

/// A state label that can be written by [write_lts], which allows writing an
/// LTS without state labels, such as the result of a reduction.
pub trait LtsStateLabel {
    /// Returns the term written for this state label, or `None` to write no state labels.
    fn to_lts_term(&self) -> Option<ATerm>;
}

impl LtsStateLabel for () {
    fn to_lts_term(&self) -> Option<ATerm> {
        None
    }
}

impl LtsStateLabel for ATermRef<'_> {
    fn to_lts_term(&self) -> Option<ATerm> {
        Some(self.protect())
    }
}

/// Loads a labelled transition system from the binary 'lts' format of the mCRL2 toolset,
/// together with the data specification that its action arguments are typed over. The
/// state labels are only kept if `read_state_labels` is true.
///
/// # Errors
///
/// Returns an error if the stream does not start with the LTS marker, contains a
/// probabilistic transition (not supported), is missing its initial state, has state
/// labels for only some of the states, or is otherwise malformed.
pub fn read_lts<R: Read>(
    reader: R,
    read_state_labels: bool,
) -> Result<
    (
        LabelledTransitionSystem<LtsMultiAction<LtsAction>, LtsStateLabels>,
        Mcrl2DataSpecification,
    ),
    MercError,
> {
    info!("Reading LTS in .lts format...");

    // Every term read stays in use until the LTS is built, so collecting garbage would be wasted.
    let _pause = GarbageCollectionPause::new();
    let mut reader = BinaryATermReader::new(BufReader::new(reader))?;

    if reader.read_aterm()? != Some(lts_marker()) {
        return Err("Stream does not contain a labelled transition system (LTS).".into());
    }

    // Read the data specification, parameters, and actions.
    let data_spec = Mcrl2DataSpecification::read(&mut reader)?;
    let _parameters = reader.read_aterm()?;
    let _actions = reader.read_aterm()?;

    // Labels are maximally shared terms, so looking up the term finds the label index without
    // translating or hashing the multi-action again.
    let mut label_indices: FxHashMap<ATerm, BuilderIndex> = FxHashMap::default();

    // The initial state is not known yet.
    let mut initial_state: Option<StateIndex> = None;
    let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());

    let progress = TimeProgress::new(
        |num_of_transitions| {
            info!("Read {num_of_transitions} transitions...");
        },
        1,
    );

    let mut state_labels = LtsStateLabels::default();

    // Compared against every term read, so created once.
    let transition_marker = transition_marker();
    let probabilistic_transition_mark = probabilistic_transition_mark();
    let initial_state_marker = initial_state_marker();

    loop {
        let term = reader.read_aterm()?;
        match term {
            Some(t) => {
                if t == transition_marker {
                    let from = StateIndex::new(reader.read_aterm_int()?);
                    let label = reader.read_aterm()?.ok_or("Missing transition label")?;
                    let to = StateIndex::new(reader.read_aterm_int()?);

                    let label_index = match label_indices.get(&label) {
                        Some(&index) => index,
                        None => {
                            let index = builder.label_index(&LtsMultiAction::from_mcrl2_aterm(label.clone())?);
                            label_indices.insert(label, index);
                            index
                        }
                    };
                    builder.add_transition_index(from, label_index, to);

                    // Reading the clock costs more than reading a transition.
                    if builder.num_of_transitions() % PROGRESS_INTERVAL == 0 {
                        progress.print(builder.num_of_transitions());
                    }
                } else if t == probabilistic_transition_mark {
                    return Err("Probabilistic transitions are not supported yet.".into());
                } else if is_list_term(&t) {
                    if read_state_labels {
                        debug!("Read state label: {t:?}");
                        state_labels.labels.write(|labels| labels.push(t.copy()));
                    }
                } else if t == initial_state_marker {
                    let length = reader.read_aterm_int()?;
                    if length != 1 {
                        return Err("Initial state length greater than 1 is not supported.".into());
                    }

                    initial_state = Some(StateIndex::new(reader.read_aterm_int()?));
                    debug!("Initial state: {:?}", initial_state);
                } else {
                    return Err(format!("Unexpected term in LTS stream: {}", t).into());
                }
            }
            None => break, // The default constructed term indicates the end of the stream.
        }
    }
    info!("Finished reading LTS.");

    // Labelled states without transitions must still exist.
    builder.require_num_of_states(state_labels.len());

    // Untrusted input can always contain duplicates.
    let lts = builder.finish(initial_state.ok_or("Missing initial state")?, true);
    if !state_labels.is_empty() && state_labels.len() != lts.num_of_states() {
        return Err(format!(
            "Found {} state labels for {} states.",
            state_labels.len(),
            lts.num_of_states()
        )
        .into());
    }

    Ok((lts.with_state_labels(state_labels), data_spec))
}

/// Write a labelled transition system in binary 'lts' format to the given
/// writer. Requires that the labels are ATerm streamable. Note that the writer
/// is buffered internally using a `BufWriter`.
///
/// # Errors
///
/// Returns an error if writing to `writer` fails, or if a label cannot be
/// converted to its mCRL2 ATerm representation.
///
/// # Details
///
/// This format is built on top the ATerm binary format. The structure is as
/// follows:
///
/// ```plain
///     lts_marker: ATerm
///     data_spec: see [`merc_data::Mcrl2DataSpecification::write`]
///     parameters: ATermList
///     action_labels: ATermList
/// ```
///
/// Afterwards we can write the following elements in any order:
///
/// ```plain
/// initial state:
///    initial_state_marker: ATerm
///    state: ATermInt
///
/// transition:
///     transition_marker: ATerm
///     from: ATermInt
///     label: ATerm (the multi_action)
///     to: ATermInt
/// ```
///
/// state_label (index derived from order of appearance):
///    `state_label: ATermList::<DataExpression>`
pub fn write_lts<'a, L, W>(writer: &mut W, lts: &'a L, data_spec: &Mcrl2DataSpecification) -> Result<(), MercError>
where
    L: LTS<Label = LtsMultiAction<LtsAction>>,
    L::StateLabel<'a>: LtsStateLabel,
    W: Write,
{
    info!("Writing LTS in .lts format...");

    let mut writer = BinaryATermWriter::new(BufWriter::new(writer))?;

    writer.write_aterm(&lts_marker())?;

    // Write the data specification, parameters, and actions.
    data_spec.write(&mut writer)?;
    writer.write_aterm(&ATermList::<ATerm>::empty().into())?; // Empty parameters
    writer.write_aterm(&ATermList::<ATerm>::empty().into())?; // Empty action labels

    // Convert the internal multi-actions to the ATerm representation that mCRL2 expects.
    let label_terms = lts
        .labels()
        .iter()
        .map(|label| label.to_mcrl2_aterm())
        .collect::<Result<Vec<ATerm>, MercError>>()?;

    // Write the initial state.
    writer.write_aterm(&initial_state_marker())?;
    writer.write_aterm_int(1)?; // Length of initial state is 1.
    writer.write_aterm_int(*lts.initial_state_index())?;

    for state_label in lts.state_labels() {
        if let Some(term) = state_label.to_lts_term() {
            writer.write_aterm(&term)?;
        }
    }

    let num_of_transitions = lts.num_of_transitions();
    let progress = TimeProgress::new(
        move |written: usize| {
            info!(
                "Wrote {} transitions ({}%)...",
                LargeFormatter(written),
                written * 100 / num_of_transitions
            );
        },
        1,
    );

    let transition_marker = transition_marker();
    let mut written = 0;
    for state in lts.iter_states() {
        for transition in lts.outgoing_transitions(state) {
            writer.write_aterm(&transition_marker)?;
            writer.write_aterm_int(*state)?;
            writer.write_aterm(&label_terms[transition.label.value()])?;
            writer.write_aterm_int(*transition.to)?;

            if written % PROGRESS_INTERVAL == 0 {
                progress.print(written);
            }
            written += 1;
        }
    }

    info!("Finished writing LTS.");
    Ok(())
}

/// The number of transitions between checks whether progress should be printed.
const PROGRESS_INTERVAL: usize = 4096;

/// Returns the ATerm marker for a labelled transition system.
pub(crate) fn lts_marker() -> ATerm {
    ATerm::constant(&Symbol::new("labelled_transition_system", 0))
}

/// Returns the ATerm marker for a transition.
pub(crate) fn transition_marker() -> ATerm {
    ATerm::constant(&Symbol::new("transition", 0))
}

/// Returns the ATerm marker for the initial state.
pub(crate) fn initial_state_marker() -> ATerm {
    ATerm::constant(&Symbol::new("initial_state", 0))
}

/// Returns the ATerm marker for the probabilistic transition.
fn probabilistic_transition_mark() -> ATerm {
    ATerm::constant(&Symbol::new("probabilistic_transition", 0))
}

#[cfg(test)]
mod tests {
    use merc_aterm::ATerm;
    use merc_aterm::ATermInt;
    use merc_aterm::ATermList;
    use merc_aterm::ATermStreamable;
    use merc_aterm::ATermWrite;
    use merc_aterm::BinaryATermWriter;
    use merc_data::Mcrl2DataSpecification;

    use super::initial_state_marker;
    use super::lts_marker;
    use super::transition_marker;
    use crate::LTS;
    use crate::LtsAction;
    use crate::LtsMultiAction;
    use crate::TransitionLabel;
    use crate::read_lts;

    /// Writes an LTS header followed by `body`, as a stream that `read_lts` accepts up to `body`.
    fn lts_stream(body: &[ATerm]) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::new();
        let mut writer = BinaryATermWriter::new(&mut buffer).unwrap();
        writer.write_aterm(&lts_marker()).unwrap();
        Mcrl2DataSpecification::default().write(&mut writer).unwrap();
        writer.write_aterm(&ATermList::<ATerm>::empty().into()).unwrap();
        writer.write_aterm(&ATermList::<ATerm>::empty().into()).unwrap();
        for term in body {
            writer.write_aterm(term).unwrap();
        }
        ATermWrite::flush(&mut writer).unwrap();
        drop(writer);
        buffer
    }

    #[test]
    fn test_read_lts_rejects_malformed_transitions() {
        let label = LtsMultiAction::<LtsAction>::tau_label().to_mcrl2_aterm().unwrap();
        let not_an_int = ATerm::from_string("f(a)").unwrap();

        // A non-integer state is an error, not a panic or a misread state.
        let stream = lts_stream(&[
            transition_marker(),
            not_an_int.clone(),
            label.clone(),
            ATermInt::new(0).into(),
        ]);
        assert!(read_lts(&stream[..], false).is_err());

        let stream = lts_stream(&[initial_state_marker(), ATermInt::new(1).into(), not_an_int]);
        assert!(read_lts(&stream[..], false).is_err());

        // A transition cut off by the end of the stream.
        let stream = lts_stream(&[transition_marker(), ATermInt::new(0).into()]);
        assert!(read_lts(&stream[..], false).is_err());

        // Integers written as terms (as opposed to by write_aterm_int) are still accepted.
        let stream = lts_stream(&[
            initial_state_marker(),
            ATermInt::new(1).into(),
            ATermInt::new(0).into(),
            transition_marker(),
            ATermInt::new(0).into(),
            label,
            ATermInt::new(1).into(),
        ]);
        let (lts, _data_spec) = read_lts(&stream[..], false).unwrap();
        assert_eq!(lts.num_of_transitions(), 1);
        assert_eq!(*lts.initial_state_index(), 0);
    }
}
