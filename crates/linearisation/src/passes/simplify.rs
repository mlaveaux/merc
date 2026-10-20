use crate::data::NameGenerator;
use merc_lps::LinearProcess;

use crate::passes::stage::SumElimination;
use crate::passes::stage::SummandStage;

/// The final simplification pass: sum elimination on every summand (see [SumElimination]), which
/// parallel composition and communication have already applied to the summands they produced, and
/// removal of the deadlock summands, which contribute no behaviour without time.
pub(crate) fn simplify(process: &mut LinearProcess, names: &mut NameGenerator) {
    let summands = std::mem::take(&mut process.action_summands);
    process.action_summands = summands
        .into_iter()
        .filter_map(|summand| SumElimination.finish(summand, names))
        .collect();
    process.deadlock_summands.clear();
}
