use std::io::Read;

use mcrl3_aterm::ATerm;
use mcrl3_aterm::ATermRead;
use mcrl3_aterm::ATermStreamable;
use mcrl3_aterm::BinaryATermReader;
use mcrl3_aterm::Symbol;
use mcrl3_data::DataSpecification;
use mcrl3_utilities::IndexedSet;
use mcrl3_utilities::MCRL3Error;

use crate::LabelledTransitionSystem;

/// Loads a labelled transition system from the binary 'lts' format of the mCRL2 toolset.
pub fn read_lts(reader: impl Read) -> Result<LabelledTransitionSystem, MCRL3Error> {
    let mut reader = BinaryATermReader::new(reader)?;

    if reader.read_aterm()? != Some(lts_marker()) {
        return Err("Stream does not contain a labelled transition system (LTS).".into());
    }

    // Read the data specification, parameters, and actions.
    let _data_spec = DataSpecification::read(&mut reader)?;
    let _parameters = reader.read_aterm()?;
    let _actions = reader.read_aterm()?;

    // An indexed set to keep track of indices for multi-actions
    let multi_actions = IndexedSet::<ATerm>::new();

    // Keep track of the number of states (derived from the transitions).
    let num_of_states: usize = 1;

    loop {
        let term = reader.read_aterm()?;
        match term {
            Some(t) => {
                // Process the term (state, transition, etc.)
                // For now, we just ignore it.
            }
            None => break, // The default constructed term indicates the end of the stream.
        }
    }

    unimplemented!();
}

/// Returns the ATerm marker for a labelled transition system.
fn lts_marker() -> ATerm {
    ATerm::constant(&Symbol::new("labelled_transition_system", 0))
}
