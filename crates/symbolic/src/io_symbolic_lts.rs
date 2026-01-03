use std::io::Read;

use log::info;
use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::BinaryATermReader;
use merc_aterm::Symbol;
use merc_data::DataSpecification;
use merc_data::DataVariable;
use merc_io::BitStreamRead;
use merc_ldd::BinaryLddReader;
use merc_ldd::Storage;
use merc_utilities::MercError;

use crate::SummandGroup;
use crate::SymbolicLts;

/// Reads a symbolic LTS from a binary stream in the mCRL2 `.sym` format.
///
/// # Details
///
/// The stream contains
/// <marker>: ATerm
/// <data specification>
/// <process parameters>: ATermList<ATerm>
///
/// <initial state>: LDD
/// <states>: LDD
///
/// For each process parameter:
///   <number of entries>: u64
///   For each entry:
///     <value>: ATerm
///
/// <number of action labels>: u64
/// For each action label:
///   <action label>: ATerm
///
/// <number of summand groups>: u64
/// For each summand group:
///  <number of read parameters>: u64
///  For each read parameter:
///    <read parameter>: ATerm
///
/// <number of write parameters>: u64
/// For each write parameter:
///  <write parameter>: ATerm
pub fn read_symbolic_lts<R: Read>(storage: &mut Storage, reader: R) -> Result<SymbolicLts, MercError> {
    info!("Reading symbolic LTS in the mCRL2 symbolic format...");

    let aterm_stream = BinaryATermReader::new(reader)?;
    let mut stream = BinaryLddReader::new(aterm_stream)?;

    if ATermRead::read_aterm(&mut stream)? != Some(symbolic_labelled_transition_system_mark()) {
        return Err("Expected symbolic labelled transition system stream".into());
    }

    let data_spec = DataSpecification::read(&mut stream)?;
    let process_parameters: ATermList<DataVariable> = stream.read_aterm()?.ok_or("Expected process parameters")?.into();
    let process_parameters: Vec<DataVariable> = process_parameters.to_vec();

    let initial_state = stream.read_ldd(storage)?;
    let states = stream.read_ldd(storage)?;

    // Read the values for the process parameters.
    for _parameter in &process_parameters {
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
        // Note: this is not an ATermInt, as expected by `read_aterm_iter`, but a variable integer.
        let num_of_reads = stream.read_integer()?;
        let mut read_parameters: Vec<DataVariable> = Vec::with_capacity(num_of_reads as usize);
        for _ in 0..num_of_reads {
            read_parameters.push(stream.read_aterm()?.ok_or("Unexpected end of stream")?.into());
        }

        let num_of_writes = stream.read_integer()?;
        let mut write_parameters: Vec<DataVariable> = Vec::with_capacity(num_of_writes as usize);
        for _ in 0..num_of_writes {
            write_parameters.push(stream.read_aterm()?.ok_or("Unexpected end of stream")?.into());
        }

        let relation = stream.read_ldd(storage)?;

        summand_groups.push(SummandGroup::new(
            storage,
            &process_parameters,
            read_parameters,
            write_parameters,
            relation,
        )?);
    }

    Ok(SymbolicLts::new(data_spec, states, initial_state, summand_groups))
}

/// Returns the ATerm mark for symbolic labelled transition systems.
fn symbolic_labelled_transition_system_mark() -> ATerm {
    ATerm::constant(&Symbol::new("symbolic_labelled_transition_system", 0))
}

#[cfg(test)]
mod tests {
    use merc_utilities::test_logger;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_read_symbolic_lts_wms_sym() {
        test_logger();
        let input = include_bytes!("../../../examples/lts/WMS.sym");

        let mut storage = Storage::new();
        let _lts = read_symbolic_lts(&mut storage, &input[..]).unwrap();
    }
}
