use std::io::Read;

use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::BinaryATermReader;
use merc_aterm::Symbol;
use merc_data::DataSpecification;
use merc_io::BitStreamRead;
use merc_ldd::BinaryLddReader;
use merc_ldd::Ldd;
use merc_ldd::Storage;
use merc_utilities::MercError;

/// Represents a symbolic LTS encoded by a disjunctive transition relation and a set of states.
pub struct SymbolicLts {
    data_specification: DataSpecification,

    states: Ldd,

    /// A singleton LDD representing the initial state.
    initial_state: Ldd,

    summand_groups: Vec<SummandGroup>,
}

impl SymbolicLts {
    /// Returns the LDD representing the set of states.
    pub fn states(&self) -> &Ldd {
        &self.states
    }

    /// Returns the LDD representing the initial state.
    pub fn initial_state(&self) -> &Ldd {
        &self.initial_state
    }
}

/// Represents a short vector transition relation for a group of summands.
struct SummandGroup {
    read_parameters: Vec<ATerm>,
    write_parameters: Vec<ATerm>,

    /// The transition relation T -> U for this summand group, such that T are the original parameters projected on the read_parameters and U the ones projected on the write_parameters.
    relation: Ldd,
}

/// Reads a symbolic LTS from a binary stream.
pub fn read_symbolic_lts<R: Read>(reader: R, storage: &mut Storage) -> Result<SymbolicLts, MercError> {
    let aterm_stream = BinaryATermReader::new(reader)?;
    let mut stream = BinaryLddReader::new(aterm_stream)?;

    if ATermRead::read_aterm(&mut stream)? != Some(symbolic_labelled_transition_system_mark()) {
        return Err("Expected symbolic labelled transition system stream".into());
    }

    let _data_spec = DataSpecification::read(&mut stream)?;
    let process_parameters: ATermList<ATerm> = stream.read_aterm()?.ok_or("Expected process parameters")?.into();

    let initial_state = stream.read_ldd(storage)?;
    let states = stream.read_ldd(storage)?;

    // Read the values for the process parameters.
    for _parameter in process_parameters {
        let num_of_entries = stream.read_integer()?;

        for _ in 0..num_of_entries {
            let _value = stream.read_aterm()?;
        }
    }

    // Read the action labels.
    let num_of_action_labels = stream.read_integer()?;
    for _ in 0..num_of_action_labels {
        let _action_label = stream.read_aterm()?;
    }

    // Read the summand groups.
    let mut summand_groups = Vec::new();
    let num_of_groups = stream.read_integer()?;
    for _ in 0..num_of_groups {
        let read_parameters: Vec<ATerm> = stream.read_aterm_iter()?.collect::<Result<Vec<_>, _>>()?;
        let write_parameters: Vec<ATerm> = stream.read_aterm_iter()?.collect::<Result<Vec<_>, _>>()?;

        let relation = stream.read_ldd(storage)?;

        summand_groups.push(SummandGroup {
            read_parameters,
            write_parameters,
            relation,
        });
    }

    Ok(SymbolicLts {
        data_specification: _data_spec,
        states,
        initial_state,
        summand_groups,
    })
}

/// Returns the ATerm mark for symbolic labelled transition systems.
fn symbolic_labelled_transition_system_mark() -> ATerm {
    ATerm::constant(&Symbol::new("symbolic_labelled_transition_system", 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_wms_sym() {
        let input = include_bytes!("../../../examples/lts/WMS.sym");

        let mut storage = Storage::new();
        let _lts = read_symbolic_lts(&input[..], &mut storage).unwrap();
    }
}
