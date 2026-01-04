#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use log::info;
use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermReader;
use merc_aterm::BinaryATermWriter;
use merc_aterm::Symbol;
use merc_aterm::is_list_term;
use merc_data::DataSpecification;
use merc_io::LargeFormatter;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::LTS;
use crate::LabelledTransitionSystem;
use crate::LtsBuilder;
use crate::MultiAction;
use crate::StateIndex;

/// Loads a labelled transition system from the binary 'lts' format of the mCRL2 toolset.
pub fn read_lts(
    reader: impl Read,
    hidden_labels: Vec<String>,
) -> Result<LabelledTransitionSystem<MultiAction>, MercError> {
    info!("Reading LTS in .lts format...");

    let mut reader = BinaryATermReader::new(BufReader::new(reader))?;

    if reader.read_aterm()? != Some(lts_marker()) {
        return Err("Stream does not contain a labelled transition system (LTS).".into());
    }

    // Read the data specification, parameters, and actions.
    let _data_spec = DataSpecification::read(&mut reader)?;
    let _parameters = reader.read_aterm()?;
    let _actions = reader.read_aterm()?;

    // Use a cache to avoid translating the same multi-action multiple times.
    let mut multi_actions: HashMap<ATerm, MultiAction> = HashMap::new();

    // The initial state is not known yet.
    let mut initial_state: Option<StateIndex> = None;
    let mut builder = LtsBuilder::new(Vec::new(), hidden_labels);

    let progress = TimeProgress::new(
        |num_of_transitions| {
            info!("Read {num_of_transitions} transitions...");
        },
        1,
    );

    loop {
        let term = reader.read_aterm()?;
        match term {
            Some(t) => {
                if t == transition_marker() {
                    let from: ATermInt = reader.read_aterm()?.ok_or("Missing from state")?.into();
                    let label = reader.read_aterm()?.ok_or("Missing transition label")?;
                    let to: ATermInt = reader.read_aterm()?.ok_or("Missing to state")?.into();

                    if let Some(multi_action) = multi_actions.get(&label) {
                        // Multi-action already exists in the cache.
                        builder.add_transition(
                            StateIndex::new(from.value()),
                            multi_action,
                            StateIndex::new(to.value()),
                        );
                    } else {
                        // New multi-action found, add it to the builder.
                        let multi_action = MultiAction::from_mcrl2_aterm(label.clone())?;
                        multi_actions.insert(label.clone(), multi_action.clone());
                        builder.add_transition(
                            StateIndex::new(from.value()),
                            &multi_action,
                            StateIndex::new(to.value()),
                        );
                    }

                    progress.print(builder.num_of_transitions());
                } else if t == probabilistic_transition_mark() {
                    return Err("Probabilistic transitions are not supported yet.".into());
                } else if is_list_term(&t) {
                    // State labels can be ignored for the reduction algorithm.
                } else if t == initial_state_marker() {
                    let length = ATermInt::from(reader.read_aterm()?.ok_or("Missing initial state length")?).value();
                    if length != 1 {
                        return Err("Initial state length greater than 1 is not supported.".into());
                    }

                    initial_state = Some(StateIndex::new(
                        ATermInt::from(reader.read_aterm()?.ok_or("Missing initial state index")?).value(),
                    ));
                    println!("Initial state: {:?}", initial_state);
                } else {
                    return Err(format!("Unexpected term in LTS stream: {}", t).into());
                }
            }
            None => break, // The default constructed term indicates the end of the stream.
        }
    }
    info!("Finished reading LTS.");

    Ok(builder.finish(initial_state.ok_or("Missing initial state")?))
}

/// Write a labelled transition system in binary 'lts' format to the given
/// writer. Requires that the labels are ATerm streamable. Note that the writer
/// is buffered internally using a `BufWriter`.
///
/// # Details
///
/// This format is built on top the ATerm binary format. The structure is as
/// follows:
///
/// ```plain
///     lts_marker: ATerm
///     data_spec: see [`merc_data::DataSpecification::write`]
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
///    state_label: ATermList::<DataExpression>
pub fn write_lts<L>(writer: &mut impl Write, lts: &L) -> Result<(), MercError>
where
    L: LTS<Label = MultiAction>,
{
    info!("Writing LTS in .lts format...");

    let mut writer = BinaryATermWriter::new(BufWriter::new(writer))?;

    writer.write_aterm(&lts_marker())?;

    // Write the data specification, parameters, and actions.
    DataSpecification::default().write(&mut writer)?;
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
    writer.write_aterm(&ATermInt::new(1))?; // Length of initial state is 1.
    writer.write_aterm(&ATermInt::new(*lts.initial_state_index()))?;

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

    let mut written = 0;
    for state in lts.iter_states() {
        for transition in lts.outgoing_transitions(state) {
            writer.write_aterm(&transition_marker())?;
            writer.write_aterm(&ATermInt::new(*state))?;
            writer.write_aterm(&label_terms[transition.label.value()])?;
            writer.write_aterm(&ATermInt::new(*transition.to))?;

            progress.print(written);
            written += 1;
        }
    }

    info!("Finished writing LTS.");
    Ok(())
}

/// Returns the ATerm marker for a labelled transition system.
fn lts_marker() -> ATerm {
    ATerm::constant(&Symbol::new("labelled_transition_system", 0))
}

/// Returns the ATerm marker for a transition.
fn transition_marker() -> ATerm {
    ATerm::constant(&Symbol::new("transition", 0))
}

/// Returns the ATerm marker for the initial state.
fn initial_state_marker() -> ATerm {
    ATerm::constant(&Symbol::new("initial_state", 0))
}

/// Returns the ATerm marker for the probabilistic transition.
fn probabilistic_transition_mark() -> ATerm {
    ATerm::constant(&Symbol::new("probabilistic_transition", 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use merc_utilities::random_test;

    use crate::LTS;
    use crate::random_lts_monolithic;

    #[test]
    #[cfg_attr(miri, ignore)] // Tests are too slow under miri.
    fn test_read_lts() {
        let lts = read_lts(include_bytes!("../../../examples/lts/abp.lts").as_ref(), vec![]).unwrap();

        assert_eq!(lts.num_of_states(), 74);
        assert_eq!(lts.num_of_transitions(), 92);
        assert_eq!(*lts.initial_state_index(), 0);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_lts_io() {
        random_test(100, |rng| {
            let lts = random_lts_monolithic::<MultiAction>(rng, 100, 3, 20);

            let mut buffer: Vec<u8> = Vec::new();
            write_lts(&mut buffer, &lts).unwrap();

            let result_lts = read_lts(&buffer[0..], vec![]).unwrap();

            crate::check_equivalent(&lts, &result_lts);
        })
    }
}
