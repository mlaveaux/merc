use std::borrow::Borrow;
use std::cell::Cell;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use log::debug;
use log::info;

use mcrl2::_aterm;
use mcrl2::ATerm;
use mcrl2::ATermList;
use mcrl2::ATermSend;
use mcrl2::DataExpression;
use mcrl2::DataExpressionRef;
use mcrl2::DataVariable;
use mcrl2::LearnSuccessorsContext;
use mcrl2::LinearProcessSpecification;
use mcrl2::LinearSummand;
use mcrl2::Protected;
use mcrl2::free_variables_data_expression;
use mcrl2::is_variable;
use mcrl2::mcrl2_aterm_to_merc;
use mcrl2::pretty_print_multi_action;
use mcrl2::remove_index;
use mcrl2::tau_multi_action;
use merc_explore::CacheLPS;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_explore::LPS;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_explore::configure_rayon_thread_pool;
use merc_explore::explore;
use merc_explore::explore_parallel;
use merc_io::TimeProgress;
use merc_lts::ConcurrentLtsBuilder;
use merc_lts::LtsAction;
use merc_lts::LtsBuilder;
use merc_lts::LtsMultiAction;
use merc_lts::PerStateDedup;
use merc_lts::StateIndex;
use merc_lts::TransitionLabel;
use merc_unsafety::ConcurrentIndexedSet;
use merc_utilities::MercError;
use merc_utilities::ShardedCounter;
use merc_utilities::Timing;

use crate::cfg_lps::CfgLinearProcessSpecification;

/// Periodic progress reporter for LPS exploration, printing the number of
/// discovered states and transitions.
pub fn lps_progress() -> TimeProgress<(usize, usize)> {
    TimeProgress::new(
        |(states, transitions): (usize, usize)| {
            info!("Explored {states} states, {transitions} transitions...");
        },
        1,
    )
}

/// Explore the linear process specification explicitly, forwarding the
/// discovered transitions to `builder`.
///
/// The LPS is explored as given, any preprocessing must be applied beforehand.
///
/// When `control_flow` is set, a [`ControlFlowGraph`](mcrl2::ControlFlowGraph)
/// is layered on top of the explicit LPS to prune summands whose control flow
/// guard cannot hold in the current state (see [`CfgLinearProcessSpecification`]).
/// The pruning never changes the explored transition system.
pub fn explore_lps_explicit<B>(
    builder: &mut B,
    lps: LinearProcessSpecification,
    caching: CachingStrategy,
    strategy: ExplorationStrategy,
    control_flow: bool,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: LtsBuilder<Mcrl2MultiActionLabel>,
{
    if control_flow {
        let lps = CfgLinearProcessSpecification::new(lps)?;
        info!(
            "Control flow analysis identified {} control flow parameter(s)",
            lps.control_flow_parameters().len()
        );
        let result = explore_lps_explicit_impl(builder, &lps, caching, strategy, timing);
        debug!("{}", lps.metrics());
        result
    } else {
        let lps = ExplicitLinearProcessSpecification::new(lps)?;
        debug!("{lps:?}");
        explore_lps_explicit_impl(builder, &lps, caching, strategy, timing)
    }
}

/// Shared exploration driver over any explicit-state [`LPS`] view producing
/// mCRL2 multi-action labels, used by both the plain explicit explorer and the
/// control-flow-pruning variant.
fn explore_lps_explicit_impl<B, L>(
    builder: &mut B,
    lps: &L,
    caching: CachingStrategy,
    strategy: ExplorationStrategy,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: LtsBuilder<Mcrl2MultiActionLabel>,
    L: LPS<Value = usize, Label = Mcrl2MultiActionLabel, StateInfo = (), Summand = ExplicitSummand>,
{
    // Only layer the enumeration cache on top of the LPS when a caching strategy
    // is actually requested; otherwise explore the bare LPS directly.
    match caching {
        CachingStrategy::None => run_explore_explicit(builder, lps, strategy, timing),
        _ => {
            let cached = CacheLPS::new(lps, caching);
            let result = run_explore_explicit(builder, &cached, strategy, timing)?;
            debug!("{}", cached.metrics());
            Ok(result)
        }
    }
}

/// Runs the sequential explicit exploration loop over any [`LPS`] view producing
/// mCRL2 multi-action labels, counting states and transitions and finalising the
/// builder. The view is either the bare LPS or one wrapped in [`CacheLPS`].
fn run_explore_explicit<B, M>(
    builder: &mut B,
    lps: &M,
    strategy: ExplorationStrategy,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: LtsBuilder<Mcrl2MultiActionLabel>,
    M: LPS<Value = usize, Label = Mcrl2MultiActionLabel, StateInfo = ()>,
{
    // Count states and transitions in the exploration closures, driving the periodic progress
    // reporter from `on_transition`.
    let progress = lps_progress();
    let states = Cell::new(0usize);
    let transitions = Cell::new(0usize);

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
        |b: &mut B, from, label: &Mcrl2MultiActionLabel, to| {
            dedup.add(from, label, to, |from, label, to| {
                transitions.set(transitions.get() + 1);
                b.add_transition(from, label, to)
            })?;
            progress.print((states.get(), transitions.get() + dedup.len()));
            Ok(())
        },
    )?;

    dedup.flush(|from, label, to| {
        transitions.set(transitions.get() + 1);
        builder.add_transition(from, label, to)
    })?;

    info!(
        "Exploration complete: {} states, {} transitions",
        states.get(),
        transitions.get(),
    );
    builder.require_num_of_states(states.get());

    builder.finish(initial)
}

/// Explores the linear process specification explicitly in parallel across
/// `threads` worker threads, streaming the discovered transitions into
/// `builder`.
///
/// As for [`explore_lps_explicit`], the LPS is explored as given: preprocessing
/// it is the caller's responsibility.
pub fn explore_lps_explicit_parallel<B>(
    builder: &mut B,
    lps: LinearProcessSpecification,
    caching: CachingStrategy,
    threads: usize,
    control_flow: bool,
    pinned: bool,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: ConcurrentLtsBuilder<Mcrl2MultiActionLabel>,
{
    if control_flow {
        let lps = CfgLinearProcessSpecification::new(lps)?;
        info!(
            "Control flow analysis identified {} control flow parameter(s)",
            lps.control_flow_parameters().len()
        );
        let result = explore_lps_explicit_parallel_impl(builder, &lps, caching, threads, pinned, timing);
        debug!("{}", lps.metrics());
        result
    } else {
        let lps = ExplicitLinearProcessSpecification::new(lps)?;
        explore_lps_explicit_parallel_impl(builder, &lps, caching, threads, pinned, timing)
    }
}

/// Shared parallel exploration driver over any explicit-state [`LPS`] view,
/// used by both the plain explicit explorer and the control-flow-pruning
/// variant.
fn explore_lps_explicit_parallel_impl<B, L>(
    builder: &mut B,
    lps: &L,
    caching: CachingStrategy,
    threads: usize,
    pinned: bool,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: ConcurrentLtsBuilder<Mcrl2MultiActionLabel>,
    L: LPS<Value = usize, Label = Mcrl2MultiActionLabel, StateInfo = (), Summand = ExplicitSummand> + Sync,
{
    let pool = configure_rayon_thread_pool(threads, pinned)?;

    // Only layer the enumeration cache on top of the LPS when a caching strategy
    // is actually requested; otherwise explore the bare LPS directly.
    match caching {
        CachingStrategy::None => run_explore_explicit_parallel(builder, lps, &pool, timing),
        _ => {
            let cached = CacheLPS::new(lps, caching);
            let result = run_explore_explicit_parallel(builder, &cached, &pool, timing)?;
            debug!("{}", cached.metrics());
            Ok(result)
        }
    }
}

/// Runs the parallel explicit exploration loop over any [`LPS`] view producing
/// mCRL2 multi-action labels, streaming the discovered transitions into the
/// concurrent builder. The view is either the bare LPS or one wrapped in
/// [`CacheLPS`].
fn run_explore_explicit_parallel<B, M>(
    builder: &mut B,
    lps: &M,
    pool: &rayon::ThreadPool,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    B: ConcurrentLtsBuilder<Mcrl2MultiActionLabel>,
    M: LPS<Value = usize, Label = Mcrl2MultiActionLabel, StateInfo = ()> + Sync,
{
    // Shared (immutable) reference to the builder used by the worker threads;
    // `ConcurrentLtsBuilder` requires `Sync`, so the workers can add transitions
    // concurrently while the builder synchronises internally.
    let builder_ref: &B = builder;

    // Shared state/transition counts.
    let states = ShardedCounter::new();
    let transitions = ShardedCounter::new();
    let progress = lps_progress();

    // Forwards a deduplicated transition into the shared builder, counting it only once it is
    // actually written (see the `Local` doc below).
    let flush_one = |from, label: &Mcrl2MultiActionLabel, to| {
        transitions.increment();
        builder_ref.add_transition_shared(from, label, to)
    };

    let initial = timing.measure("explore", || -> Result<_, MercError> {
        pool.install(|| {
            // Each worker gets its own `PerStateDedup`: work is distributed per *state* (see
            // `explore_parallel`'s work-stealing deques), so one worker's outgoing transitions
            // for a given state are never interleaved with another's, and a single buffer per
            // worker is enough to catch the duplicates two different summands can instantiate to
            // (see `explore_lps_explicit`). A single buffer shared across workers would not work:
            // different workers process unrelated states concurrently.
            let (initial, mut locals) = explore_parallel(
                lps,
                PerStateDedup::<Mcrl2MultiActionLabel>::new,
                |_local: &mut PerStateDedup<Mcrl2MultiActionLabel>, _state, _info: &()| {
                    states.increment();
                    Ok(())
                },
                |local: &mut PerStateDedup<Mcrl2MultiActionLabel>, from, label: &Mcrl2MultiActionLabel, to| {
                    if progress.is_due() {
                        progress.print((states.get() as usize, transitions.get() as usize));
                    }
                    local.add(from, label, to, flush_one)
                },
            )?;

            // Each worker's last state is still sitting in its buffer; flush them all now that
            // exploration is complete.
            for local in &mut locals {
                local.flush(flush_one)?;
            }

            Ok(initial)
        })
    })?;

    let total_states = states.get() as usize;
    let total_transitions = transitions.get() as usize;
    info!("Exploration complete: {total_states} states, {total_transitions} transitions");

    // Finalise the builder, recording the total number of states so isolated
    // (deadlock) states are still reflected in the result.
    builder.require_num_of_states(total_states);

    builder.finish(initial)
}

/// A typed mCRL2 multi-action label backed by an [`ATermSend`].
///
/// We keep the term itself so labels remain maximally shared and can be used as
/// hash/ordering keys. Backing it with [`ATermSend`] (rather than [`ATerm`])
/// makes the label `Send + Sync`, so it can be created on one worker thread and
/// stored in (or dropped from) structures shared across threads — the
/// enumeration cache and the concurrent LTS builder. Display uses mCRL2's
/// pretty-printer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mcrl2MultiActionLabel {
    term: ATermSend,
}

impl Mcrl2MultiActionLabel {
    fn from_multi_action_term(term: ATermSend) -> Self {
        debug_assert!(
            is_mcrl2_timed_multi_action_term(&term),
            "Expected TimedMultAct term as transition label"
        );
        Self { term }
    }

    /// Protects the multi-action term on the current thread for use with the
    /// mCRL2 FFI, which expects a thread-local [`ATerm`]. Exposed so consumers
    /// can inspect the individual actions and their arguments, which `Display`
    /// flattens into a single pretty-printed string.
    pub fn as_aterm(&self) -> ATerm {
        self.term.protect_local()
    }

    /// Converts this label into the typed multi-action representation used by
    /// [`merc_lts`]'s binary `.lts` format, translating the underlying term into the
    /// `merc_aterm` term pool via [`mcrl2::mcrl2_aterm_to_merc`].
    pub fn to_lts_multi_action(&self) -> Result<LtsMultiAction<LtsAction>, MercError> {
        let term = remove_index(&self.as_aterm().copy());
        LtsMultiAction::from_mcrl2_aterm(mcrl2_aterm_to_merc(&term.copy()))
    }
}

/// Adapts an [`LtsBuilder<LtsMultiAction<LtsAction>>`] into one that accepts
/// [`Mcrl2MultiActionLabel`]s, converting each one via
/// [`Mcrl2MultiActionLabel::to_lts_multi_action`] as it is added.
///
/// This lets a typed builder such as [`merc_lts::LtsStream`] — which only understands the
/// binary `.lts` format's typed multi-actions — receive the untyped mCRL2 multi-actions the
/// explorer produces directly, streaming the conversion transition by transition instead of
/// converting the whole LTS after materialising it in memory.
pub struct LtsMultiActionAdapter<B> {
    inner: B,
}

impl<B> LtsMultiActionAdapter<B> {
    /// Wraps `inner`, a builder over the typed `.lts` multi-action representation.
    pub fn new(inner: B) -> Self {
        Self { inner }
    }
}

impl<B: LtsBuilder<LtsMultiAction<LtsAction>>> LtsBuilder<Mcrl2MultiActionLabel> for LtsMultiActionAdapter<B> {
    type LTS = B::LTS;

    fn add_transition<Q>(&mut self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        Mcrl2MultiActionLabel: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = Mcrl2MultiActionLabel> + Eq + Hash,
    {
        let converted = label.to_owned().to_lts_multi_action()?;
        self.inner.add_transition(from, &converted, to)
    }

    fn finish(&mut self, initial_state: StateIndex) -> Result<Self::LTS, MercError> {
        self.inner.finish(initial_state)
    }

    fn num_of_transitions(&self) -> usize {
        self.inner.num_of_transitions()
    }

    fn num_of_states(&self) -> usize {
        self.inner.num_of_states()
    }

    fn require_num_of_states(&mut self, num_states: usize) {
        self.inner.require_num_of_states(num_states)
    }
}

impl<B: ConcurrentLtsBuilder<LtsMultiAction<LtsAction>>> ConcurrentLtsBuilder<Mcrl2MultiActionLabel>
    for LtsMultiActionAdapter<B>
{
    fn add_transition_shared<Q>(&self, from: StateIndex, label: &Q, to: StateIndex) -> Result<(), MercError>
    where
        Mcrl2MultiActionLabel: Borrow<Q>,
        Q: ?Sized + ToOwned<Owned = Mcrl2MultiActionLabel> + Eq + Hash,
    {
        let converted = label.to_owned().to_lts_multi_action()?;
        self.inner.add_transition_shared(from, &converted, to)
    }
}

impl fmt::Display for Mcrl2MultiActionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", pretty_print_multi_action(&self.as_aterm()))
    }
}

impl TransitionLabel for Mcrl2MultiActionLabel {
    fn tau_label() -> Self {
        Self::from_multi_action_term(ATermSend::from(&tau_multi_action()))
    }

    fn is_tau_label(&self) -> bool {
        self == &Self::tau_label()
    }

    fn matches_label(&self, label: &str) -> bool {
        pretty_print_multi_action(&self.as_aterm()) == label
    }

    fn from_index(_i: usize) -> Self {
        panic!("Mcrl2MultiActionLabel does not support synthetic label generation")
    }
}

fn is_mcrl2_timed_multi_action_term(term: &ATermSend) -> bool {
    let term = term.copy();
    let symbol = term.get_head_symbol();
    symbol.name() == "TimedMultAct" && symbol.arity() == 2
}

/// Explicit-state view of a [mcrl2::LinearProcessSpecification] that implements
/// the [merc_explore::LPS] trait.
///
/// State vectors are indices into a shared `ValueMapping`: a thread-safe set
/// interning the data expressions observed for every process parameter. The set
/// stores bare [`DataExpressionRef`]s kept alive by the garbage collector
/// through the [`Protected`] wrapper, so the mapping is globally consistent and
/// safe to populate from several worker threads at once. Labels are the printed
/// multi-actions of the summands.
pub struct ExplicitLinearProcessSpecification {
    /// The underlying LPS. Retained because each per-thread [`ExplicitContext`]
    /// builds its own [`LearnSuccessorsContext`] from it.
    lps: LinearProcessSpecification,

    /// Cached process parameter variables in declaration order.
    process_parameters: Vec<*const _aterm>,

    /// The summands extracted from the LPS.
    summands: Vec<ExplicitSummand>,

    /// Concurrent value interning shared with every summand. Owned here through
    /// [`Protected`] so the garbage-collection protection is released when the
    /// LPS is dropped.
    value_mapping: Protected<ValueMapping>,

    /// The initial state vector.
    initial_state: Vec<usize>,
}

/// Shared interning of the data expressions observed as parameter values,
/// mapping each distinct expression to a dense `usize` used in state vectors.
type ValueMapping = ConcurrentIndexedSet<DataExpressionRef<'static>>;

// SAFETY: after construction the LPS is immutable except for `value_mapping`,
// whose backing [`ConcurrentIndexedSet`] is itself thread-safe. The cached
// `*const _aterm` parameter pointers are stable maximally shared term addresses,
// and mCRL2 garbage collection is stop-the-world, so no `&self` access can race
// with collection.
unsafe impl Sync for ExplicitLinearProcessSpecification {}

impl ExplicitLinearProcessSpecification {
    pub fn new(lps: LinearProcessSpecification) -> Result<Self, MercError> {
        let parameters = lps.parameters();
        let parameter_terms: Vec<DataVariable> = parameters.to_vec();
        let process_parameters: Vec<*const _aterm> = parameter_terms.iter().map(|param| param.address()).collect();
        let num_parameters = parameters.len();

        // Shared value interning, kept alive as a garbage-collection container
        // by `value_mapping`.
        let value_mapping = Protected::new(ValueMapping::new());

        let mut summands = Vec::new();
        for index in 0..lps.num_summands() {
            summands.push(ExplicitSummand::new(
                &lps.action_summand(index)?,
                &parameters,
                value_mapping.handle(),
            ));
        }

        // Temporary enumeration context used to rewrite the initial state
        // expressions to normal form. Its substitution is seeded with the
        // constant assignments recorded during preprocessing.
        let context = LearnSuccessorsContext::new(&lps);

        let initial_state = lps
            .initial_process()
            .expressions()
            .iter()
            .map(|param| {
                // Rewrite the initial expression under a context with the
                // constant assignments produced by
                // `replace_constants_by_variables`.
                let expr = unsafe { DataExpressionRef::from_address(param.address()) };
                let rewritten = context.rewrite_under_sigma(&expr);

                // SAFETY: the rewritten term is interned into `value_mapping`.
                value_mapping
                    .insert(unsafe { DataExpressionRef::from_address(rewritten.address()) })
                    .0
            })
            .collect::<Vec<usize>>();

        debug_assert_eq!(
            initial_state.len(),
            num_parameters,
            "Initial state vector length must match number of parameters"
        );

        Ok(ExplicitLinearProcessSpecification {
            lps,
            summands,
            process_parameters,
            value_mapping,
            initial_state,
        })
    }

    /// The process parameter variables in declaration order.
    pub fn parameters(&self) -> Vec<DataVariable> {
        self.lps.parameters().to_vec()
    }

    /// The underlying linear process specification.
    pub fn lps(&self) -> &LinearProcessSpecification {
        &self.lps
    }

    /// Recovers the typed value interned at `index` in the shared value mapping,
    /// e.g. one previously observed as a process parameter value during
    /// exploration.
    pub fn value(&self, index: usize) -> DataExpression {
        self.value_mapping
            .get_by_index(index)
            .expect("interned value must be present in the value mapping")
            .protect()
    }

    /// Rewrites `value` to normal form under `context` and interns it into the
    /// shared value mapping, returning its dense index.
    ///
    /// The rewriting and interning mirror the construction of the initial state
    /// vector, so the returned index can be compared directly against the
    /// entries of explored state vectors.
    pub fn intern_normal_form(&self, context: &LearnSuccessorsContext, value: &DataExpressionRef) -> usize {
        let rewritten = context.rewrite_under_sigma(value);

        // SAFETY: the rewritten term is interned into `self.value_mapping`, a
        // `Protected` container that keeps every interned term live through GC
        // marking for as long as the mapping exists.
        self.value_mapping
            .insert(unsafe { DataExpressionRef::from_address(rewritten.address()) })
            .0
    }
}

/// Per-thread enumeration context for an [`ExplicitLinearProcessSpecification`].
///
/// Owns the mCRL2 enumeration backend and the reusable scratch buffers, so the
/// LPS and its summands stay immutable and shareable by `&self` while each
/// worker thread drives its own context.
pub struct ExplicitContext {
    /// Backend used by mCRL2 to perform the enumeration, staged per source
    /// state by [`LPS::prepare`] and consumed by [`Summand::enumerate`].
    context: LearnSuccessorsContext,

    /// Reusable scratch buffer holding the `*const _aterm` parameter values
    /// resolved from the current source state. Filled during [`LPS::prepare`]
    /// and consumed immediately by [`LearnSuccessorsContext::set_assignments`].
    parameter_values: Vec<*const _aterm>,

    /// Reusable scratch buffer for the next-state vector produced for each
    /// enumerated solution. Reset and refilled for every solution.
    next_state_buf: Vec<usize>,
}

// Deliberately not `Send`: `context: LearnSuccessorsContext` is thread-affine (see
// `PbesSrfContext` in `explore_srf.rs`), and `crates/merc_lps/tests/
// explicit_context_send_soundness_test.rs` demonstrated the same cross-thread crash. No call
// site needs `Send` here either — see the removed `<P::Summand as Summand>::Context: Send`
// bound in `crates/explore/src/explore.rs`.

/// A single summand of the LPS, prepared for explicit enumeration.
pub struct ExplicitSummand {
    /// The indices of the parameters that this summand reads.
    read_indices: Vec<usize>,

    /// The indices of the parameters that this summand writes (non-identity
    /// assignments).
    write_indices: Vec<usize>,

    /// The condition of this summand.
    condition: DataExpression,

    /// The summation variables of this summand.
    summation_variables: ATermList<DataVariable>,

    /// Only the non-identity assignments (write parameters) of this summand.
    write_assignments: ATermList<ATerm>,

    /// The multi-action of this summand.
    multi_action: Mcrl2MultiActionLabel,

    /// Handle to the enclosing LPS's value interning, used to intern enumerated
    /// next-state values from any worker thread.
    mapping: Arc<ValueMapping>,
}

impl ExplicitSummand {
    fn new(summand: &LinearSummand, parameters: &ATermList<DataVariable>, mapping: Arc<ValueMapping>) -> Self {
        // Collect free variables from the condition.
        let mut read_vars = free_variables_data_expression(&summand.condition().copy());
        let parameters = parameters.to_vec();

        // Collect free variables from the update expressions and identify
        // which parameters are actually written.
        let mut write_vars = Vec::new();
        let mut write_assignments = Vec::new();

        for assignment in summand.assignments().iter() {
            let lhs: DataVariable = assignment.arg(0).protect().into();
            let rhs = assignment.arg(1);

            if DataExpressionRef::from(lhs.copy()) != DataExpressionRef::from(rhs.copy()) {
                let rhs_vars = free_variables_data_expression(&rhs.copy().into());
                read_vars.extend(rhs_vars);

                write_vars.push(lhs);
                write_assignments.push(assignment.protect());
            }
        }

        let write_assignments = ATermList::from_double_iter(write_assignments.into_iter());

        // The multi-action's data arguments (and time) may reference process
        // parameters that occur neither in the condition nor in any next-state
        // update.
        for subterm in summand.multi_action().iter() {
            if is_variable(&subterm) {
                read_vars.push(subterm.protect().into());
            }
        }

        let read_indices: Vec<usize> = parameters
            .iter()
            .enumerate()
            .filter(|(_, param)| read_vars.contains(param))
            .map(|(i, _)| i)
            .collect();

        let write_indices: Vec<usize> = parameters
            .iter()
            .enumerate()
            .filter(|(_, param)| write_vars.contains(param))
            .map(|(i, _)| i)
            .collect();

        let condition: DataExpression = summand.condition();
        let summation_variables: ATermList<DataVariable> = summand.summation_variables();

        debug_assert_eq!(
            write_indices.len(),
            write_assignments.iter().count(),
            "Number of write indices must match number of write assignments"
        );
        debug_assert!(read_indices.iter().is_sorted(), "Read indices must be strictly sorted");
        debug_assert!(
            write_indices.iter().is_sorted(),
            "Write indices must be strictly sorted"
        );

        let multi_action = Mcrl2MultiActionLabel::from_multi_action_term(ATermSend::from(&summand.multi_action()));

        Self {
            read_indices,
            write_indices,
            condition,
            summation_variables,
            write_assignments,
            multi_action,
            mapping,
        }
    }

    /// The condition (guard) of this summand.
    pub fn condition(&self) -> &DataExpression {
        &self.condition
    }

    /// The non-identity assignments (write parameters) of this summand.
    pub fn write_assignments(&self) -> &ATermList<ATerm> {
        &self.write_assignments
    }

    /// The multi-action template of this summand, before rewriting under any
    /// particular source state.
    pub fn multi_action(&self) -> &Mcrl2MultiActionLabel {
        &self.multi_action
    }
}

impl LPS for ExplicitLinearProcessSpecification {
    type Value = usize;
    type Label = Mcrl2MultiActionLabel;
    type StateInfo = ();
    type Summand = ExplicitSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.initial_state.clone()
    }

    fn summands(&self) -> &[Self::Summand] {
        &self.summands
    }

    fn create_context(&self) -> ExplicitContext {
        ExplicitContext {
            context: LearnSuccessorsContext::new(&self.lps),
            parameter_values: Vec::with_capacity(self.process_parameters.len()),
            next_state_buf: Vec::new(),
        }
    }

    fn prepare<'a>(
        &'a self,
        context: &mut ExplicitContext,
        state: &'a [Self::Value],
    ) -> impl Iterator<Item = usize> + 'a {
        debug_assert_eq!(
            state.len(),
            self.process_parameters.len(),
            "State vector length must match number of process parameters"
        );

        context.parameter_values.clear();
        for value_index in state.iter() {
            context.parameter_values.push(
                self.value_mapping
                    .get_by_index(*value_index)
                    .expect("Value must be in the mapping")
                    .address(),
            );
        }

        context
            .context
            .set_assignments(&self.process_parameters, &context.parameter_values);

        // Every summand is a candidate for every source state.
        0..self.summands.len()
    }

    fn state_info(&self, _state: &[Self::Value], _context: &<Self::Summand as Summand>::Context) -> Self::StateInfo {}
}

impl Summand for ExplicitSummand {
    type Value = usize;
    type Label = Mcrl2MultiActionLabel;
    type Context = ExplicitContext;

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
        // Borrow the backend and the next-state scratch buffer as disjoint
        // fields so the enumeration callback can fill the buffer while the
        // backend call is in progress.
        let ExplicitContext {
            context: learn,
            next_state_buf,
            ..
        } = context;

        // The mCRL2 backend wants a thread-local `ATerm` template; protect the
        // shared label term on this thread for the duration of the call.
        let multi_action_template = self.multi_action.as_aterm();

        // We cannot return errors through the C callback, so the first error is
        // captured here, further solutions are skipped, and it is propagated once
        // the FFI enumeration returns. This avoids unwinding into the C++ frame.
        let mut report_result: Result<(), MercError> = Ok(());

        learn.enumerate_raw_with_current_assignments(
            &self.condition,
            &self.summation_variables,
            &self.write_assignments,
            &multi_action_template,
            |values: &[*const _aterm], multi_action: *const _aterm| {
                if report_result.is_err() {
                    return;
                }

                debug_assert_eq!(
                    values.len(),
                    self.write_indices.len(),
                    "Enumerated values must match number of write indices"
                );

                // Build the next-state vector in the cached buffer instead of
                // allocating a fresh `Vec` per enumerated transition.
                next_state_buf.clear();
                next_state_buf.extend_from_slice(state);
                for (i, &value) in values.iter().enumerate() {
                    let param_index = self.write_indices[i];
                    // SAFETY: the term is interned into `self.mapping`, a
                    // `Protected` container that keeps every interned term live
                    // through GC marking for as long as the mapping exists.
                    let (new_index, _) = self.mapping.insert(unsafe { DataExpressionRef::from_address(value) });
                    next_state_buf[param_index] = new_index;
                }

                // Wrap the rewritten multi-action term as a typed label. The
                // `ATermSend` protects the term beyond the temporary handed to
                // the callback (via the global send protection set) and aterms
                // are maximally shared, so equal multi-actions share storage and
                // the label is safe to store across threads.
                // SAFETY: `multi_action` is the live rewritten multi-action term
                // handed to this callback by the mCRL2 enumerator.
                let label = Mcrl2MultiActionLabel::from_multi_action_term(unsafe { ATermSend::from_ptr(multi_action) });

                if let Err(err) = report(&label, next_state_buf) {
                    report_result = Err(err);
                }
            },
        );

        report_result
    }
}

impl fmt::Debug for ExplicitLinearProcessSpecification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ExplicitLinearProcessSpecification:")?;

        writeln!(f, "  Parameters:")?;
        for (i, param) in self.lps.parameters().iter().enumerate() {
            writeln!(f, "    {:?}: {:?}", i, param)?;
        }

        writeln!(f, "  Summands:")?;
        for (i, summand) in self.summands.iter().enumerate() {
            writeln!(f, "    {:?}: {:?}", i, summand)?;
        }
        Ok(())
    }
}

impl fmt::Debug for ExplicitSummand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} -> {}", self.condition.pretty_print(), self.multi_action)?;
        writeln!(f, "\t\tread indices: {:?}", self.read_indices)?;
        writeln!(f, "\t\twrite indices: {:?}", self.write_indices)
    }
}
