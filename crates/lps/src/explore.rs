use std::cell::Cell;
use std::cell::RefCell;
use std::ops::ControlFlow;
use std::rc::Rc;

use merc_aterm::Term;
use merc_collections::IndexedSet;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::bool_literal;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_enumerate::Outcome;
use merc_explore::CacheLPS;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_explore::LPS;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_explore::explore;
use merc_io::TimeProgress;
use merc_lts::LtsAction;
use merc_lts::LtsBuilder;
use merc_lts::LtsMultiAction;
use merc_lts::PerStateDedup;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::utilities::Chained;
use merc_sabre::utilities::SliceSubstitution;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::lps::ActionSummand;
use crate::lps::LinearProcessSpecification;
use crate::lps::contains_variable;

/// Interns process-parameter values into stable `usize` indices shared by
/// state vectors.
///
/// Each stored [`DataExpression`] is independently protected against garbage
/// collection, so the set needs no separate GC-rooting container.
type ValueMapping = Rc<RefCell<IndexedSet<DataExpression>>>;

/// A native, data-backed [`LPS`] implementation for a
/// [`LinearProcessSpecification`], instantiating `sum` variables with
/// `merc_enumerate::Enumerator` instead of the mCRL2 FFI enumerator used by
/// `tools/mcrl2/crates/merc_lps`.
///
/// # Concurrency
///
/// This first implementation is single-threaded only: [`ValueMapping`] uses
/// plain `Rc`/`RefCell` sharing, not the thread-safe interning
/// `tools/mcrl2/crates/merc_lps`'s FFI-backed version uses. Driving this LPS
/// through `merc_explore::explore_parallel` is not supported yet.
pub struct ExploreLinearProcessSpecification {
    lps: LinearProcessSpecification,
    summands: Vec<ExploreSummand>,
    rewrite_spec: RewriteSpecification,
    enumeration_plans: Rc<EnumerationPlans>,
    value_mapping: ValueMapping,
    initial_state: Vec<usize>,
    /// The [`InnermostRewriter`] built in [`Self::new`] to normalize the
    /// initial state, held onto so [`Self::create_context`] can reuse it
    /// instead of compiling a second `SetAutomaton` from the same
    /// [`RewriteSpecification`]. Taken by the first call; later calls (not
    /// expected while only sequential, single-context exploration is
    /// supported) fall back to building a fresh one.
    initial_rewriter: RefCell<Option<InnermostRewriter>>,
}

impl ExploreLinearProcessSpecification {
    /// Builds a native LPS driver from an already-read
    /// [`LinearProcessSpecification`], rewriting the initial process
    /// expressions to normal form to build the initial state vector.
    ///
    /// `lps.data_spec` must already be a complete data specification (every
    /// sort/constructor/mapping/equation the rewriter needs, standard
    /// prelude included) — [`crate::read_lps`]/[`crate::read_lps_file`]
    /// produce exactly that, and any other producer of a
    /// [`LinearProcessSpecification`] must too.
    ///
    /// Deadlock summands never contribute transitions — they mark a `delta`
    /// alternative, not a guarded action — so only the action summands
    /// become [`Summand`]s driving exploration.
    pub fn new(mut lps: LinearProcessSpecification) -> Self {
        lps.simplify_one_point_lps();

        let parameters = Rc::new(lps.process.parameters.clone());
        let rewrite_spec = RewriteSpecification::from_data_specification(&lps.data_spec);
        let enumeration_plans = Rc::new(EnumerationPlans::build(&lps.data_spec));
        let value_mapping: ValueMapping = Rc::new(RefCell::new(IndexedSet::new()));

        let mut rewriter = InnermostRewriter::new(&rewrite_spec);
        let summands = lps
            .process
            .action_summands
            .iter()
            .map(|summand| ExploreSummand::from_action_summand(summand, &parameters, &value_mapping))
            .collect();

        let initial_state = lps
            .initial_process
            .expressions()
            .iter()
            .map(|expr| {
                let value = rewriter.rewrite(&expr);
                *value_mapping.borrow_mut().insert(value).0
            })
            .collect();

        ExploreLinearProcessSpecification {
            lps,
            summands,
            rewrite_spec,
            enumeration_plans,
            value_mapping,
            initial_state,
            initial_rewriter: RefCell::new(Some(rewriter)),
        }
    }

    /// The underlying, fully-read specification.
    pub fn lps(&self) -> &LinearProcessSpecification {
        &self.lps
    }

    /// Recovers the typed value interned at `index` in the shared value
    /// mapping, e.g. one previously observed as a process parameter value
    /// during exploration.
    pub fn value(&self, index: usize) -> DataExpression {
        self.value_mapping
            .borrow()
            .get_by_index(index)
            .expect("interned value must be present in the value mapping")
            .clone()
    }
}

/// Per-thread (currently: the only thread) enumeration context. Everything in
/// here is reused across every summand and state visited on this thread, so
/// its caches and scratch buffers carry over instead of being rebuilt per
/// `enumerate` call.
pub struct ExploreContext {
    rewriter: InnermostRewriter,
    enumerator: Enumerator,
    generator: FreshVariableGenerator,
    next_state_buf: Vec<usize>,
    /// The source state's parameter values, resolved from the value mapping
    /// once per state in [`LPS::prepare`] and shared by every summand.
    state_values: Vec<DataExpression>,
}

/// A single summand of the LPS, prepared for enumeration with
/// `merc_enumerate::Enumerator`.
pub struct ExploreSummand {
    /// The indices of the parameters this summand reads: those free in the
    /// condition, the write assignments' right-hand sides, or the actions.
    read_indices: Vec<usize>,
    /// The indices of the parameters this summand writes. mCRL2's linearizer
    /// omits identity assignments from the wire format, so this has exactly
    /// one entry per element of `assignment_rhs`, in the same order.
    write_indices: Vec<usize>,
    /// The process parameters, shared with the parent LPS and every sibling
    /// summand.
    parameters: Rc<Vec<DataVariable>>,
    /// The residual condition, with every one-point-eliminated summation
    /// variable already substituted away — see
    /// [`LinearProcessSpecification::simplify_one_point_lps`].
    condition: DataExpression,
    /// The summation variables still to be enumerated; a subset of the raw
    /// summand's, with the ones the one-point rule eliminated removed.
    summation_variables: Vec<DataVariable>,
    /// Each write assignment's right-hand side, one-point-compiled, in
    /// `write_indices` order.
    assignment_rhs: Vec<DataExpression>,
    /// Each action's label name, read out once here rather than from the
    /// `ActionLabel` term on every solution.
    action_labels: Vec<String>,
    /// `action_labels[i]`'s argument count, so `action_args` (flattened
    /// across every action) can be re-chunked per action in
    /// [`ExploreSummand::enumerate`].
    action_arg_counts: Vec<usize>,
    /// Every action's arguments, one-point-compiled, flattened in
    /// `action_labels` order.
    action_args: Vec<DataExpression>,
    value_mapping: ValueMapping,
}

impl ExploreSummand {
    /// Builds an [`ExploreSummand`] from an already one-point-simplified
    /// [`ActionSummand`] — [`LinearProcessSpecification::simplify_one_point_lps`]
    /// must have run over the owning LPS first, so `summand.condition` is
    /// already the residual condition and `summand.summation_variables`
    /// already excludes every variable the one-point rule eliminated.
    fn from_action_summand(
        summand: &ActionSummand,
        parameters: &Rc<Vec<DataVariable>>,
        value_mapping: &ValueMapping,
    ) -> Self {
        let assignment_vars: Vec<DataVariable> =
            summand.assignments.iter().map(|a| a.arg(0).protect().into()).collect();
        let action_labels: Vec<String> = summand.actions.iter().map(|a| a.label().name().to_string()).collect();
        let action_arguments: Vec<Vec<DataExpression>> =
            summand.actions.iter().map(|a| a.arguments().to_vec()).collect();
        let action_arg_counts: Vec<usize> = action_arguments.iter().map(Vec::len).collect();

        let condition = summand.condition.clone();
        let assignment_rhs: Vec<DataExpression> = summand
            .assignments
            .iter()
            .map(|a| DataExpression::from(a.arg(1).protect()))
            .collect();
        let action_args: Vec<DataExpression> = action_arguments.into_iter().flatten().collect();

        let write_indices = write_indices_of(&assignment_vars, parameters);
        let read_indices = read_indices_of(
            parameters,
            &condition,
            assignment_rhs.iter().cloned(),
            action_args.iter().cloned(),
        );

        ExploreSummand {
            read_indices,
            write_indices,
            parameters: parameters.clone(),
            condition,
            summation_variables: summand.summation_variables.clone(),
            assignment_rhs,
            action_labels,
            action_arg_counts,
            action_args,
            value_mapping: value_mapping.clone(),
        }
    }
}

impl Summand for ExploreSummand {
    type Value = usize;
    type Label = LtsMultiAction<LtsAction>;
    type Context = ExploreContext;

    fn read_positions(&self) -> &[usize] {
        &self.read_indices
    }

    fn effect(&self) -> StateEffect<'_> {
        StateEffect::Positions(&self.write_indices)
    }

    fn enumerate<F>(&self, context: &mut Self::Context, state: &[usize], mut report: F) -> Result<(), MercError>
    where
        F: FnMut(&Self::Label, &[usize]) -> Result<(), MercError>,
    {
        let &mut ExploreContext {
            ref mut rewriter,
            ref mut enumerator,
            ref mut generator,
            ref mut next_state_buf,
            ref state_values,
        } = context;

        // The full parameter list, not just `read_indices`: only the variables
        // this summand actually mentions are ever looked up.
        let sigma = SliceSubstitution::new(&self.parameters, state_values);

        let condition = rewriter.rewrite_with(&self.condition, &sigma);

        // A guard that already rewrote to `false` under the state's values
        // alone can never become `true` for any summation-variable binding,
        // so there's nothing to search — skip setting up the enumerator.
        if condition == bool_literal(false) {
            return Ok(());
        }

        let outcome = enumerator.enumerate_normalized(
            rewriter,
            generator,
            &self.summation_variables,
            condition,
            |rewriter, solution| {
                // The summation variables shadow the process parameters, so
                // this layer has to come first.
                let solution_slice = SliceSubstitution::new(&self.summation_variables, solution.values());
                let solution_sigma = Chained::new(&solution_slice, &sigma);

                next_state_buf.clear();
                next_state_buf.extend_from_slice(state);
                for (rhs, &write_index) in self.assignment_rhs.iter().zip(self.write_indices.iter()) {
                    let value = rewriter.rewrite_with(rhs, &solution_sigma);
                    let index = *self.value_mapping.borrow_mut().insert(value).0;
                    next_state_buf[write_index] = index;
                }

                let mut actions = merc_collections::VecBag::new();
                // let mut offset = 0;
                // for (label, &count) in self.action_labels.iter().zip(self.action_arg_counts.iter()) {
                //     let arguments = self.action_args[offset..offset + count]
                //         .iter()
                //         .map(|arg| rewriter.rewrite_with(arg, &solution_sigma))
                //         .collect();
                //     offset += count;
                //     actions.insert(LtsAction::new(label.clone(), arguments));
                // }
                let label = LtsMultiAction::new(actions);

                match report(&label, next_state_buf) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(err) => ControlFlow::Break(err),
                }
            },
        );

        // Discard any names generated for this call before the next one
        // reuses `generator`, so its `used` set stays bounded by the LPS's
        // own variables rather than growing over the whole exploration.
        generator.reset();

        match outcome {
            Outcome::Exhausted => {}
            Outcome::Stopped(err) => return Err(err),
            Outcome::LimitReached => {
                return Err("sum-variable enumeration hit its search limit; the resulting state space would be an unsound under-approximation".into());
            }
            Outcome::NotEnumerable(variable, reason) => {
                return Err(format!("sum variable '{}' is not enumerable: {:?}", variable.name(), reason).into());
            }
            Outcome::Undecided(body) => {
                return Err(format!(
                    "summand condition '{}' rewrote to '{body}' rather than true or false for some instantiation of its sum variables; \
                     the data specification does not decide it, so the resulting state space would silently miss transitions",
                    self.condition
                )
                .into());
            }
        }

        Ok(())
    }
}

impl LPS for ExploreLinearProcessSpecification {
    type Value = usize;
    type Label = LtsMultiAction<LtsAction>;
    type StateInfo = ();
    type Summand = ExploreSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.initial_state.clone()
    }

    fn summands(&self) -> &[Self::Summand] {
        &self.summands
    }

    fn create_context(&self) -> ExploreContext {
        // Compiling a `SetAutomaton` is expensive, so hand over the one built
        // in `new()` before falling back to building another.
        let rewriter = self
            .initial_rewriter
            .borrow_mut()
            .take()
            .unwrap_or_else(|| InnermostRewriter::new(&self.rewrite_spec));

        // Every name a summand's fresh variables must avoid is fixed for the
        // lifetime of the LPS — the process parameters and every summand's
        // summation variables — since solution values are already-rewritten
        // normal forms and so never introduce a name of their own.
        let used_names = self
            .lps
            .process
            .parameters
            .iter()
            .chain(
                self.summands
                    .iter()
                    .flat_map(|summand| summand.summation_variables.iter()),
            )
            .map(|v| v.name().to_string());
        let generator = FreshVariableGenerator::new(used_names);

        ExploreContext {
            rewriter,
            enumerator: Enumerator::new(self.enumeration_plans.clone()),
            generator,
            next_state_buf: Vec::with_capacity(self.lps.process.parameters.len()),
            state_values: Vec::with_capacity(self.lps.process.parameters.len()),
        }
    }

    fn prepare<'a>(&'a self, context: &mut ExploreContext, state: &'a [usize]) -> impl Iterator<Item = usize> + 'a {
        debug_assert_eq!(
            state.len(),
            self.lps.process.parameters.len(),
            "state vector length must match the number of process parameters"
        );

        let mapping = self.value_mapping.borrow();
        context.state_values.clear();
        context.state_values.extend(state.iter().map(|&index| {
            mapping
                .get_by_index(index)
                .expect("state value must be present in the value mapping")
                .clone()
        }));

        // Every summand is a candidate for every source state.
        0..self.summands.len()
    }

    fn state_info(&self, _state: &[usize], _context: &ExploreContext) {}
}

/// Computes the sorted indices of `parameters` that occur free in `condition`
/// or any of `assignment_rhs`/`action_arguments`.
fn read_indices_of(
    parameters: &[DataVariable],
    condition: &DataExpression,
    assignment_rhs: impl Iterator<Item = DataExpression>,
    action_arguments: impl Iterator<Item = DataExpression>,
) -> Vec<usize> {
    let mut read = vec![false; parameters.len()];
    for (i, param) in parameters.iter().enumerate() {
        if contains_variable(condition.copy(), param) {
            read[i] = true;
        }
    }
    for rhs in assignment_rhs {
        for (i, param) in parameters.iter().enumerate() {
            if !read[i] && contains_variable(rhs.copy(), param) {
                read[i] = true;
            }
        }
    }
    for arg in action_arguments {
        for (i, param) in parameters.iter().enumerate() {
            if !read[i] && contains_variable(arg.copy(), param) {
                read[i] = true;
            }
        }
    }

    read.iter()
        .enumerate()
        .filter_map(|(i, &is_read)| is_read.then_some(i))
        .collect()
}

/// Computes the index into `parameters` of each assignment's left-hand
/// variable, in `assignments` order.
fn write_indices_of(assignment_vars: &[DataVariable], parameters: &[DataVariable]) -> Vec<usize> {
    assignment_vars
        .iter()
        .map(|lhs| {
            parameters
                .iter()
                .position(|p| p == lhs)
                .expect("an assignment's left-hand side must be a process parameter")
        })
        .collect()
}

/// Explores `lps` natively, forwarding the discovered transitions to `builder`.
///
/// Mirrors `tools/mcrl2/crates/merc_lps`'s `explore_lps_explicit` driver, but
/// without any FFI dependency: [`ExploreLinearProcessSpecification`] already
/// produces `merc_lts::LtsMultiAction<LtsAction>` labels directly, so no
/// adapter is needed to hand transitions to a `merc_lts` builder such as
/// `AutStream` or `LtsStream`.
pub fn explore_lps<B>(
    builder: &mut B,
    lps: LinearProcessSpecification,
    caching: CachingStrategy,
    strategy: ExplorationStrategy,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: LtsBuilder<LtsMultiAction<LtsAction>>,
{
    let native = ExploreLinearProcessSpecification::new(lps);

    match caching {
        CachingStrategy::None => run_explore(builder, &native, strategy, timing),
        _ => {
            let cached = CacheLPS::new(&native, caching);
            let result = run_explore(builder, &cached, strategy, timing)?;
            log::debug!("{}", cached.metrics());
            Ok(result)
        }
    }
}

/// Periodic progress reporter for LPS exploration, printing the number of
/// discovered states and transitions, mirroring
/// `tools/mcrl2/crates/merc_lps`'s `lps_progress`.
fn lps_progress() -> TimeProgress<(usize, usize)> {
    TimeProgress::new(
        |(states, transitions): (usize, usize)| {
            log::info!("Explored {states} states, {transitions} transitions...");
        },
        1,
    )
}

/// Runs the sequential exploration loop over any [`LPS`] view producing
/// `merc_lts::LtsMultiAction<LtsAction>` labels, deduplicating transitions per
/// source state before finalizing the builder.
fn run_explore<B, M>(
    builder: &mut B,
    lps: &M,
    strategy: ExplorationStrategy,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: LtsBuilder<LtsMultiAction<LtsAction>>,
    M: LPS<Value = usize, Label = LtsMultiAction<LtsAction>, StateInfo = ()>,
{
    let progress = lps_progress();
    let states = Cell::new(0usize);

    // Different summands can instantiate to the same (label, successor) pair
    // for a given state, so deduplicate them.
    let mut dedup = PerStateDedup::new();

    let initial = explore(
        lps,
        strategy,
        timing,
        builder,
        |_b: &mut B, _state, _info: &()| {
            states.set(states.get() + 1);
            Ok(())
        },
        |b: &mut B, from, label: &LtsMultiAction<LtsAction>, to| {
            dedup.add(from, label, to, |from, label, to| b.add_transition(from, label, to))?;
            progress.print((states.get(), b.num_of_transitions() + dedup.len()));
            Ok(())
        },
    )?;

    dedup.flush(|from, label, to| builder.add_transition(from, label, to))?;

    log::info!(
        "Exploration complete: {} states, {} transitions",
        states.get(),
        builder.num_of_transitions(),
    );
    builder.require_num_of_states(states.get());

    builder.finish(initial)
}
