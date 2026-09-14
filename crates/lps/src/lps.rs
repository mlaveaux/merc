use merc_aterm::ATermList;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::DataWhrDecl;
use merc_data::Mcrl2DataSpecification;
use merc_data::is_data_binder;
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;
use merc_data::is_data_where_clause;
use merc_enumerate::simplify_one_point;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;

use crate::terms::Action;
use crate::terms::ActionLabel;
use crate::terms::LinearProcessInit;

/// A single action-labelled summand of a [`LinearProcess`].
///
/// Corresponds to mCRL2's `stochastic_action_summand`, minus the stochastic
/// distribution (see [`crate::terms::Distribution`]), which this crate does
/// not act on.
#[derive(Clone)]
pub struct ActionSummand {
    /// The summation (`sum d: D . ...`) variables of this summand.
    pub summation_variables: Vec<DataVariable>,
    /// The guard of this summand.
    pub condition: DataExpression,
    /// The multi-action performed by this summand; empty for an internal
    /// (`tau`) action.
    pub actions: Vec<Action>,
    /// The time at which this summand's action occurs, if the process is
    /// timed. `None` for the overwhelmingly common untimed case (encoded on
    /// the wire as mCRL2's `@undefined_real`, see
    /// [`merc_data::is_undefined_real`]).
    pub time: Option<DataExpression>,
    /// The process-parameter assignments performed by this summand. Only
    /// non-identity assignments are guaranteed to be present here; readers
    /// that need every parameter's next value must default the rest to their
    /// current value, mirroring `ExplicitSummand`'s `write_assignments`.
    pub assignments: Vec<DataWhrDecl>,
}

/// A single deadlocking summand of a [`LinearProcess`]: a guarded time
/// constraint with no action and no successor.
#[derive(Clone)]
pub struct DeadlockSummand {
    /// The summation variables of this summand.
    pub summation_variables: Vec<DataVariable>,
    /// The guard of this summand.
    pub condition: DataExpression,
    /// The time at which this summand's deadlock occurs, if the process is
    /// timed. See [`ActionSummand::time`].
    pub time: Option<DataExpression>,
}

/// The linear process itself: its parameters and the summands defining its
/// transition relation.
pub struct LinearProcess {
    /// The process parameters, in declaration order; a state vector has
    /// exactly one value per parameter, in this order.
    pub parameters: Vec<DataVariable>,
    /// The action-labelled summands.
    pub action_summands: Vec<ActionSummand>,
    /// The deadlocking summands.
    pub deadlock_summands: Vec<DeadlockSummand>,
}

/// A native, fully in-memory representation of an mCRL2 Linear Process
/// Specification, read from its binary `.lps` format by
/// [`crate::io::read_lps_file`]/[`crate::io::read_lps`].
pub struct LinearProcessSpecification {
    /// The data specification the process is defined over.
    ///
    /// Must be *complete* — every sort/constructor/mapping/equation a
    /// rewriter needs, standard prelude included — not just what a source
    /// specification itself declares. [`crate::io::read_lps`] guarantees
    /// this by merging in the prelude (see `crate::prelude`) right after
    /// reading a `.lps` file's own `user_defined_*`-only sections; any other
    /// producer of a `LinearProcessSpecification` (e.g. a future native
    /// linearizer) must do the same.
    pub data_spec: Mcrl2DataSpecification,
    /// The declared action labels (name and parameter sorts).
    pub action_labels: Vec<ActionLabel>,
    /// Global (free) variables of the specification.
    pub global_variables: Vec<DataVariable>,
    /// The linear process.
    pub process: LinearProcess,
    /// The initial process state.
    pub initial_process: LinearProcessInit,
}

impl LinearProcessSpecification {
    /// Applies `merc_enumerate`'s one-point rule to every action summand's
    /// condition once, ahead of state space exploration, rather than
    /// per-state or per-context — mirroring mCRL2's own linearizer, which
    /// performs this kind of static simplification once rather than
    /// repeating it every time the process is driven. Each summand's
    /// eliminated summation variables are substituted into its write
    /// assignments and action arguments and dropped from
    /// `summation_variables`.
    pub fn simplify_one_point_lps(&mut self) {
        let rewrite_spec = RewriteSpecification::from_data_specification(&self.data_spec);
        let mut rewriter = InnermostRewriter::new(&rewrite_spec);

        for summand in &mut self.process.action_summands {
            simplify_summand_one_point(summand, &mut rewriter);
        }
    }
}

/// Runs the one-point rule once for `summand`; see
/// [`LinearProcessSpecification::simplify_one_point_lps`].
fn simplify_summand_one_point<R: RewriteEngine>(summand: &mut ActionSummand, rewriter: &mut R) {
    let assignment_vars: Vec<DataVariable> = summand.assignments.iter().map(|a| a.arg(0).protect().into()).collect();
    let action_arguments: Vec<Vec<DataExpression>> = summand.actions.iter().map(|a| a.arguments().to_vec()).collect();
    let action_arg_counts: Vec<usize> = action_arguments.iter().map(Vec::len).collect();

    let mut condition = rewriter.rewrite(&summand.condition);
    let mut rest: Vec<DataExpression> =
        Vec::with_capacity(assignment_vars.len() + action_arg_counts.iter().sum::<usize>());
    rest.extend(
        summand
            .assignments
            .iter()
            .map(|a| DataExpression::from(a.arg(1).protect())),
    );
    rest.extend(action_arguments.into_iter().flatten());

    simplify_one_point(rewriter, &summand.summation_variables, &mut condition, &mut rest);

    let mut rest = rest.into_iter();
    let assignment_rhs: Vec<DataExpression> = rest.by_ref().take(assignment_vars.len()).collect();
    let action_args: Vec<DataExpression> = rest.collect();

    // `simplify_one_point` doesn't report which summation variables it
    // eliminated — a caller only interested in that can just re-scan the
    // rewritten terms for them occurring free, which is what this does
    // across every term the elimination could have touched (not just
    // `condition`: a summation variable used only in an assignment or
    // action argument, never the condition, is not one-point-eligible at
    // all, but would also never be found free in `condition` — so checking
    // `condition` alone would wrongly drop it).
    summand.summation_variables.retain(|v| {
        contains_variable(condition.copy(), v)
            || assignment_rhs.iter().any(|rhs| contains_variable(rhs.copy(), v))
            || action_args.iter().any(|arg| contains_variable(arg.copy(), v))
    });

    summand.condition = condition;
    summand.assignments = assignment_vars
        .into_iter()
        .zip(assignment_rhs)
        .map(|(var, rhs)| DataWhrDecl::new(var, rhs))
        .collect();

    let mut action_args = action_args.into_iter();
    summand.actions = summand
        .actions
        .iter()
        .zip(action_arg_counts.iter())
        .map(|(action, &count)| {
            let arguments = ATermList::from_double_iter(action_args.by_ref().take(count));
            Action::new(action.label(), arguments)
        })
        .collect();
}

/// Returns true iff `var` occurs free anywhere in `term`.
///
/// Does not descend into binders or where clauses: an LPS condition, write
/// assignment, or action argument is not expected to contain either, since
/// quantifier elimination (`docs/enumeration-crate-plan.md` Phase 4) is not
/// wired into any rewriter yet.
pub(crate) fn contains_variable(term: DataExpressionRef<'_>, var: &DataVariable) -> bool {
    if is_data_variable(&term) {
        return Term::copy(&term) == Term::copy(var);
    }
    if is_data_machine_number(&term) || is_data_binder(&term) || is_data_where_clause(&term) {
        return false;
    }

    term.data_arguments().any(|arg| contains_variable(arg, var))
}
