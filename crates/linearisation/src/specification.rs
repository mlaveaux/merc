use merc_syntax::ActionName;
use merc_syntax::DataExpr;
use merc_syntax::IdDecl;

/// A single action instance within a multi-action: `name(arg1, ..., argn)`.
///
/// A multi-action is `Vec<ActionInstance>`; the empty vector is `tau` (mCRL2
/// treats `tau` as literally the empty multi-action, not a distinct action).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ActionInstance {
    pub name: ActionName,
    pub arguments: Vec<DataExpr>,
}

impl ActionInstance {
    pub fn new(name: ActionName, arguments: Vec<DataExpr>) -> Self {
        ActionInstance { name, arguments }
    }
}

/// A `sum d:D . c(d,p) -> a(f(d,p)) . P(g(d,p))`-shaped summand: bound
/// variables, a guard, the multi-action it performs, and the assignment to
/// every process parameter afterwards.
///
/// `next_state` has exactly as many entries as
/// [`LinearProcessSpecification::process_parameters`], in the same order,
/// each expressed in terms of the process parameters and
/// `summation_variables` currently in scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionSummand {
    pub summation_variables: Vec<IdDecl>,
    pub condition: DataExpr,
    pub multi_action: Vec<ActionInstance>,
    pub next_state: Vec<DataExpr>,
}

/// A summand that only ever deadlocks: `sum d:D . c(d,p) -> delta`.
///
/// Kept separate from [`ActionSummand`] the way mCRL2's `deadlock_summand`
/// is, rather than folding it in as an summand with an empty multi-action —
/// an empty multi-action is `tau`, which *does* still perform a step, unlike
/// `delta`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadlockSummand {
    pub summation_variables: Vec<IdDecl>,
    pub condition: DataExpr,
}

/// A linear process specification (LPS): a process with one flat vector of
/// parameters, defined entirely by a set of [`ActionSummand`]s and
/// [`DeadlockSummand`]s over that vector, plus the initial value of each
/// parameter.
///
/// This is the target `merc_linearisation::translate::linearise` produces
/// from a `merc_typecheck::ProcessSpecification`; see the crate README for
/// what's implemented so far. It mirrors mCRL2's `lps::linear_process` /
/// `lps::specification` (`linear_process.h`, `specification.h`), but keeps
/// expressions at the `merc_syntax` (typed-source) level rather than lowering
/// them to `merc_data`'s aterm representation — see `translate.rs`'s module
/// documentation for why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinearProcessSpecification {
    pub process_parameters: Vec<IdDecl>,
    pub action_summands: Vec<ActionSummand>,
    pub deadlock_summands: Vec<DeadlockSummand>,
    /// The initial value of each process parameter, parallel to
    /// `process_parameters`.
    pub initial_assignment: Vec<DataExpr>,
}
