use std::collections::HashMap;
use std::collections::HashSet;
use std::iter;
use std::ops::Range;
use std::sync::Arc;

use mcrl2::_aterm;
use mcrl2::ATermStringRef;
use mcrl2::DataExpression;
use mcrl2::DataExpressionRef;
use mcrl2::DataSpecification;
use mcrl2::DataVariable;
use mcrl2::Pbes;
use mcrl2::PbesConnective;
use mcrl2::PbesExpression;
use mcrl2::PbesExpressionRef;
use mcrl2::PbesExpressionVisitor;
use mcrl2::PbesFlattenIter;
use mcrl2::PbesFlattenStack;
use mcrl2::PbesPropositionalVariableInstantiationRef;
use mcrl2::PbesRewriteContext;
use mcrl2::Protected;
use mcrl2::is_pbes_and;
use mcrl2::is_pbes_false;
use mcrl2::is_pbes_or;
use mcrl2::is_pbes_propositional_variable_instantiation;
use mcrl2::is_pbes_true;
use mcrl2::is_variable;
use mcrl2::variable_occurrences_data_expression;
use merc_explore::ExplorationStrategy;
use merc_explore::LPS;
use merc_explore::OwnedStateEffect;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_unsafety::ConcurrentIndexedSet;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::PGBuilder;
use merc_vpg::Player;
use merc_vpg::Priority;

use crate::explore_common::ParameterLayoutLPS;
use crate::explore_common::PbesVertex;
use crate::explore_common::PbesVertexKind;
use crate::explore_common::UNIFY_IGNORE_CE_EQUATIONS;
use crate::explore_common::UNIFY_RESET_PARAMETERS;
use crate::explore_common::compute_priorities;
use crate::explore_common::explore_pbes_impl;
use crate::explore_common::explore_pbes_parallel_impl;

/// Tag values occupy the top 4 bits of a `state[0]` word so they never collide
/// with equation indices (which are small) or with usize::MAX (the sequence
/// forest's empty-slot sentinel).
///
/// `[AND_OP/OR_OP, subformula_idx]` is how an *intermediate* vertex of the
/// parity game graph is represented — a nested and/or sub-formula that has no
/// equation of its own, produced when flattening ([`enumerate_formula_children`])
/// can't remove a connective nested inside the other one (e.g. the `||` in
/// `(A || B) && C`). `AND_OP`/`OR_OP` says which connective it is (also fixing
/// its owner, the same way [`player_of`] does for an equation), and
/// `subformula_idx` is its slot in `subformula_mapping`; together they are all
/// [`emit_as_target`] needs to create such a vertex and later re-expand it.
const TAG_MASK: usize = 0xF << (usize::BITS - 4);
/// Source state is a true-sink (self-loop, Even wins).
const TRUE_SINK: usize = 0x1 << (usize::BITS - 4);
/// Source state is a false-sink (self-loop, Odd wins).
const FALSE_SINK: usize = 0x2 << (usize::BITS - 4);
/// Source state is a subformula AND node; full state: `[AND_OP, subformula_idx]`.
const AND_OP: usize = 0x3 << (usize::BITS - 4);
/// Source state is a subformula OR node; full state: `[OR_OP, subformula_idx]`.
const OR_OP: usize = 0x4 << (usize::BITS - 4);

/// The priority every subformula vertex is given.
///
/// A subformula vertex is a syntactic artefact rather than a fixpoint, so it
/// must not influence which priority dominates a cycle. Giving it the neutral
/// minimum achieves that: [`compute_priorities`] only produces priorities `>= 0`,
/// so a subformula vertex is never the maximum on a cycle that also contains an
/// instantiation — and every cycle does, because a subformula vertex's
/// successors are strict subterms of its own formula (see [`emit_as_target`]),
/// which makes the subgraph of subformula vertices acyclic.
///
/// This is what lets the same nested formula be *one* vertex no matter which
/// equations reach it, mirroring mCRL2's `SG1`, whose `insert_vertex(psi)` is
/// keyed on the formula alone and leaves the rank of such a vertex undefined.
const SUBFORMULA_PRIORITY: usize = 0;

type ValueMapping = ConcurrentIndexedSet<DataExpressionRef<'static>>;
type SubformulaMapping = ConcurrentIndexedSet<PbesExpressionRef<'static>>;

/// Maps a propositional variable name to the index of the equation defining it.
///
/// Keyed on the name term rather than on a `String`: names are maximally shared,
/// so a lookup is a pointer hash, where rendering the name costs two `String`
/// allocations per edge.
type NameMapping = HashMap<ATermStringRef<'static>, usize>;

/// The key of a [`NameMapping`], for a name read off a live term.
///
/// # Safety
///
/// The name must be a subterm of a term that stays alive for as long as the
/// mapping is used.
unsafe fn name_key(name: ATermStringRef<'_>) -> ATermStringRef<'static> {
    // SAFETY: the caller upholds that the name stays live.
    unsafe { ATermStringRef::from_address(name.address()) }
}

/// Parity-game LPS that directly explores a PBES without converting to SRF.
///
/// Applies the `enumerate_quantifiers_rewriter` on-the-fly to instantiate each
/// equation's right-hand side, then walks the resulting PBES expression to find
/// immediate successor states.
///
/// State layout:
/// - PVI state:  `[eq_idx, intern(v0), …, intern(vn)]` — length `1 + num_params`
/// - TRUE sink:  `[TRUE_SINK]`
/// - FALSE sink: `[FALSE_SINK]`
/// - Subformula AND: `[AND_OP, subformula_idx]`
/// - Subformula OR:  `[OR_OP,  subformula_idx]`
///
/// # Why this explorer is not wrapped in a [`merc_explore::CacheLPS`]
///
/// Unlike [`crate::explore_srf::PbesSrfLps`], which caching speeds up
/// substantially, none of this explorer's summands can benefit:
///
/// - An explorer enumerates each state exactly once, so a cache hit needs *two
///   distinct* states to share a key. The sink and subformula summands read
///   `[0]` and `[0, 1]` respectively, which is their entire state, so their keys
///   are injective and they never hit — measured on `alloc3`, the subformula
///   summand stored one entry per subformula vertex for zero hits.
/// - An equation summand is [`StateEffect::Opaque`] unless its right-hand side is
///   a bare instantiation (see `formula_positions`). Under an opaque effect the
///   whole next state is captured, so every parameter the right-hand side merely
///   passes through has to be part of the key as well, which widens the key to
///   the point where distinct states rarely share one.
/// - The expensive work — the quantifier-enumerating rewrite of the right-hand
///   side — happens in [`LPS::prepare`], which
///   [`merc_explore::CacheLPS`] forwards uncached. Even on a hit that cost is
///   already paid, leaving only the cheap walk over the rewritten formula to save.
///
/// Making caching worthwhile here would need a state effect that can describe
/// "either an opaque vector or the source with these positions overwritten", so
/// that pass-through values could be replayed from the live source state instead
/// of joining the key.
pub struct PbesLps {
    /// The unified PBES; retained so the terms borrowed by the summands stay
    /// alive, and read back by [`PbesLps::parameters`].
    pbes: Pbes,

    /// Data specification used to build each per-thread [`PbesContext`].
    data_spec: DataSpecification,

    /// Flat list of summands: summand `i` instantiates equation `i` (so a source
    /// state fires exactly the summand named by its equation index `state[0]`),
    /// followed by the two sink summands and the subformula summand.
    summands: Vec<PbesSummand>,

    /// Index into [`PbesLps::summands`] of the summand fired by a true sink state.
    true_sink_summand: usize,

    /// Index into [`PbesLps::summands`] of the summand fired by a false sink state.
    false_sink_summand: usize,

    /// Index into [`PbesLps::summands`] of the summand fired by a subformula node.
    subformula_summand: usize,

    /// The initial state vector.
    initial_state: Vec<usize>,

    /// Cached data-parameter variables (length `num_params`). All equations share
    /// the same parameter list after unification.
    process_parameters: Vec<*const _aterm>,

    /// Number of data parameters shared by every equation.
    num_params: usize,

    /// Interning table for enumerated parameter values, shared with the summands.
    /// Retained here because dropping it would unprotect every interned term.
    value_mapping: Protected<ValueMapping>,

    /// Interning table for nested (and/or) sub-formulas, shared with the summands.
    /// Only ever accessed through the handles held by the summands; retained here
    /// because dropping it would unprotect every interned sub-formula.
    #[allow(dead_code)]
    subformula_mapping: Protected<SubformulaMapping>,
}

// SAFETY: after construction, PbesLps is immutable except for the two
// ConcurrentIndexedSets (which are thread-safe) and the Pbes (read-only).
unsafe impl Sync for PbesLps {}

/// Determines which state shape a [`PbesSummand`] fires on and how it computes
/// its successors.
enum PbesSummandKind {
    /// Instantiates the right-hand side of an equation for the current parameter
    /// values; `priority` is the parity-game priority of that equation.
    Equation {
        formula: mcrl2::PbesExpression,
        priority: usize,
    },

    /// A sink, which has itself as its only successor; the payload is the sink's
    /// own single-word state ([`TRUE_SINK`] or [`FALSE_SINK`]).
    Sink(usize),

    /// Expands an interned nested sub-formula into its operands.
    Subformula,
}

/// A single summand of a [`PbesLps`], pre-bound to the state shape it fires on.
pub struct PbesSummand {
    /// The state shape this summand fires on and how successors are derived.
    kind: PbesSummandKind,

    /// Handle to the enclosing LPS's value interning, used to intern enumerated
    /// next-state values from any worker thread.
    value_mapping: Arc<ValueMapping>,

    /// Handle to the enclosing LPS's sub-formula interning.
    subformula_mapping: Arc<SubformulaMapping>,

    /// Maps a propositional variable name to its equation index.
    name_to_eq: Arc<NameMapping>,

    /// Positions of the source state read by this summand.
    read_positions: Vec<usize>,

    /// How this summand's next states relate to its source state.
    effect: OwnedStateEffect,
}

/// The lookup tables needed to encode a PBES sub-formula as a parity-game state: equation
/// names to indices, plus the interning tables for parameter values and sub-formulas.
#[derive(Clone, Copy)]
struct TargetTables<'a> {
    name_to_eq: &'a NameMapping,
    value_mapping: &'a ValueMapping,
    subformula_mapping: &'a SubformulaMapping,
}

/// Per-thread enumeration context for a [`PbesLps`].
pub struct PbesContext {
    /// The worker's own quantifier-enumerating rewriter.
    rewrite: PbesRewriteContext,

    /// Scratch buffer holding the parameter values of the source state.
    parameter_values: Vec<*const _aterm>,

    /// Scratch buffer for the next state reported to the callback.
    next_state_buf: Vec<usize>,

    /// Scratch worklist for walking the operands of an and/or chain.
    chain_stack: PbesFlattenStack,

    /// The instantiated right-hand side of the last explored state.
    psi: Option<mcrl2::PbesExpression>,

    /// The owner and priority of the last explored state, reported by [`LPS::state_info`].
    player_priority: Option<(Player, Priority)>,
}

// Deliberately not `Send`: `rewrite: PbesRewriteContext` is documented as not `Send` (its
// underlying C++ rewriter is single-threaded), and `crates/merc_pbes/tests/
// pbes_context_send_soundness_test.rs` demonstrated that moving a `PbesContext` across threads
// and using it there crashes the process (the same class of thread-local-bookkeeping crash as
// `PbesSrfContext`, see `explore_srf.rs`). No call site needs `Send` here either — see the
// removed `<P::Summand as Summand>::Context: Send` bound in `crates/explore/src/explore.rs`.

impl PbesLps {
    pub fn new(mut pbes: Pbes) -> Result<Self, MercError> {
        pbes.unify_parameters(UNIFY_IGNORE_CE_EQUATIONS, UNIFY_RESET_PARAMETERS)?;

        let equations = pbes.equations();
        let num_equations = equations.len();
        if num_equations == 0 {
            return Err("PBES has no equations".into());
        }

        let num_params = equations[0].variable().parameters().len();
        let is_mu: Vec<bool> = equations.iter().map(|e| e.is_mu()).collect();
        let priorities = compute_priorities(&is_mu);

        // SAFETY: every name is a subterm of an equation of `pbes`, which this
        // explorer retains for as long as the mapping is used.
        let name_to_eq: NameMapping = equations
            .iter()
            .enumerate()
            .map(|(i, eq)| (unsafe { name_key(eq.variable().name().copy()) }, i))
            .collect();
        let name_to_eq = Arc::new(name_to_eq);

        // Raw pointers to the unified parameter variables (all equations share
        // the same list after unify_parameters).
        let process_parameters: Vec<*const _aterm> = equations[0]
            .variable()
            .parameters()
            .iter()
            .map(|v: DataVariable| v.address())
            .collect();

        let value_mapping = Protected::new(ValueMapping::new());
        let subformula_mapping = Protected::new(SubformulaMapping::new());
        let data_spec = pbes.data_specification();

        // Building a rewriter normalises the data specification *in place*: it
        // imports the system-defined sorts and appends their constructors, which
        // reallocates vectors of aterms.
        let rewriter = PbesRewriteContext::from_data_spec(&data_spec)?;

        // Rewrite the initial state before interning it.
        let initial_expr = PbesExpression::from(pbes.initial_state());
        // SAFETY: `initial_expr` owns a protected term read from the live PBES.
        let initial_rewritten = unsafe { rewriter.rewrite_formula(&initial_expr) }?;
        if !is_pbes_propositional_variable_instantiation(&initial_rewritten.copy()) {
            return Err(MercError::from(format!(
                "The initial state does not rewrite to a propositional variable instantiation: {}",
                initial_rewritten.copy()
            )));
        }
        let initial_pvi = PbesPropositionalVariableInstantiationRef::from(initial_rewritten.copy());

        // SAFETY: the name is a subterm of `initial_rewritten`, still in scope.
        let initial_eq_idx = *name_to_eq
            .get(&unsafe { name_key(initial_pvi.name()) })
            .ok_or_else(|| MercError::from(format!("Unknown initial equation: {}", initial_pvi.name())))?;

        let mut initial_state = Vec::with_capacity(1 + num_params);
        initial_state.push(initial_eq_idx);
        for arg in initial_pvi.arguments().iter() {
            // SAFETY: the term is interned into `value_mapping`, a `Protected`
            // container that keeps every interned term live through GC marking
            // for as long as the mapping exists.
            let (idx, _) = value_mapping.insert(unsafe { DataExpressionRef::from_address(arg.address()) });
            initial_state.push(idx);
        }
        drop(rewriter);

        let make_summand = |kind, read_positions, effect| PbesSummand {
            kind,
            value_mapping: value_mapping.handle(),
            subformula_mapping: subformula_mapping.handle(),
            name_to_eq: name_to_eq.clone(),
            read_positions,
            effect,
        };

        let mut summands: Vec<PbesSummand> = Vec::with_capacity(num_equations + 3);

        for (eq_idx, eq) in equations.iter().enumerate() {
            let formula = eq.formula();
            let (read_positions, effect) = formula_positions(&formula, &process_parameters);
            summands.push(make_summand(
                PbesSummandKind::Equation {
                    formula,
                    priority: priorities[eq_idx],
                },
                read_positions,
                effect,
            ));
        }

        // Three summands, fixed in number regardless of PBES size, are appended
        // after the num_equations equation summands: `prepare` dispatches on
        // `state[0]`'s tag bits alone (TAG_MASK), so one shared summand serves
        // every state carrying a given tag rather than one summand per vertex.

        // A sink's only transition is the self-loop, so nothing changes.
        let true_sink_summand = summands.len();
        summands.push(make_summand(
            PbesSummandKind::Sink(TRUE_SINK),
            vec![0],
            OwnedStateEffect::Positions(vec![]),
        ));
        let false_sink_summand = summands.len();
        summands.push(make_summand(
            PbesSummandKind::Sink(FALSE_SINK),
            vec![0],
            OwnedStateEffect::Positions(vec![]),
        ));
        // A subformula vertex expands into propositional variable instantiations,
        // sinks or further subformula vertices, all of different lengths.
        let subformula_summand = summands.len();
        summands.push(make_summand(
            PbesSummandKind::Subformula,
            vec![0, 1],
            OwnedStateEffect::Opaque,
        ));

        Ok(PbesLps {
            pbes,
            data_spec,
            summands,
            true_sink_summand,
            false_sink_summand,
            subformula_summand,
            initial_state,
            process_parameters,
            num_params,
            value_mapping,
            subformula_mapping,
        })
    }

    /// Only used to size the group in tests; the tool derives the degree from
    /// [`crate::explore_common::symmetry_parameter_basis`] instead, which does
    /// not need a constructed LPS.
    #[cfg(test)]
    pub fn num_params(&self) -> usize {
        self.num_params
    }

    /// The unified data parameters, in state-vector order: entry `i` occupies
    /// state position `1 + i` of a propositional variable instantiation.
    ///
    /// This is the vector [`crate::explore_common::symmetry_parameter_basis`]
    /// returns, since both unify with the same flags, but a caller that permutes
    /// parameter positions should check rather than assume that.
    pub fn parameters(&self) -> Vec<DataVariable> {
        self.pbes.equations()[0].variable().parameters().iter().collect()
    }
}

impl LPS for PbesLps {
    type Value = usize;
    type Label = ();
    type StateInfo = PbesVertex;
    // A PBES has no notion of an action; do not encode a trailing "action" dimension for it.
    const HAS_LABELS: bool = false;
    type Summand = PbesSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.initial_state.clone()
    }

    fn summands(&self) -> &[PbesSummand] {
        &self.summands
    }

    fn create_context(&self) -> PbesContext {
        PbesContext {
            // The data specification was already accepted (and normalised) when
            // this explorer was constructed, so building another rewriter for it
            // cannot fail here.
            rewrite: PbesRewriteContext::from_data_spec(&self.data_spec)
                .expect("the data specification was already accepted during construction"),
            parameter_values: Vec::with_capacity(self.num_params),
            next_state_buf: Vec::with_capacity(1 + self.num_params),
            chain_stack: PbesFlattenStack::new(),
            psi: None,
            player_priority: None,
        }
    }

    fn prepare<'a>(&'a self, context: &mut PbesContext, state: &'a [usize]) -> impl Iterator<Item = usize> + 'a {
        let tag = state[0] & TAG_MASK;
        let summand = if tag == TRUE_SINK {
            self.true_sink_summand
        } else if tag == FALSE_SINK {
            self.false_sink_summand
        } else if tag == AND_OP || tag == OR_OP {
            self.subformula_summand
        } else {
            debug_assert!(tag == 0, "unexpected state tag {tag:#x}");
            let eq_idx = state[0];
            let PbesSummandKind::Equation { formula, priority } = &self.summands[eq_idx].kind else {
                panic!("an untagged state[0] must be the index of an equation summand");
            };

            context.parameter_values.clear();
            for &vi in &state[1..=self.num_params] {
                context.parameter_values.push(
                    self.value_mapping
                        .get_by_index(vi)
                        .expect("parameter value must be in mapping")
                        .address(),
                );
            }

            // SAFETY: `process_parameters` and `parameter_values` are live term
            // pointers from the global pool; the rewriter produces a protected
            // result that is immediately stored in `context.psi`.
            let psi = unsafe {
                context
                    .rewrite
                    .set_assignments(&self.process_parameters, &context.parameter_values);
                context.rewrite.rewrite_formula(formula)
            }
            .expect("the rewriter cannot evaluate the right-hand side of this equation");

            let player = player_of(&psi);
            context.psi = Some(psi);
            context.player_priority = Some((player, Priority::new(*priority)));

            eq_idx
        };

        iter::once(summand)
    }

    fn state_info(&self, state: &[usize], context: &PbesContext) -> PbesVertex {
        let tag = state[0] & TAG_MASK;
        if tag == TRUE_SINK {
            PbesVertex::new(Player::Even, Priority::new(0), PbesVertexKind::Sink)
        } else if tag == FALSE_SINK {
            PbesVertex::new(Player::Odd, Priority::new(1), PbesVertexKind::Sink)
        } else if tag == AND_OP {
            PbesVertex::new(
                Player::Odd,
                Priority::new(SUBFORMULA_PRIORITY),
                PbesVertexKind::Subformula,
            )
        } else if tag == OR_OP {
            PbesVertex::new(
                Player::Even,
                Priority::new(SUBFORMULA_PRIORITY),
                PbesVertexKind::Subformula,
            )
        } else {
            let (player, priority) = context
                .player_priority
                .expect("prepare must be called before state_info");
            PbesVertex::instantiation(player, priority)
        }
    }
}

impl ParameterLayoutLPS for PbesLps {
    fn parameter_range(&self, state: &[usize]) -> Option<Range<usize>> {
        // Every tagged state (sink or subformula vertex) stores something other
        // than parameters; an untagged state[0] is an equation index.
        if state[0] & TAG_MASK == 0 {
            debug_assert_eq!(state.len(), 1 + self.num_params);
            Some(1..1 + self.num_params)
        } else {
            None
        }
    }
}

impl PbesSummand {
    fn tables(&self) -> TargetTables<'_> {
        TargetTables {
            name_to_eq: &self.name_to_eq,
            value_mapping: &self.value_mapping,
            subformula_mapping: &self.subformula_mapping,
        }
    }
}

impl Summand for PbesSummand {
    type Value = usize;
    type Label = ();
    type Context = PbesContext;

    fn read_positions(&self) -> &[usize] {
        &self.read_positions
    }

    fn effect(&self) -> StateEffect<'_> {
        self.effect.borrow()
    }

    fn enumerate<F>(&self, context: &mut PbesContext, state: &[usize], mut report: F) -> Result<(), MercError>
    where
        F: FnMut(&(), &[usize]) -> Result<(), MercError>,
    {
        match &self.kind {
            PbesSummandKind::Sink(sink_state) => {
                context.next_state_buf.clear();
                context.next_state_buf.push(*sink_state);
                report(&(), &context.next_state_buf)
            }
            PbesSummandKind::Equation { .. } => {
                let psi = context.psi.as_ref().expect("prepare must precede enumerate");
                let formula = psi.copy();
                enumerate_formula_children(
                    formula,
                    self.tables(),
                    &mut context.chain_stack,
                    &mut context.next_state_buf,
                    &mut report,
                )
            }
            PbesSummandKind::Subformula => {
                let subformula_idx = state[1];
                let pbes_ref = self
                    .subformula_mapping
                    .get_by_index(subformula_idx)
                    .expect("subformula index must be valid");
                let formula = pbes_ref.copy();
                enumerate_formula_children(
                    formula,
                    self.tables(),
                    &mut context.chain_stack,
                    &mut context.next_state_buf,
                    &mut report,
                )
            }
        }
    }
}

/// Returns the player for the given top-level PBES formula.
fn player_of(psi: &mcrl2::PbesExpression) -> Player {
    let r = psi.copy();
    if is_pbes_and(&r) || is_pbes_false(&r) {
        Player::Odd
    } else {
        // OR, PVI, true — player Even (existential / single outgoing edge)
        Player::Even
    }
}

/// Emits successor states for the given PBES formula.
///
/// AND/OR chains are flattened (like mCRL2's `split_and`/`split_or`) so that
/// `(A && B) && (C && D)` emits 4 direct edges instead of 2 subformula vertices.
/// Cross-operator nesting still produces subformula vertices via [`emit_as_target`].
fn enumerate_formula_children<F>(
    formula: PbesExpressionRef<'_>,
    tables: TargetTables<'_>,
    stack: &mut PbesFlattenStack,
    buf: &mut Vec<usize>,
    report: &mut F,
) -> Result<(), MercError>
where
    F: FnMut(&(), &[usize]) -> Result<(), MercError>,
{
    // Only the chain's own operator is flattened: a nested operand of the other
    // one becomes a subformula vertex in `emit_as_target`, and a leaf (a PVI,
    // `true` or `false`) is a chain of one that comes back unchanged.
    let connective = if is_pbes_or(&formula.copy()) {
        PbesConnective::Or
    } else {
        PbesConnective::And
    };

    for leaf in PbesFlattenIter::new(formula, connective, stack) {
        emit_as_target(leaf, tables, buf, report)?;
    }

    Ok(())
}

/// Emits a transition to the parity-game state corresponding to `expr`.
///
/// PVI → concrete PVI state `[eq_idx, arg0, …]`.
/// AND/OR sub-formula → subformula vertex `[AND_OP/OR_OP, subformula_idx]`.
/// `true` / `false` → respective sink state.
///
/// A sub-formula target is always a strict subterm of `expr`'s parent formula,
/// which is what makes the subformula subgraph acyclic and lets those vertices
/// share [`SUBFORMULA_PRIORITY`] regardless of which equation reached them.
///
/// A subformula vertex needs no stored record of its enclosing equation's
/// parameter values: by the time `expr` reaches here it is a subterm of
/// `psi`, the right-hand side [`PbesLps::prepare`] already instantiated with
/// those values, so every parameter reference in it is already substituted
/// away. The interned term is therefore closed — `subformula_idx` alone is
/// enough to re-expand it later.
fn emit_as_target<F>(
    expr: PbesExpressionRef<'_>,
    tables: TargetTables<'_>,
    buf: &mut Vec<usize>,
    report: &mut F,
) -> Result<(), MercError>
where
    F: FnMut(&(), &[usize]) -> Result<(), MercError>,
{
    if is_pbes_propositional_variable_instantiation(&expr.copy()) {
        let pvi = PbesPropositionalVariableInstantiationRef::from(expr);
        let target_eq = *tables
            .name_to_eq
            // SAFETY: the name is a subterm of `expr`, which the caller holds live.
            .get(&unsafe { name_key(pvi.name()) })
            .ok_or_else(|| MercError::from(format!("Unknown equation name in PVI: {}", pvi.name())))?;
        buf.clear();
        buf.push(target_eq);
        for arg in pvi.arguments().iter() {
            // SAFETY: term interned into the Protected value_mapping.
            let (idx, _) = tables
                .value_mapping
                .insert(unsafe { DataExpressionRef::from_address(arg.address()) });
            buf.push(idx);
        }
        report(&(), buf)
    } else if is_pbes_and(&expr.copy()) {
        // SAFETY: term is a sub-expression of the rewritten psi still in context;
        // the subformula_mapping (Protected) keeps it alive via GC marking.
        let (subformula_idx, _) = tables
            .subformula_mapping
            .insert(unsafe { PbesExpressionRef::from_address(expr.address()) });
        buf.clear();
        buf.extend([AND_OP, subformula_idx]);
        report(&(), buf)
    } else if is_pbes_or(&expr.copy()) {
        // SAFETY: term is a sub-expression of the rewritten psi still in context;
        // the subformula_mapping (Protected) keeps it alive via GC marking.
        let (subformula_idx, _) = tables
            .subformula_mapping
            .insert(unsafe { PbesExpressionRef::from_address(expr.address()) });
        buf.clear();
        buf.extend([OR_OP, subformula_idx]);
        report(&(), buf)
    } else if is_pbes_true(&expr.copy()) {
        buf.clear();
        buf.push(TRUE_SINK);
        report(&(), buf)
    } else if is_pbes_false(&expr.copy()) {
        buf.clear();
        buf.push(FALSE_SINK);
        report(&(), buf)
    } else {
        Err(MercError::from(format!(
            "Unexpected PBES formula shape after rewriting: {}",
            expr.copy()
        )))
    }
}

/// Builds a parity game by exploring the given PBES directly (no SRF
/// conversion), using `builder` to accumulate the result - see [`PGBuilder`].
///
/// # Why there is no caching option
///
/// Enumeration caching ([`merc_explore::CacheLPS`]) cannot pay off for this
/// explorer, so it is deliberately not offered; see [`PbesLps`] for the details.
pub fn explore_pbes<B: PGBuilder>(
    pbes: Pbes,
    strategy: ExplorationStrategy,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError> {
    let lps = PbesLps::new(pbes)?;
    explore_pbes_impl(&lps, strategy, timing, builder)
}

/// Builds a parity game by exploring the given PBES directly in parallel,
/// using `builder` to accumulate the result - see [`PGBuilder`].
///
/// Caching is not offered here either, for the reasons given on [`explore_pbes`].
pub fn explore_pbes_parallel<B: PGBuilder>(
    pbes: Pbes,
    threads: usize,
    pinned: bool,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError> {
    let lps = PbesLps::new(pbes)?;
    explore_pbes_parallel_impl(&lps, threads, pinned, timing, builder)
}

/// Computes the read positions and the state effect of an equation summand.
///
/// The effect is [`StateEffect::Positions`] only when the right-hand side is a
/// bare propositional variable instantiation. Such an equation always produces
/// exactly one next state, of the same length as the source, so the written
/// positions describe it exactly.
///
/// Every other shape is [`StateEffect::Opaque`]. It is not enough to look for a
/// syntactic `&&`/`||`: the right-hand side is rewritten with quantifier
/// enumeration before it is explored, and that turns a `forall`/`exists` into an
/// and/or chain (a subformula vertex, length 3) and can collapse a `val(...)` to
/// `true`/`false` (a sink, length 1). Neither has the length of the source state,
/// so no set of write positions can describe them.
///
/// `read_positions` is `{0}` ∪ `{k+1 | param[k]` occurs in `formula}`. Identity
/// arguments (`X(..., d_k, ...)` passing `d_k` straight through) only count under
/// an opaque effect: there the whole next state is captured, so a passed-through
/// value has to be part of the cache key, whereas a positional effect replays it
/// from the live source state.
fn formula_positions(formula: &mcrl2::PbesExpression, params: &[*const _aterm]) -> (Vec<usize>, OwnedStateEffect) {
    // Single-pass visitor: collect read-variable addresses and the write-position
    // mask simultaneously, visiting each PVI argument exactly once.
    struct FormulaPositions<'p> {
        /// Variables occurring anywhere other than as an identity PVI argument.
        var_addrs: HashSet<*const _aterm>,
        /// Parameters passed straight through by some PVI argument.
        identity_var_addrs: HashSet<*const _aterm>,
        /// Parameter positions some PVI argument writes a non-identity value to.
        write_mask: Vec<bool>,
        params: &'p [*const _aterm],
    }

    impl PbesExpressionVisitor for FormulaPositions<'_> {
        fn visit_propositional_variable_instantiation(
            &mut self,
            inst: &PbesPropositionalVariableInstantiationRef<'_>,
        ) -> Option<mcrl2::PbesExpression> {
            for (k, (arg, &param_addr)) in inst.arguments().iter().zip(self.params.iter()).enumerate() {
                if is_variable(&arg.copy()) && arg.address() == param_addr {
                    self.identity_var_addrs.insert(param_addr);
                    continue;
                }
                for v in variable_occurrences_data_expression(&arg.copy()) {
                    self.var_addrs.insert(v.address());
                }
                self.write_mask[k] = true;
            }
            None
        }

        fn visit_data_expression(&mut self, expr: &DataExpressionRef<'_>) -> Option<DataExpression> {
            for v in variable_occurrences_data_expression(expr) {
                self.var_addrs.insert(v.address());
            }
            None
        }
    }

    let mut collector = FormulaPositions {
        var_addrs: HashSet::new(),
        identity_var_addrs: HashSet::new(),
        write_mask: vec![false; params.len()],
        params,
    };
    collector.visit(&formula.copy());

    // Rewriting a propositional variable instantiation rewrites its arguments but
    // cannot change its shape, so this is the one case with a positional effect.
    let is_bare_instantiation = is_pbes_propositional_variable_instantiation(&formula.copy());

    // Position 0 (the equation index) is always read and always written.
    let mut read_positions = vec![0usize];
    for (k, &param_addr) in params.iter().enumerate() {
        let read = collector.var_addrs.contains(&param_addr)
            || (!is_bare_instantiation && collector.identity_var_addrs.contains(&param_addr));
        if read {
            read_positions.push(k + 1);
        }
    }

    let effect = if is_bare_instantiation {
        let mut write_positions = vec![0usize];
        for (k, &written) in collector.write_mask.iter().enumerate() {
            if written {
                write_positions.push(k + 1);
            }
        }
        OwnedStateEffect::Positions(write_positions)
    } else {
        OwnedStateEffect::Opaque
    };

    (read_positions, effect)
}
