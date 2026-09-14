use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::ATermList;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::BinaryATermReader;
use merc_aterm::Symb;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::DataWhrDecl;
use merc_data::Mcrl2DataSpecification;
use merc_data::is_undefined_real;
use merc_utilities::MercError;

use crate::lps::ActionSummand;
use crate::lps::DeadlockSummand;
use crate::lps::LinearProcess;
use crate::lps::LinearProcessSpecification;
use crate::terms::Action;
use crate::terms::ActionLabel;
use crate::terms::LinearProcessInit;

/// The marker term mCRL2 writes as the first term of a `.lps` stream, before
/// the data specification. See `lps_io.cpp:18-21,146` in the mCRL2 sources.
const MARKER_SYMBOL: &str = "linear_process_specification";

/// Reads a [`LinearProcessSpecification`] from the `.lps` file at `path`.
pub fn read_lps_file<P: AsRef<Path>>(path: P) -> Result<LinearProcessSpecification, MercError> {
    let file = File::open(path)?;
    let mut reader = BinaryATermReader::new(BufReader::new(file))?;
    read_lps(&mut reader)
}

/// Reads a [`LinearProcessSpecification`] from an already-open binary ATerm
/// stream, following mCRL2's `.lps` wire protocol exactly (see the module
/// doc comment on `crate` for the field sequence).
pub fn read_lps<R: ATermRead>(reader: &mut R) -> Result<LinearProcessSpecification, MercError> {
    let marker = next_term(reader, "the .lps marker term")?;
    let symbol = marker.get_head_symbol();
    if symbol.name() != MARKER_SYMBOL || symbol.arity() != 0 {
        return Err(format!(
            "Expected the '{MARKER_SYMBOL}' marker term, found a term headed by '{}' (arity {})",
            symbol.name(),
            symbol.arity()
        )
        .into());
    }

    // Complete the `.lps` file's own (`user_defined_*`-only) data section
    // with the standard prelude immediately, so every other module in this
    // crate — and any future producer of a `LinearProcessSpecification` — can
    // treat `data_spec` as already rewriter-ready.
    let data_spec = crate::prelude::with_standard_prelude(&Mcrl2DataSpecification::read(reader)?);

    let action_labels_term = next_term(reader, "the action label declarations")?;
    let action_labels: ATermList<ActionLabel> = action_labels_term.into();
    let action_labels = action_labels.to_vec();

    let global_variables = reader
        .read_aterm_iter()?
        .map(|t| t.map(DataVariable::from))
        .collect::<Result<Vec<_>, _>>()?;

    let parameters_term = next_term(reader, "the process parameters")?;
    let parameters: ATermList<DataVariable> = parameters_term.into();
    let parameters = parameters.to_vec();

    let action_summands = read_action_summands(reader)?;
    let deadlock_summands = read_deadlock_summands(reader)?;

    let initial_process_term = next_term(reader, "the initial process")?;
    let initial_process: LinearProcessInit = initial_process_term.into();

    Ok(LinearProcessSpecification {
        data_spec,
        action_labels,
        global_variables,
        process: LinearProcess {
            parameters,
            action_summands,
            deadlock_summands,
        },
        initial_process,
    })
}

/// Reads the next top-level term, turning end-of-stream into a descriptive
/// error instead of a silent `None`.
fn next_term<R: ATermRead>(reader: &mut R, what: &str) -> Result<ATerm, MercError> {
    reader
        .read_aterm()?
        .ok_or_else(|| format!("Unexpected end of stream while reading {what}").into())
}

/// Reads a summand's `time` field, mapping mCRL2's `@undefined_real` sentinel
/// (the wire encoding of "this process is untimed") to `None`.
fn read_time<R: ATermRead>(reader: &mut R, what: &str) -> Result<Option<DataExpression>, MercError> {
    let term = next_term(reader, what)?;
    let time: DataExpression = term.into();
    if is_undefined_real(&time) {
        Ok(None)
    } else {
        Ok(Some(time))
    }
}

/// Reads the `action_summands` vector: a count followed by that many
/// summands, each streamed as several consecutive top-level terms rather than
/// one term per element (see `lps_io.cpp:64-72`), so this cannot use
/// [`ATermRead::read_aterm_iter`], which assumes one term per element.
fn read_action_summands<R: ATermRead>(reader: &mut R) -> Result<Vec<ActionSummand>, MercError> {
    let count_term = next_term(reader, "the action summand count")?;
    let count: ATermInt = count_term.into();
    let mut summands = Vec::with_capacity(count.value());

    for _ in 0..count.value() {
        // The stochastic distribution is read and discarded: this crate does
        // not explore stochastic specifications.
        let _distribution = next_term(reader, "a summand's distribution")?;

        let summation_variables_term = next_term(reader, "a summand's summation variables")?;
        let summation_variables: ATermList<DataVariable> = summation_variables_term.into();

        let condition_term = next_term(reader, "a summand's condition")?;
        let condition: DataExpression = condition_term.into();

        let actions_term = next_term(reader, "a summand's actions")?;
        let actions: ATermList<Action> = actions_term.into();

        let time = read_time(reader, "a summand's time expression")?;

        let assignments_term = next_term(reader, "a summand's assignments")?;
        let assignments: ATermList<DataWhrDecl> = assignments_term.into();

        summands.push(ActionSummand {
            summation_variables: summation_variables.to_vec(),
            condition,
            actions: actions.to_vec(),
            time,
            assignments: assignments.to_vec(),
        });
    }

    Ok(summands)
}

/// Reads the `deadlock_summands` vector, following the same
/// count-then-multi-term-elements shape as [`read_action_summands`] (see
/// `lps_io.cpp:42-48`).
fn read_deadlock_summands<R: ATermRead>(reader: &mut R) -> Result<Vec<DeadlockSummand>, MercError> {
    let count_term = next_term(reader, "the deadlock summand count")?;
    let count: ATermInt = count_term.into();
    let mut summands = Vec::with_capacity(count.value());

    for _ in 0..count.value() {
        let summation_variables_term = next_term(reader, "a deadlock summand's summation variables")?;
        let summation_variables: ATermList<DataVariable> = summation_variables_term.into();

        let condition_term = next_term(reader, "a deadlock summand's condition")?;
        let condition: DataExpression = condition_term.into();

        let time = read_time(reader, "a deadlock summand's time expression")?;

        summands.push(DeadlockSummand {
            summation_variables: summation_variables.to_vec(),
            condition,
            time,
        });
    }

    Ok(summands)
}
