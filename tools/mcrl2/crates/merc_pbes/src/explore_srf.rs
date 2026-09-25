use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use log::debug;
use log::info;

use mcrl2::_aterm;
use mcrl2::ATerm;
use mcrl2::ATermList;
use mcrl2::DataExpression;
use mcrl2::DataExpressionRef;
use mcrl2::DataSpecification;
use mcrl2::DataVariable;
use mcrl2::LearnSuccessorsContext;
use mcrl2::PbesPropositionalVariableInstantiation;
use mcrl2::Protected;
use mcrl2::SrfPbes;
use mcrl2::free_variables_data_expression;
use mcrl2::make_data_assignment_list;
use mcrl2::tau_multi_action;
use merc_explore::CacheLPS;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_explore::LPS;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_unsafety::ConcurrentIndexedSet;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::PGBuilder;
use merc_vpg::Player;
use merc_vpg::Priority;

use crate::cfg_srf::CfgPbesSrfLps;
use crate::explore_common::ParameterLayoutLPS;
use crate::explore_common::PbesVertex;
use crate::explore_common::compute_priorities;
use crate::explore_common::explore_pbes_impl;
use crate::explore_common::explore_pbes_parallel_impl;

/// Builds a parity game by exploring the given PBES in SRF format, using
/// `builder` to accumulate the result - see [`PGBuilder`].
///
/// When `control_flow` is set, a [`ControlFlowGraph`](mcrl2::ControlFlowGraph)
/// is layered on top of the SRF view to prune summands whose source-value guard
/// cannot hold in the current state (see [`CfgPbesSrfLps`]), in addition to the
/// pruning by equation index ([`PbesSrfLps::prepare`]) that always applies. The
/// pruning never changes the resulting parity game.
pub fn explore_srf_pbes<B: PGBuilder>(
    srf_pbes: SrfPbes,
    strategy: ExplorationStrategy,
    caching: CachingStrategy,
    control_flow: bool,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError> {
    if control_flow {
        let lps = CfgPbesSrfLps::new(srf_pbes)?;
        info!(
            "Control flow analysis identified {} control flow parameter(s)",
            lps.control_flow_parameters().len()
        );
        let result = explore_srf_lps(&lps, strategy, caching, timing, builder);
        debug!("{}", lps.metrics());
        result
    } else {
        let lps = PbesSrfLps::new(srf_pbes)?;
        explore_srf_lps(&lps, strategy, caching, timing, builder)
    }
}

/// Builds a parity game by exploring the given PBES in SRF format in
/// parallel, using `builder` to accumulate the result - see [`PGBuilder`].
///
/// As for [`explore_srf_pbes`], `control_flow` layers a [`CfgPbesSrfLps`] on top
/// of the plain SRF view.
pub fn explore_srf_pbes_parallel<B: PGBuilder>(
    srf_pbes: SrfPbes,
    threads: usize,
    caching: CachingStrategy,
    control_flow: bool,
    pinned: bool,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError> {
    if control_flow {
        let lps = CfgPbesSrfLps::new(srf_pbes)?;
        info!(
            "Control flow analysis identified {} control flow parameter(s)",
            lps.control_flow_parameters().len()
        );
        let result = explore_srf_lps_parallel(&lps, threads, caching, pinned, timing, builder);
        debug!("{}", lps.metrics());
        result
    } else {
        let lps = PbesSrfLps::new(srf_pbes)?;
        explore_srf_lps_parallel(&lps, threads, caching, pinned, timing, builder)
    }
}

/// Shared caching driver for [`explore_srf_pbes`], used for both the plain SRF
/// view and the control-flow-pruning variant.
fn explore_srf_lps<L, B: PGBuilder>(
    lps: &L,
    strategy: ExplorationStrategy,
    caching: CachingStrategy,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError>
where
    L: LPS<Value = usize, Label = (), StateInfo = PbesVertex, Summand = PbesSrfSummand>,
{
    match caching {
        CachingStrategy::None => explore_pbes_impl(lps, strategy, timing, builder),
        _ => {
            let cached = CacheLPS::new(lps, caching);
            let game = explore_pbes_impl(&cached, strategy, timing, builder)?;
            debug!("{}", cached.metrics());
            Ok(game)
        }
    }
}

/// Shared caching driver for [`explore_srf_pbes_parallel`], used for both the
/// plain SRF view and the control-flow-pruning variant.
fn explore_srf_lps_parallel<L, B: PGBuilder>(
    lps: &L,
    threads: usize,
    caching: CachingStrategy,
    pinned: bool,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError>
where
    L: LPS<Value = usize, Label = (), StateInfo = PbesVertex, Summand = PbesSrfSummand> + Sync,
{
    match caching {
        CachingStrategy::None => explore_pbes_parallel_impl(lps, threads, pinned, timing, builder),
        _ => {
            let cached = CacheLPS::new(lps, caching);
            let game = explore_pbes_parallel_impl(&cached, threads, pinned, timing, builder)?;
            debug!("{}", cached.metrics());
            Ok(game)
        }
    }
}

/// Per-thread enumeration context for a [`PbesSrfLps`].
///
/// Owns the mCRL2 enumeration backend and the reusable scratch buffers, so the
/// LPS and its summands stay immutable and shareable by `&self` while each
/// worker thread drives its own context.
pub struct PbesSrfContext {
    /// Backend used to evaluate summand conditions and enumerate solutions,
    /// staged per source state by [`LPS::prepare`].
    context: LearnSuccessorsContext,

    /// Reusable scratch buffer holding the resolved parameter pointers for the
    /// current source state.
    parameter_values: Vec<*const _aterm>,

    /// Reusable next-state buffer, pre-sized to `1 + num_params` and fully
    /// overwritten for every enumerated solution.
    next_state_buf: Vec<usize>,
}

// Deliberately not `Send`: `context: LearnSuccessorsContext` is documented as thread-affine
// (must be created and destroyed on the same thread), and `crates/merc_pbes/tests/
// pbes_srf_context_send_soundness_test.rs` demonstrated that moving a `PbesSrfContext` across
// threads and using it there crashes the process (a C++-side hashtable assertion in atermpp's
// thread-local protection-set bookkeeping). The only current call site
// (`merc_explore::explore_parallel`) already creates and drops its context within a single
// `rayon::broadcast` closure invocation and never needs to move it, so no `Send` bound is
// actually required — see the removed `<P::Summand as Summand>::Context: Send` bound in
// `crates/explore/src/explore.rs`.

/// Explicit-state view of a PBES in SRF normal form.
///
/// State vectors have layout `[equation_index, param_0, …, param_{n-1}]` where
/// `equation_index` is a flat index into [`SrfPbes::equations`] and each
/// `param_i` is an index into the shared `ValueMapping`.
pub struct PbesSrfLps {
    /// The unified SRF PBES; retained so summand pointers stay alive, and read
    /// back by [`PbesSrfLps::parameters`].
    srf: SrfPbes,

    /// Data specification used to build each per-thread [`PbesSrfContext`].
    data_spec: DataSpecification,

    /// Flat list of summands, one per `(equation, srf_summand)` pair.
    summands: Vec<PbesSrfSummand>,

    /// For each equation index, the indices into [`PbesSrfLps::summands`] of the
    /// summands belonging to that equation. A source state only explores the
    /// summands of its current equation (`state[0]`).
    equation_summands: Vec<Vec<usize>>,

    /// For each equation index, whether it is reachable from the initial
    /// equation via the summands' equation-index graph (ignoring data guards).
    /// See [`PbesSrfLps::is_equation_reachable`].
    equation_reachable: Vec<bool>,

    /// The initial state vector.
    initial_state: Vec<usize>,

    /// Per-equation vertex description used by [`LPS::state_info`]. Every SRF
    /// state stands for a propositional variable instantiation.
    state_info: Vec<PbesVertex>,

    /// Cached data-parameter variables (length `num_params`). All equations
    /// share the same parameter list after [`SrfPbes::unify_parameters`].
    process_parameters: Vec<*const _aterm>,

    /// Number of data parameters per equation.
    num_params: usize,

    /// Concurrent value interning shared with every summand. Owned here through
    /// [`Protected`] so the garbage-collection protection is released when the
    /// LPS is dropped.
    value_mapping: Protected<ValueMapping>,
}

/// Shared interning of the data expressions observed as parameter values,
/// mapping each distinct expression to a dense `usize` used in state vectors.
type ValueMapping = ConcurrentIndexedSet<DataExpressionRef<'static>>;

// SAFETY: after construction the LPS is immutable except for `value_mapping`,
// whose backing `ConcurrentIndexedSet` is itself thread-safe and is the only
// field workers mutate. The other terms are all !Send, but only read.
unsafe impl Sync for PbesSrfLps {}

/// A single SRF summand, pre-bound to the equation it belongs to and the
/// target equation it transitions into.
pub struct PbesSrfSummand {
    /// Source equation index; the summand fires only when `state[0]` equals it.
    equation_index: usize,

    /// Target equation index written to position 0 of the next state.
    target_equation_index: usize,

    /// The summand's data condition (after SRF conversion).
    condition: DataExpression,

    /// Existential summation variables of the summand.
    summation_variables: ATermList<DataVariable>,

    /// Pre-built assignment list `params := target_args(summation_vars, params)`
    /// passed verbatim to the enumerator.
    write_assignments: ATermList<ATerm>,

    /// Handle to the enclosing LPS's value interning, used to intern enumerated
    /// next-state values from any worker thread.
    mapping: Arc<ValueMapping>,

    /// Cached tau multi-action term; protected once at construction instead of
    /// on every summand enumeration.
    tau: ATerm,

    /// Number of data parameters, used to size the next-state buffer.
    num_params: usize,

    /// Positions in the state vector that fully determine this summand's
    /// enumeration result; position 0 (equation index) is always included.
    read_positions: Vec<usize>,

    /// Positions in the state vector that this summand may change; position 0
    /// (equation index) is always included.
    write_positions: Vec<usize>,
}

impl PbesSrfLps {
    /// Constructs a new [`PbesSrfLps`] from a PBES. Requires that the PBES has
    /// been converted to SRF and that all equations share the same parameter
    /// vector.
    pub fn new(srf_pbes: SrfPbes) -> Result<Self, MercError> {
        if srf_pbes.equations().is_empty() {
            return Err("PBES has no equations".into());
        }

        debug_assert!(
            srf_pbes.is_unified(),
            "PbesSrfLps::new requires a PBES whose equations share one parameter vector; \
             call `SrfPbes::unify_parameters` on it first"
        );

        let num_params = srf_pbes
            .equations()
            .iter()
            .map(|eq| eq.variable().parameters().len())
            .max()
            .unwrap_or(0);

        let is_mu: Vec<bool> = srf_pbes.equations().iter().map(|e| e.is_mu()).collect();
        let priorities = compute_priorities(&is_mu);

        // Equation name -> equation index, used when resolving target PVIs.
        let name_to_eq: HashMap<String, usize> = srf_pbes
            .equations()
            .iter()
            .enumerate()
            .map(|(i, eq)| (eq.variable().name().to_string(), i))
            .collect();

        // (Player, Priority) per equation. PBES convention: conjunctive (∧)
        // is owned by ∀ (Odd), disjunctive (∨) is owned by ∃ (Even).
        let state_info: Vec<PbesVertex> = srf_pbes
            .equations()
            .iter()
            .enumerate()
            .map(|(i, eq)| {
                let player = if eq.is_conjunctive() { Player::Odd } else { Player::Even };
                PbesVertex::instantiation(player, Priority::new(priorities[i]))
            })
            .collect();

        // Cached parameter-variable pointers. After `unify_parameters` all
        // equations share the same list, so we take the one of equation 0.
        let process_parameters: Vec<*const _aterm> = srf_pbes.equations()[0]
            .variable()
            .parameters()
            .iter()
            .map(|v: DataVariable| v.address())
            .collect();

        // Shared value interning, kept alive as a garbage-collection container
        // by `value_mapping`.
        let value_mapping = Protected::new(ValueMapping::new());

        let data_spec = srf_pbes.data_specification();
        let tau = tau_multi_action();

        // Build the initial state vector from the initial PVI.
        // After `unify_parameters`, the SRF's initial PVI has `num_params`
        // arguments; the original `pbes.initial_state()` may have fewer.
        let initial_pvi = srf_pbes.initial_state();
        let initial_eq_name = initial_pvi.name().to_string();
        let initial_eq_idx = *name_to_eq
            .get(&initial_eq_name)
            .ok_or_else(|| MercError::from(format!("Unknown initial equation: {initial_eq_name}")))?;

        let mut initial_state = Vec::with_capacity(1 + num_params);
        initial_state.push(initial_eq_idx);
        for arg in initial_pvi.arguments().iter() {
            // SAFETY: the term is interned into `value_mapping`, a `Protected`
            // container that keeps every interned term live through GC marking
            // for as long as the mapping exists.
            let (idx, _) = value_mapping.insert(unsafe { DataExpressionRef::from_address(arg.address()) });
            initial_state.push(idx);
        }

        // Flatten (equation, srf_summand) pairs into a single summand list,
        // recording which flat indices belong to each equation.
        let mut summands = Vec::new();
        let mut equation_summands: Vec<Vec<usize>> = vec![Vec::new(); srf_pbes.equations().len()];
        for (eq_idx, eq) in srf_pbes.equations().iter().enumerate() {
            // The parameters list of this equation (LHS of the assignment list).
            let eq_param_term: ATerm = eq.variable().parameters().into();
            let params_vec: Vec<DataVariable> = eq.variable().parameters().iter().collect();

            for srf_summand in eq.summands() {
                let target_pvi: PbesPropositionalVariableInstantiation = srf_summand.variable().into();
                let target_eq_name = target_pvi.name().to_string();
                let target_eq_idx = *name_to_eq
                    .get(&target_eq_name)
                    .ok_or_else(|| MercError::from(format!("Unknown target equation: {target_eq_name}")))?;

                let target_args: ATerm = target_pvi.arguments().protect().into();
                let assignments_term = make_data_assignment_list(&eq_param_term, &target_args);
                let write_assignments: ATermList<ATerm> = ATermList::new(assignments_term);

                // Collect free variables from the condition.
                let condition_de: DataExpression = srf_summand.condition().into();
                let mut read_vars = free_variables_data_expression(&condition_de.copy());

                // The summand's own existentially quantified variables. A target
                // argument that is syntactically identical to a process parameter is
                // only a genuine pass-through if it is *not* one of these: SRF
                // conversion is free to reuse a process parameter's name and sort for
                // a summation variable, in which case the argument refers to the
                // fresh quantified value, not the outer state parameter, even though
                // the two terms compare equal.
                let summation_vars: Vec<DataVariable> = srf_summand.parameters().iter().collect();

                // Position 0 (equation index) is always read; extend with positions
                // whose parameter appears free in the condition or a *written* target
                // argument. Also compute write positions: position 0 is always written;
                // position k+1 is written iff the k-th target argument is not the
                // identity (param_k).
                //
                // An identity argument contributes neither a write position nor a read
                // dependency: the effect is [`StateEffect::Positions`], so a position
                // outside the write set is replayed from the *live* source state rather
                // than from the cache, and the value the summand was originally
                // enumerated for is irrelevant. Adding it to the key would only split
                // cache entries that could be shared. This matters because
                // [`SrfPbes::unify_parameters`] above gives every equation the full
                // parameter vector, which turns each parameter an equation merely passes
                // through into exactly such an identity argument.
                let mut read_positions = vec![0usize];
                let mut write_positions = vec![0usize];
                for (k, (param, arg)) in params_vec.iter().zip(target_pvi.arguments().iter()).enumerate() {
                    if Into::<DataExpressionRef<'_>>::into(param.copy()) == arg.copy()
                        && !summation_vars.contains(param)
                    {
                        continue;
                    }
                    read_vars.extend(free_variables_data_expression(&arg.copy()));
                    write_positions.push(k + 1);
                }
                for (i, param) in params_vec.iter().enumerate() {
                    if read_vars.contains(param) {
                        read_positions.push(i + 1);
                    }
                }

                equation_summands[eq_idx].push(summands.len());
                summands.push(PbesSrfSummand {
                    equation_index: eq_idx,
                    target_equation_index: target_eq_idx,
                    condition: srf_summand.condition().into(),
                    summation_variables: srf_summand.parameters(),
                    write_assignments,
                    mapping: value_mapping.handle(),
                    tau: tau.clone(),
                    num_params,
                    read_positions,
                    write_positions,
                });
            }
        }

        // Equation-level reachability from the initial equation, ignoring data
        // guards: a coarse, conservative over-approximation of actual state
        // reachability.
        let mut equation_reachable = vec![false; srf_pbes.equations().len()];
        let mut stack = vec![initial_eq_idx];
        equation_reachable[initial_eq_idx] = true;
        while let Some(eq_idx) = stack.pop() {
            for &summand_idx in &equation_summands[eq_idx] {
                let target = summands[summand_idx].target_equation_index;
                if !equation_reachable[target] {
                    equation_reachable[target] = true;
                    stack.push(target);
                }
            }
        }

        Ok(Self {
            srf: srf_pbes,
            data_spec,
            summands,
            equation_summands,
            equation_reachable,
            initial_state,
            state_info,
            process_parameters,
            num_params,
            value_mapping,
        })
    }

    pub fn num_params(&self) -> usize {
        self.num_params
    }

    /// The `(Player, Priority)` of every equation, indexed by equation index.
    ///
    /// This is the same information `LPS::state_info` reports per-state, exposed directly so the
    /// symbolic path (which learns equation *values*, not per-state `state_info` calls) can build a
    /// [`merc_vpg::SymbolicParityGame`] from it without recomputing `compute_priorities`.
    pub fn equation_info(&self) -> &[PbesVertex] {
        &self.state_info
    }

    /// The unified data parameters, in state-vector order: entry `i` occupies
    /// state position `1 + i`.
    pub fn parameters(&self) -> Vec<DataVariable> {
        self.srf.equations()[0].variable().parameters().iter().collect()
    }

    /// Creates a fresh [`LearnSuccessorsContext`] for rewriting closed terms to
    /// normal form, independent of the per-thread enumeration contexts created by
    /// [`LPS::create_context`]. Used by [`mcrl2::ControlFlowGraph`].
    pub fn analysis_context(&self) -> LearnSuccessorsContext {
        LearnSuccessorsContext::from_data_spec(&self.data_spec)
    }

    /// Whether `equation_index` is reachable from the initial equation via the
    /// summands' equation-index graph (ignoring data guards — a coarse,
    /// conservative over-approximation of actual state reachability).
    ///
    /// [`mcrl2::ControlFlowGraph`] uses this to keep a summand whose own
    /// equation is unreachable from spuriously disqualifying a genuine control
    /// flow parameter merely by existing. mCRL2's boilerplate `true`/`false`
    /// SRF sink equations are *not* an instance of this: the summand that
    /// resets parameters into such a sink belongs to the (ordinarily
    /// reachable) equation that calls into it, not to the sink itself, so this
    /// reachability check does not see it — see
    /// [`mcrl2::ControlFlowGraph`]'s doc comment for what actually protects
    /// against that case.
    pub fn is_equation_reachable(&self, equation_index: usize) -> bool {
        self.equation_reachable[equation_index]
    }

    /// Rewrites `value` to normal form under `context` and interns it into the
    /// shared value mapping, returning its dense index.
    ///
    /// The rewriting and interning mirror the enumeration performed during
    /// exploration, so the returned index can be compared directly against the
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

impl LPS for PbesSrfLps {
    type Value = usize;
    type Label = ();
    type StateInfo = PbesVertex;
    // A PBES has no notion of an action; do not encode a trailing "action" dimension for it.
    const HAS_LABELS: bool = false;
    type Summand = PbesSrfSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.initial_state.clone()
    }

    fn summands(&self) -> &[Self::Summand] {
        &self.summands
    }

    fn create_context(&self) -> PbesSrfContext {
        PbesSrfContext {
            context: LearnSuccessorsContext::from_data_spec(&self.data_spec),
            parameter_values: Vec::with_capacity(self.num_params),
            next_state_buf: vec![0; 1 + self.num_params],
        }
    }

    fn prepare<'a>(
        &'a self,
        context: &mut PbesSrfContext,
        state: &'a [Self::Value],
    ) -> impl Iterator<Item = usize> + 'a {
        debug_assert_eq!(
            state.len(),
            1 + self.num_params,
            "State vector length must match 1 + number of parameters"
        );

        // Look up the *const _aterm representative for each parameter value.
        context.parameter_values.clear();
        for &value_index in state.iter().skip(1) {
            context.parameter_values.push(
                self.value_mapping
                    .get_by_index(value_index)
                    .expect("Parameter value must be in mapping")
                    .address(),
            );
        }

        context
            .context
            .set_assignments(&self.process_parameters, &context.parameter_values);

        // Only the summands of the current equation (`state[0]`) can fire.
        self.equation_summands[state[0]].iter().copied()
    }

    fn state_info(&self, state: &[Self::Value], _context: &PbesSrfContext) -> Self::StateInfo {
        self.state_info[state[0]]
    }
}

impl ParameterLayoutLPS for PbesSrfLps {
    fn parameter_range(&self, state: &[usize]) -> Option<Range<usize>> {
        // Every SRF state is `[equation_index, params...]`.
        debug_assert_eq!(state.len(), 1 + self.num_params());
        Some(1..1 + self.num_params())
    }
}

impl PbesSrfSummand {
    /// The equation this summand belongs to; it only ever fires from a state
    /// whose equation-index position (state position 0) equals this.
    pub fn equation_index(&self) -> usize {
        self.equation_index
    }

    /// The summand's data condition (after SRF conversion).
    pub fn condition(&self) -> &DataExpression {
        &self.condition
    }

    /// The assignment list `params := target_args`, containing only the
    /// parameters this summand actually changes: `make_data_assignment_list`
    /// filters out identity assignments, so a parameter absent here is passed
    /// through unchanged.
    pub fn write_assignments(&self) -> &ATermList<ATerm> {
        &self.write_assignments
    }
}

impl Summand for PbesSrfSummand {
    type Value = usize;
    type Label = ();
    type Context = PbesSrfContext;

    fn read_positions(&self) -> &[usize] {
        &self.read_positions
    }

    fn effect(&self) -> StateEffect<'_> {
        // An SRF summand always emits `[target_equation, params...]`, which has
        // the same length as every source state.
        StateEffect::Positions(&self.write_positions)
    }

    fn enumerate<F>(&self, context: &mut Self::Context, state: &[usize], mut report: F) -> Result<(), MercError>
    where
        F: FnMut(&Self::Label, &[usize]) -> Result<(), MercError>,
    {
        // PBES summands only fire from their owning equation. `LPS::prepare`
        // already restricts enumeration to the current equation's summands, so
        // this only ever runs for a matching state.
        debug_assert_eq!(
            state[0], self.equation_index,
            "summand enumerated for a state of a different equation"
        );

        // Borrow the backend and the next-state scratch buffer as disjoint
        // fields so the enumeration callback can fill the buffer while the
        // backend call is in progress.
        let PbesSrfContext {
            context: learn,
            next_state_buf,
            ..
        } = context;

        // We cannot return errors through the C callback, so the first error is
        // captured here, further solutions are skipped, and it is propagated once
        // the FFI enumeration returns. This avoids unwinding into the C++ frame.
        let mut report_result: Result<(), MercError> = Ok(());

        learn.enumerate_raw_with_current_assignments(
            &self.condition,
            &self.summation_variables,
            &self.write_assignments,
            &self.tau,
            |next_values: &[*const _aterm], _multi_action| {
                if report_result.is_err() {
                    return;
                }

                debug_assert_eq!(
                    next_values.len(),
                    self.num_params,
                    "Enumerated values must match number of parameters"
                );

                next_state_buf[0] = self.target_equation_index;
                for (i, &ptr) in next_values.iter().enumerate() {
                    // SAFETY: the term is interned into `self.mapping`, a
                    // `Protected` container that keeps every interned term live
                    // through GC marking for as long as the mapping exists.
                    let (idx, _) = self.mapping.insert(unsafe { DataExpressionRef::from_address(ptr) });
                    next_state_buf[1 + i] = idx;
                }

                // The PBES has no actions; we pass a placeholder unit label.
                if let Err(err) = report(&(), next_state_buf) {
                    report_result = Err(err);
                }
            },
        );

        report_result
    }
}
