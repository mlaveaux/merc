//!
//!  write_lts_header(data_spec, parameters, action_labels)
//!
//! In any order:
//!  Write transitions (to, label, from), where 'to' and 'from' are indices and 'label' the multi_action, as necessary.
//!  Write state labels (state_label_lts) in their order such that writing the i-th state label belongs to state with index i.
//!  Write the initial state.

use std::io::BufReader;
use std::io::Read;

use log::info;
use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::BinaryATermReader;
use merc_aterm::Symbol;
use merc_aterm::is_list_term;
use merc_data::DataSpecification;
use merc_io::TimeProgress;
use merc_utilities::IndexedSet;
use merc_utilities::MercError;

use crate::LabelledTransitionSystem;
use crate::LtsBuilder;
use crate::StateIndex;

/// Loads a labelled transition system from the binary 'lts' format of the mCRL2 toolset.
pub fn read_lts(reader: impl Read, hidden_labels: Vec<String>) -> Result<LabelledTransitionSystem, MercError> {
    info!("Reading LTS in .lts format...");

    let mut reader = BinaryATermReader::new(BufReader::new(reader))?;

    if reader.read_aterm()? != Some(lts_marker()) {
        return Err("Stream does not contain a labelled transition system (LTS).".into());
    }

    // Read the data specification, parameters, and actions.
    let _data_spec = DataSpecification::read(&mut reader)?;
    let _parameters = reader.read_aterm()?;
    let _actions = reader.read_aterm()?;

    // An indexed set to keep track of indices for multi-actions
    let _multi_actions: IndexedSet<ATerm> = IndexedSet::new();

    // The initial state is not known yet.
    let mut initial_state: Option<StateIndex> = None;    
    let mut builder = LtsBuilder::new(Vec::new(), hidden_labels);

    let mut progress = TimeProgress::new(
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


                    builder.add_transition(
                        StateIndex::new(from.value()),
                        &label.to_string(),
                        StateIndex::new(to.value()),
                    );

                    progress.print(builder.num_of_transitions());
                } else if t == probabilistic_transition_mark() {
                    unimplemented!("Probabilistic transitions are not supported yet.");
                } else if is_list_term(&t) {
                    // State labels can be ignored for the reduction algorithm.
                } else if t == initial_state_marker() {
                    initial_state = Some(StateIndex::new(
                        ATermInt::from(reader.read_aterm()?.ok_or("Missing initial state")?).value(),
                    ));
                }
            }
            None => break, // The default constructed term indicates the end of the stream.
        }
    }
    info!("Finished reading LTS");

    Ok(builder.finish(initial_state.ok_or("Missing initial state")?, false))
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

/// A multi-action, i.e., a set of action labels.
// struct MultiAction {
//     actions: Vec<LabelIndex>,
// }

// struct Action {
//     name: String,
//     sort: SortExpr,
// }

#[cfg(test)]
mod tests {
    use super::*;

    use crate::LTS;

    #[test]
    fn test_read_lts() {
        let lts = read_lts(include_bytes!("../../../examples/lts/abp.lts").as_ref(), vec![]).unwrap();

        assert_eq!(lts.num_of_states(), 74);
        assert_eq!(lts.num_of_transitions(), 92);
    }
}
