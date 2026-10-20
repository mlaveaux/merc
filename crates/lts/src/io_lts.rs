#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use log::debug;
use log::info;
use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermReader;
use merc_aterm::BinaryATermWriter;
use merc_aterm::GarbageCollectionPause;
use merc_aterm::Symbol;
use merc_aterm::is_list_term;
use merc_data::Mcrl2DataSpecification;
use merc_io::LargeFormatter;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::LTS;
use crate::LabelledTransitionSystem;
use crate::LtsAction;
use crate::LtsBuilder;
use crate::LtsBuilderMem;
use crate::LtsMultiAction;
use crate::StateIndex;

/// Loads a labelled transition system from the binary 'lts' format of the mCRL2 toolset,
/// together with the data specification that its action arguments are typed over.
///
/// # Errors
///
/// Returns an error if the stream does not start with the LTS marker, contains a
/// probabilistic transition (not supported), is missing its initial state, or is
/// otherwise malformed.
pub fn read_lts<R: Read>(
    reader: R,
    read_state_labels: bool,
) -> Result<
    (
        LabelledTransitionSystem<LtsMultiAction<LtsAction>>,
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

    // Use a cache to avoid translating the same multi-action multiple times.
    let mut multi_actions: HashMap<ATerm, LtsMultiAction<LtsAction>> = HashMap::new();

    // The initial state is not known yet.
    let mut initial_state: Option<StateIndex> = None;
    let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());

    let progress = TimeProgress::new(
        |num_of_transitions| {
            info!("Read {num_of_transitions} transitions...");
        },
        1,
    );

    let mut state_labels = Vec::new();

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

                    if let Some(multi_action) = multi_actions.get(&label) {
                        // Multi-action already exists in the cache.
                        builder.add_transition(from, multi_action, to)?;
                    } else {
                        // New multi-action found, add it to the builder.
                        let multi_action = LtsMultiAction::from_mcrl2_aterm(label.clone())?;
                        multi_actions.insert(label, multi_action.clone());
                        builder.add_transition(from, &multi_action, to)?;
                    }

                    // Reading the clock costs more than reading a transition.
                    if builder.num_of_transitions() % PROGRESS_INTERVAL == 0 {
                        progress.print(builder.num_of_transitions());
                    }
                } else if t == probabilistic_transition_mark {
                    return Err("Probabilistic transitions are not supported yet.".into());
                } else if is_list_term(&t) {
                    if read_state_labels {
                        let t: ATermList<ATerm> = t.into();
                        debug!("Read state label: {:?}", t.to_vec());
                        state_labels.push(t);
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

    Ok((
        // Untrusted input can always contain duplicates.
        builder.finish(initial_state.ok_or("Missing initial state")?, true),
        data_spec,
    ))
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
pub fn write_lts<L, W>(writer: &mut W, lts: &L, data_spec: &Mcrl2DataSpecification) -> Result<(), MercError>
where
    L: LTS<Label = LtsMultiAction<LtsAction>>,
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
    use merc_utilities::random_test;

    use super::initial_state_marker;
    use super::lts_marker;
    use super::transition_marker;
    use super::write_lts;
    use crate::LTS;
    use crate::LtsAction;
    use crate::LtsMultiAction;
    use crate::TransitionLabel;
    use crate::random_lts;
    use crate::read_lts;

    #[test]
    #[cfg_attr(miri, ignore)] // Tests are too slow under miri.
    fn test_read_lts() {
        let (lts, _data_spec) = read_lts(include_bytes!("../../../examples/lts/abp.lts").as_ref(), true).unwrap();

        assert_eq!(lts.num_of_states(), 74);
        assert_eq!(lts.num_of_transitions(), 92);
        assert_eq!(*lts.initial_state_index(), 0);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_lts_io() {
        random_test(100, |rng| {
            let lts = random_lts::<LtsMultiAction<LtsAction>, _>(rng, 1000, 3);

            let mut buffer: Vec<u8> = Vec::new();
            write_lts(&mut buffer, &lts, &Mcrl2DataSpecification::default()).unwrap();

            let (result_lts, _data_spec) = read_lts(&buffer[0..], false).unwrap();

            crate::check_equivalent(&lts, &result_lts);
        })
    }

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
