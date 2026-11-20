use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use hashbrown::HashMap;
use itertools::Itertools;
use log::info;
use rustc_hash::FxBuildHasher;

use merc_aterm::ATerm;
use merc_aterm::ATermList;
use merc_aterm::ATermSend;
use merc_aterm::Term;
use merc_collections::Odometer;
use merc_collections::VecBag;
use merc_data::DataExpression;
use merc_io::TimeProgress;
use merc_lts::ConcurrentLtsBuilder;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsAction;
use merc_lts::LtsBuilder;
use merc_lts::LtsBuilderMem;
use merc_lts::LtsMultiAction;
use merc_lts::MutexLtsBuilder;
use merc_lts::PerStateDedup;
use merc_lts::SimpleAction;
use merc_lts::StateIndex;
use merc_lts::StateLabels;
use merc_lts::TransitionLabel;
use merc_syntax::CommExpr;
use merc_syntax::MultiActionLabel;
use merc_utilities::MercError;
use merc_utilities::ShardedCounter;
use merc_utilities::Timing;

use crate::ExplorationStrategy;
use crate::LPS;
use crate::StateEffect;
use crate::Summand;
use crate::configure_rayon_thread_pool;
use crate::explore;
use crate::explore_parallel;

/// The trait for labels used in the composition operator.
pub trait CombineLabel: TransitionLabel + Ord {
    /// Returns true iff the data arguments of two actions are compatible for communication.
    /// Defaults to `true` for label types without structured arguments (e.g., `String`).
    fn comm_args_compatible(&self, _other: &Self) -> bool {
        true
    }

    /// Creates a new action as the result of a communication expression with the given `label`.
    /// `representative` is one of the matched input actions and provides argument values.
    fn from_comm(label: String, representative: &Self) -> Self;
}

impl CombineLabel for SimpleAction {
    fn comm_args_compatible(&self, other: &Self) -> bool {
        self.arguments() == other.arguments()
    }

    fn from_comm(label: String, representative: &Self) -> Self {
        SimpleAction::new(label, representative.arguments().to_vec())
    }
}

impl CombineLabel for LtsAction {
    fn comm_args_compatible(&self, other: &Self) -> bool {
        self.arguments() == other.arguments()
    }

    fn from_comm(label: String, representative: &Self) -> Self {
        LtsAction::new(label, representative.arguments().to_vec())
    }
}

/// Computes the parallel composition hide(H, allow(A, comm(C, L1 || ... || Ln))).
///
/// The `builder` is used to construct the resulting LTS, which can also be
/// stored immediately in a file. Returns the result of [`LtsBuilder::finish`].
///
/// # Errors
///
/// Returns an error if `parallel_composition` is empty, if `allow` contains the
/// tau action, or if any expression in `comm` has a tau action or fewer than
/// two actions on its left-hand side, shares a left-hand side action with
/// another expression, or has its right-hand side appear in another
/// expression's left-hand side.
pub fn combine_lts<L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel, B: LtsBuilder<L::Label>>(
    builder: &mut B,
    parallel_composition: Vec<L>,
    hide: &[String],
    allow: &[MultiActionLabel],
    comm: &[CommExpr],
    timing: &Timing,
) -> Result<B::LTS, MercError> {
    check_arguments(&parallel_composition, allow, comm)?;
    let allow = sort_allow(allow);
    let lps = ComposedLts::new(&parallel_composition, hide, &allow, comm);

    // Two different combinations of underlying transitions (e.g. from different LTSs in the
    // composition) can reduce to the same (label, successor) pair for a given state once hiding,
    // allow, and communication are applied. So deduplicate then,
    let mut dedup = PerStateDedup::new();

    let progress = TimeProgress::new(
        |(states, transitions): (usize, usize)| {
            info!("Explored {states} states, {transitions} transitions...");
        },
        1,
    );

    let mut num_of_states = 0;
    let initial = explore(
        &lps,
        ExplorationStrategy::Dfs,
        timing,
        &mut (&mut *builder, &mut dedup),
        |(builder, dedup), _state, _info| {
            num_of_states += 1;
            if num_of_states % 1024 == 0 {
                progress.print((num_of_states, builder.num_of_transitions() + dedup.len()));
            }
            Ok(())
        },
        |(builder, dedup), from, label, to| {
            dedup.add(from, label, to, |from, label, to| {
                builder.add_transition(from, label, to)
            })
        },
    )?;

    dedup.flush(|from, label, to| builder.add_transition(from, label, to))?;

    info!(
        "Composition complete: {} states, {} transitions",
        num_of_states,
        builder.num_of_transitions(),
    );

    builder.require_num_of_states(num_of_states);
    builder.finish(initial)
}

/// Computes the parallel composition like [`combine_lts`], using `threads` worker threads.
///
/// The states of the result are numbered sparsely, i.e. there can be indices without transitions that are not
/// reachable from the initial state.
///
/// # Errors
///
/// See [`combine_lts`].
pub fn combine_lts_parallel<L, A, B>(
    builder: &mut B,
    parallel_composition: Vec<L>,
    hide: &[String],
    allow: &[MultiActionLabel],
    comm: &[CommExpr],
    threads: usize,
    timing: &Timing,
) -> Result<B::LTS, MercError>
where
    L: LTS<Label = LtsMultiAction<A>> + Sync,
    A: CombineLabel + Send + Sync,
    B: ConcurrentLtsBuilder<L::Label>,
{
    check_arguments(&parallel_composition, allow, comm)?;

    let allow = sort_allow(allow);
    let lps = ComposedLts::new(&parallel_composition, hide, &allow, comm);
    let pool = configure_rayon_thread_pool(threads, false)?;

    let builder_ref: &B = builder;
    let states = ShardedCounter::new();
    let transitions = ShardedCounter::new();
    let progress = TimeProgress::new(
        |(states, transitions): (usize, usize)| {
            info!("Explored {states} states, {transitions} transitions...");
        },
        1,
    );

    // Forwards a deduplicated transition into the shared builder, see [`combine_lts`].
    let flush_one = |from, label: &L::Label, to| {
        transitions.increment();
        builder_ref.add_transition_shared(from, label, to)
    };

    let (initial, max_state) = timing.measure("explore", || -> Result<_, MercError> {
        pool.install(|| {
            // Every worker deduplicates the transitions of the states that it explores.
            let (initial, locals) = explore_parallel(
                &lps,
                || (PerStateDedup::new(), 0usize, 0usize),
                |(_, num_of_states, max_state), state, _info| {
                    states.increment();
                    *num_of_states += 1;
                    *max_state = (*max_state).max(state.value());
                    if *num_of_states % 1024 == 0 && progress.is_due() {
                        progress.print((states.get() as usize, transitions.get() as usize));
                    }
                    Ok(())
                },
                |(dedup, _, _), from, label, to| dedup.add(from, label, to, flush_one),
            )?;

            // Every discovered state is explored, so this bounds the source and target of every transition.
            let mut max_state = initial.value();
            for (mut dedup, _, local_max) in locals {
                dedup.flush(flush_one)?;
                max_state = max_state.max(local_max);
            }
            Ok((initial, max_state))
        })
    })?;

    info!(
        "Composition complete: {} states, {} transitions",
        states.get(),
        transitions.get()
    );

    // The state indices are sparse, so the number of states is determined by the largest index.
    builder.require_num_of_states(max_state + 1);
    builder.finish(initial)
}

/// Computes the parallel composition like [`combine_lts_parallel`], for LTSs in the binary LTS format.
///
/// The arguments of their actions are terms that cannot be shared between threads, so the actions are converted to
/// [`SendAction`]s first and converted back in the result. The state labels of the LTSs are not used.
///
/// # Errors
///
/// See [`combine_lts`].
pub fn combine_lts_actions_parallel<S: StateLabels>(
    parallel_composition: Vec<LabelledTransitionSystem<LtsMultiAction<LtsAction>, S>>,
    hide: &[String],
    allow: &[MultiActionLabel],
    comm: &[CommExpr],
    threads: usize,
    timing: &Timing,
) -> Result<LabelledTransitionSystem<LtsMultiAction<LtsAction>>, MercError> {
    let parallel_composition = parallel_composition
        .into_iter()
        .map(|lts| {
            lts.with_state_labels(())
                .relabel(|label| Ok(SendAction::from_multi_action(&label)))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut builder = MutexLtsBuilder::new(LtsBuilderMem::new(Vec::new(), Vec::new()));
    let result = combine_lts_parallel(&mut builder, parallel_composition, hide, allow, comm, threads, timing)?;

    result.relabel(|label| Ok(SendAction::to_multi_action(&label)))
}

/// An [`LtsAction`] whose arguments are stored as a single sendable term list, so that it can be shared between
/// threads.
///
/// Terms are maximally shared, so comparing the argument lists is as cheap as comparing integers. Cloning only
/// increments a reference count, instead of protecting the term again.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SendAction {
    label: String,
    arguments: Arc<ATermSend>,
}

impl SendAction {
    /// Creates an action with the given label and arguments.
    pub fn new(label: String, arguments: &[DataExpression]) -> Self {
        let arguments: ATerm = ATermList::<DataExpression>::from_double_iter(arguments.iter().cloned()).into();
        SendAction {
            label,
            arguments: Arc::new(ATermSend::from(arguments)),
        }
    }

    /// Returns the [`LtsAction`] with the same label and arguments.
    pub fn to_action(&self) -> LtsAction {
        let arguments = ATermList::<DataExpression>::from(self.arguments.protect());
        LtsAction::new(self.label.clone(), arguments.iter().collect())
    }

    /// Returns the multi-action in which every action is converted to a [`SendAction`].
    pub fn from_multi_action(multi_action: &LtsMultiAction<LtsAction>) -> LtsMultiAction<SendAction> {
        LtsMultiAction::new(
            multi_action
                .actions()
                .iter()
                .map(|action| SendAction::new(action.label().to_owned(), action.arguments()))
                .collect(),
        )
    }

    /// Returns the multi-action in which every action is converted back to an [`LtsAction`].
    pub fn to_multi_action(multi_action: &LtsMultiAction<SendAction>) -> LtsMultiAction<LtsAction> {
        LtsMultiAction::new(multi_action.actions().iter().map(SendAction::to_action).collect())
    }
}

impl fmt::Display for SendAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_action(), f)
    }
}

impl fmt::Debug for SendAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_action(), f)
    }
}

impl TransitionLabel for SendAction {
    fn is_tau_label(&self) -> bool {
        false
    }

    fn tau_label() -> Self {
        panic!("SendAction does not support tau labels. Use LtsMultiAction for that.")
    }

    fn matches_label(&self, label: &str) -> bool {
        self.label == label
    }

    fn from_index(i: usize) -> Self {
        SendAction::new(LtsAction::from_index(i).label().to_owned(), &[])
    }
}

impl CombineLabel for SendAction {
    fn comm_args_compatible(&self, other: &Self) -> bool {
        self.arguments == other.arguments
    }

    fn from_comm(label: String, representative: &Self) -> Self {
        SendAction {
            label,
            arguments: representative.arguments.clone(),
        }
    }
}

/// Checks the arguments of [`combine_lts`], see its documentation.
fn check_arguments<L>(
    parallel_composition: &[L],
    allow: &[MultiActionLabel],
    comm: &[CommExpr],
) -> Result<(), MercError> {
    if parallel_composition.is_empty() {
        return Err("At least one LTS is required for composition.".into());
    }

    if allow.iter().any(|allow| allow.is_tau_label()) {
        return Err("Allow set cannot contain the tau action.".into());
    }

    check_communications(comm)
}

/// Checks the restrictions on the communication expressions that [`combine_lts`] documents.
fn check_communications(comm: &[CommExpr]) -> Result<(), MercError> {
    for (i, comm_i) in comm.iter().enumerate() {
        if comm_i.from.is_tau_label() {
            return Err("Communication expressions cannot have tau actions on the left-hand side.".into());
        }

        if comm_i.from.actions.len() < 2 {
            return Err(format!(
                "Communication expressions must have at least two actions on the left-hand side, but {comm_i} does not."
            )
            .into());
        }

        for comm_j in comm
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, comm_j)| comm_j)
        {
            // The left hand side of a communication cannot overlap with any other communication's left hand side. This
            // is the definition used in mCRL2, for example a|b -> a is a valid communication expression.
            if comm_i.from.actions.iter().any(|a| comm_j.from.actions.contains(a)) {
                return Err(format!(
                    "Communication expressions cannot have overlapping left-hand sides, i.e. {comm_i} and {comm_j}."
                )
                .into());
            }

            // The right hand side of a communication cannot be in another communication's left hand side.
            if comm_j.from.actions.contains(&comm_i.to) {
                return Err(
                    format!("Communication expressions cannot have right-hand side in another communication's left-hand side, i.e. {comm_i} and {comm_j}.").into(),
                );
            }
        }
    }

    Ok(())
}

/// The parallel composition hide(H, allow(A, comm(C, L1 || ... || Ln))) as an [`LPS`], so that it can be explored by
/// [`mod@crate::explore`].
///
/// The state vector consists of one state per LTS. There is one summand for every non-empty subset of the LTSs, which
/// takes one transition in each LTS of the subset simultaneously and leaves the other LTSs unchanged.
struct ComposedLts<'a, L> {
    lts_list: &'a [L],
    hide: &'a [String],
    allow: &'a [SortedMultiActionLabel],
    comm: &'a [CommExpr],

    summands: Vec<ComposedSummand<'a, L>>,
}

impl<'a, L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel> ComposedLts<'a, L> {
    fn new(lts_list: &'a [L], hide: &'a [String], allow: &'a [SortedMultiActionLabel], comm: &'a [CommExpr]) -> Self {
        Self {
            lts_list,
            hide,
            allow,
            comm,
            summands: non_empty_subsets(lts_list.len())
                .map(|components| ComposedSummand {
                    components,
                    lts: PhantomData,
                })
                .collect(),
        }
    }
}

impl<'a, L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel> LPS for ComposedLts<'a, L> {
    type Value = usize;
    type Label = LtsMultiAction<A>;
    type StateInfo = ();
    type Summand = ComposedSummand<'a, L>;

    fn initial_state(&self) -> Vec<usize> {
        self.lts_list
            .iter()
            .map(|lts| lts.initial_state_index().value())
            .collect()
    }

    fn summands(&self) -> &[Self::Summand] {
        &self.summands
    }

    fn create_context(&self) -> ComposedContext<'a, L> {
        ComposedContext {
            labels: CombinedLabels::new(self.lts_list, self.hide, self.allow, self.comm),
            outgoing_transitions: vec![Vec::new(); self.lts_list.len()],
            odometer: Odometer::new(),
            key: Vec::new(),
            target: Vec::new(),
        }
    }

    fn prepare<'b>(
        &'b self,
        context: &mut ComposedContext<'a, L>,
        state: &'b [usize],
    ) -> impl Iterator<Item = usize> + 'b {
        // Collect the outgoing transitions of every LTS once, so that the summands can share them.
        for (i, (lts, outgoing)) in self.lts_list.iter().zip(&mut context.outgoing_transitions).enumerate() {
            outgoing.clear();
            outgoing.extend(
                lts.outgoing_transitions(StateIndex::new(state[i]))
                    .map(|transition| (transition.label, transition.to.value())),
            );
        }

        // A summand in which an LTS cannot move has no transitions, see [`ComposedSummand::enumerate`].
        0..self.summands.len()
    }

    fn state_info(&self, _state: &[usize], _context: &ComposedContext<'a, L>) {}
}

/// The summand of a [`ComposedLts`] in which the LTSs in `components` take a transition simultaneously.
struct ComposedSummand<'a, L> {
    /// The indices of the LTSs that move, in increasing order.
    components: Vec<usize>,
    lts: PhantomData<&'a [L]>,
}

/// Returns every non-empty subset of `0..n` as a list in increasing order. The subsets are ordered like the binary
/// numbers 1 to 2^n - 1, in which element 0 is the least significant bit.
fn non_empty_subsets(n: usize) -> impl Iterator<Item = Vec<usize>> {
    let mut odometer = Odometer::new();
    odometer.reset(std::iter::repeat_n(2, n));

    // The last digit changes fastest, so it corresponds to element 0.
    std::iter::from_fn(move || {
        odometer.increment()?;
        Some(odometer.digits().iter().rev().positions(|&digit| digit == 1).collect())
    })
}

impl<'a, L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel> Summand for ComposedSummand<'a, L> {
    type Value = usize;
    type Label = LtsMultiAction<A>;
    type Context = ComposedContext<'a, L>;

    /// Enumerates the transitions in which every LTS in `components` takes one of its outgoing transitions, and the
    /// other LTSs stay in their state. The odometer holds the index of the outgoing transition of every component,
    /// and only the components from the first changed digit onwards are updated.
    fn enumerate<F>(&self, context: &mut Self::Context, state: &[usize], mut report: F) -> Result<(), MercError>
    where
        F: FnMut(&Self::Label, &[usize]) -> Result<(), MercError>,
    {
        let ComposedContext {
            labels,
            outgoing_transitions,
            odometer,
            key,
            target,
        } = context;

        if !odometer.reset(self.components.iter().map(|&i| outgoing_transitions[i].len())) {
            return Ok(());
        }
        key.clear();
        key.extend(self.components.iter().map(|&i| (i, LabelIndex::new(0))));
        target.clear();
        target.extend_from_slice(state);

        let mut first_changed = Some(0);
        while let Some(first) = first_changed {
            for (k, &i) in self.components.iter().enumerate().skip(first) {
                let (label, to) = outgoing_transitions[i][odometer.digits()[k]];
                key[k] = (i, label);
                target[i] = to;
            }

            if let Some(label) = labels.label(key) {
                report(label, target)?;
            }
            first_changed = odometer.increment();
        }
        Ok(())
    }

    fn read_positions(&self) -> &[usize] {
        &self.components
    }

    fn effect(&self) -> StateEffect<'_> {
        StateEffect::Positions(&self.components)
    }
}

/// The per-thread enumeration context of a [`ComposedLts`].
struct ComposedContext<'a, L: LTS> {
    labels: CombinedLabels<'a, L>,

    /// The outgoing (label, target) pairs of every LTS in the current state, filled by [`LPS::prepare`].
    outgoing_transitions: Vec<Vec<(LabelIndex, usize)>>,

    /// The buffers of [`ComposedSummand::enumerate`], kept to avoid allocating them for every summand: the odometer
    /// over the outgoing transitions, the (LTS index, label index) pair of every LTS that moves, and the target state.
    odometer: Odometer,
    key: Vec<(usize, LabelIndex)>,
    target: Vec<usize>,
}

/// Computes the labels of parallel transitions, i.e. $\tau_I(\gamma_C(\alpha_{j_0} | \cdots | \alpha_{j_m}))$ when
/// it is allowed by $A$, where LTS $j_k$ takes a transition labelled $\alpha_{j_k}$.
///
/// This label only depends on the label indices taken by the participating LTSs, so it is computed once for every
/// combination of (LTS, label index) pairs. A combination is only stored when it occurs in a parallel transition, so
/// there are at most as many entries as generated parallel transitions, and in practice far fewer.
struct CombinedLabels<'a, L: LTS> {
    lts_list: &'a [L],
    hide: &'a [String],
    allow: &'a [SortedMultiActionLabel],
    comm: &'a [CommExpr],

    /// The label for every combination of (LTS index, label index) pairs, or `None` if it is not allowed.
    cache: HashMap<Vec<(usize, LabelIndex)>, Option<L::Label>, FxBuildHasher>,
}

impl<'a, L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel> CombinedLabels<'a, L> {
    fn new(lts_list: &'a [L], hide: &'a [String], allow: &'a [SortedMultiActionLabel], comm: &'a [CommExpr]) -> Self {
        Self {
            lts_list,
            hide,
            allow,
            comm,
            cache: HashMap::default(),
        }
    }

    /// Returns the label of the parallel transition in which the LTSs take the labels of the given (LTS index, label
    /// index) pairs, in increasing order of LTS index, or `None` if it is not allowed.
    fn label(&mut self, key: &[(usize, LabelIndex)]) -> Option<&LtsMultiAction<A>> {
        let Self {
            lts_list,
            hide,
            allow,
            comm,
            cache,
        } = self;
        cache
            .entry_ref(key)
            .or_insert_with(|| combined_label(lts_list, key, hide, allow, comm))
            .as_ref()
    }

    /// Returns the number of distinct combinations of labels seen so far.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.cache.len()
    }
}

/// Returns $\tau_I(\gamma_C(\alpha))$, where $\alpha$ is the multi-action of the given (LTS index, label index) pairs,
/// or `None` if $\gamma_C(\alpha)$ is not allowed.
fn combined_label<L: LTS<Label = LtsMultiAction<A>>, A: CombineLabel>(
    lts_list: &[L],
    labels: &[(usize, LabelIndex)],
    hide: &[String],
    allow: &[SortedMultiActionLabel],
    comm: &[CommExpr],
) -> Option<LtsMultiAction<A>> {
    // Apply communication to alpha = alpha_{j_0} | ... | alpha_{j_m}.
    let multi_action = communicate(
        comm,
        labels
            .iter()
            .flat_map(|&(lts_idx, label_idx)| lts_list[lts_idx].labels()[label_idx].actions())
            .cloned()
            .collect(),
    );

    // Check allow: alpha in A ∪ {tau}.
    if !is_allowed(allow, &multi_action) {
        return None;
    }

    // Apply hide: alpha = tau_I(alpha).
    Some(hide_action(hide, multi_action))
}

/// Applies the communication operator $\gamma_C$ to the multi-action consisting of `actions`.
///
/// For each communication expression $a_1 | \cdots | a_n \rightarrow c$ in $C$,
/// repeatedly finds matching sub-multisets of actions (with equal arguments)
/// and replaces them with the result action $c$.
fn communicate<L: CombineLabel>(comm: &[CommExpr], mut actions: Vec<L>) -> LtsMultiAction<L> {
    for expr in comm {
        // Try to find a matching sub-multiset for this communication expression.
        while let Some(replacement) = find_communication_match(&actions, expr) {
            actions = replacement;
        }
    }

    LtsMultiAction::new(VecBag::from_vec(actions))
}

/// Tries to find actions matching a single communication expression
/// $a_1 | \cdots | a_n \rightarrow c$ within the given action list.
///
/// Returns the resulting action list with the matched actions replaced by the
/// communicated action, or `None` if no match was found.
fn find_communication_match<L: CombineLabel>(actions: &[L], expr: &CommExpr) -> Option<Vec<L>> {
    // For each action name in the communication expression's left-hand side,
    // find a matching action with the same label. All matched actions must
    // have the same arguments.
    //
    // We iterate over every candidate for the *first* required name so that we
    // do not greedily commit to a particular argument set.  Once the first
    // action is chosen its arguments fix the constraint for all remaining
    // names, so a single greedy pass over the rest is correct.
    let mut matched = vec![false; actions.len()];
    let first_name = expr.from.actions.first()?;

    'outer: for first_idx in 0..actions.len() {
        if !actions[first_idx].matches_label(first_name) {
            continue;
        }

        // Attempt to complete the match using actions[first_idx] as the
        // argument reference.
        matched.fill(false);
        matched[first_idx] = true;

        for required_name in expr.from.actions.iter().skip(1) {
            let mut found = false;
            for (i, action) in actions.iter().enumerate() {
                if matched[i] {
                    continue;
                }
                if action.matches_label(required_name) && action.comm_args_compatible(&actions[first_idx]) {
                    matched[i] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                continue 'outer;
            }
        }

        // Build the result: remove matched actions and add the communicated action.
        let mut result: Vec<L> = actions
            .iter()
            .enumerate()
            .filter(|(i, _)| !matched[*i])
            .map(|(_, a)| a.clone())
            .collect();

        result.push(L::from_comm(expr.to.node.clone(), &actions[first_idx]));
        return Some(result);
    }

    None
}

/// Returns true iff the given multi-action is allowed by the allow operator.
///
/// A multi-action is allowed if:
/// - The action is tau (always allowed), or
/// - The action label names, compared as sorted multisets, match one of the
///   entries in the allow set.
///
/// The actions of a multi-action are iterated in `L`'s sorted order, so this
/// only works if `L`'s `Ord` implementation orders by label name first.
fn is_allowed<L: CombineLabel>(allow: &[SortedMultiActionLabel], action: &LtsMultiAction<L>) -> bool {
    if action.is_tau_label() {
        return true;
    }

    let actions = action.actions();
    debug_assert!(
        actions.iter().zip(actions.iter().sorted()).all(|(a, b)| a == b),
        "LtsMultiAction's internal order must be consistent with the label-matching logic"
    );

    allow.iter().any(|allowed| {
        allowed.actions.len() == actions.len()
            && allowed
                .actions
                .iter()
                .zip(actions.iter())
                .all(|(name, a)| a.matches_label(name))
    })
}

/// Applies the hide operator $\tau_I$ to a multi-action.
///
/// Removes all actions whose label is in the hide set $I$. If all actions are
/// hidden the result is the tau action (empty multi-action).
fn hide_action<L: CombineLabel>(hide: &[String], mut action: LtsMultiAction<L>) -> LtsMultiAction<L> {
    if !hide.is_empty() {
        action.retain(|a| !hide.iter().any(|h| a.matches_label(h)));
    }
    action
}

/// A multi-action label with action names sorted, for proper multiset
/// comparison in [`is_allowed`].
struct SortedMultiActionLabel {
    /// Action names in sorted order.
    actions: Vec<String>,
}

impl SortedMultiActionLabel {
    fn new(label: &MultiActionLabel) -> Self {
        let mut actions: Vec<String> = label.actions.iter().map(|name| name.node.clone()).collect();
        actions.sort();
        SortedMultiActionLabel { actions }
    }
}

/// Returns the allow set with the names of every multi-action sorted, see [`is_allowed`].
fn sort_allow(allow: &[MultiActionLabel]) -> Vec<SortedMultiActionLabel> {
    allow.iter().map(SortedMultiActionLabel::new).collect()
}

#[cfg(test)]
mod tests {
    use merc_collections::VecBag;
    use merc_lts::LtsMultiAction;
    use merc_lts::SimpleAction;
    use merc_syntax::CommExpr;
    use merc_syntax::MultiActionLabel;
    use merc_syntax::Span;
    use merc_syntax::respan;

    use itertools::Itertools;

    use merc_lts::LTS;
    use merc_lts::LabelIndex;
    use merc_lts::LabelledTransitionSystem;
    use merc_lts::StateIndex;
    use merc_lts::TransitionLabel;
    use merc_lts::random_lts;
    use merc_syntax::parse_action_names;
    use merc_syntax::parse_allow_action_names;
    use merc_syntax::parse_comm_expr_set;
    use merc_utilities::random_test;

    use super::CombinedLabels;
    use super::ComposedLts;
    use super::SortedMultiActionLabel;
    use super::combined_label;
    use super::find_communication_match;
    use super::is_allowed;
    use super::non_empty_subsets;
    use super::sort_allow;
    use crate::LPS;
    use crate::Summand;

    type TestLts = LabelledTransitionSystem<LtsMultiAction<SimpleAction>>;

    /// An LTS with the given labels after tau (label index 0), in which the transitions are (from, label index, to).
    fn make_lts(labels: &[&str], num_of_states: usize, transitions: &[(usize, usize, usize)]) -> TestLts {
        let labels = std::iter::once(LtsMultiAction::tau_label())
            .chain(labels.iter().map(|label| LtsMultiAction::from_string(label).unwrap()))
            .collect();
        LabelledTransitionSystem::new(
            StateIndex::new(0),
            Some(num_of_states),
            || {
                transitions
                    .iter()
                    .map(|&(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)))
            },
            labels,
        )
    }

    /// A random LTS with multi-action labels a, b, c, ... and tau.
    fn random_test_lts(rng: &mut impl rand::Rng, num_of_states: usize, num_of_labels: u32) -> TestLts {
        random_lts::<String, _>(rng, num_of_states, num_of_labels)
            .relabel(|label| LtsMultiAction::from_string(&label))
            .unwrap()
    }

    /// The (LTS index, label index) pairs as a key of [`CombinedLabels`].
    fn key(pairs: &[(usize, usize)]) -> Vec<(usize, LabelIndex)> {
        pairs
            .iter()
            .map(|&(lts, label)| (lts, LabelIndex::new(label)))
            .collect()
    }

    /// Returns the transitions of `lps` from `state` as (label, target) pairs.
    fn enumerate_all(
        lps: &ComposedLts<'_, TestLts>,
        state: &[usize],
    ) -> Vec<(LtsMultiAction<SimpleAction>, Vec<usize>)> {
        let mut context = lps.create_context();
        let summands: Vec<usize> = lps.prepare(&mut context, state).collect();
        let mut result = Vec::new();
        for index in summands {
            lps.summands()[index]
                .enumerate(&mut context, state, |label, target| {
                    result.push((label.clone(), target.to_vec()));
                    Ok(())
                })
                .unwrap();
        }
        result
    }

    fn simple(label: &str, args: &[&str]) -> SimpleAction {
        SimpleAction::new(label.to_owned(), args.iter().map(|s| s.to_string()).collect())
    }

    fn comm(from: &[&str], to: &str) -> CommExpr {
        CommExpr::new(
            MultiActionLabel::new(from.iter().map(|s| respan(Span::default(), s.to_string())).collect()),
            respan(Span::default(), to.to_owned()),
        )
    }

    /// Builds the allow set from label-name multisets.
    fn allow(entries: &[&[&str]]) -> Vec<SortedMultiActionLabel> {
        entries
            .iter()
            .map(|names| {
                SortedMultiActionLabel::new(&MultiActionLabel::new(
                    names.iter().map(|s| respan(Span::default(), s.to_string())).collect(),
                ))
            })
            .collect()
    }

    /// Builds a multi-action from actions; `VecBag` orders them internally.
    fn multi_action(actions: Vec<SimpleAction>) -> LtsMultiAction<SimpleAction> {
        LtsMultiAction::new(VecBag::from_vec(actions))
    }

    fn is_allowed_by(entries: &[&[&str]], actions: Vec<SimpleAction>) -> bool {
        is_allowed(&allow(entries), &multi_action(actions))
    }

    /// The allow operator matches on label-name multisets only, independent of
    /// arguments. This locks in the name-primary ordering that [`is_allowed`]
    /// relies on: here `a`'s argument sorts *after* `b`'s, so an argument-first
    /// ordering of the bag would misalign the zip and wrongly reject `{a, b}`.
    #[test]
    fn allow_matches_multiset_ignoring_arguments() {
        // Arguments chosen so an argument-first sort would reorder the names.
        assert!(is_allowed_by(
            &[&["a", "b"]],
            vec![simple("a", &["9"]), simple("b", &["1"])]
        ));

        // Repeated names compare as a multiset, not a set.
        assert!(is_allowed_by(
            &[&["a", "a", "b"]],
            vec![simple("a", &["1"]), simple("a", &["2"]), simple("b", &[])]
        ));
        assert!(!is_allowed_by(
            &[&["a", "b", "b"]],
            vec![simple("a", &["1"]), simple("a", &["2"]), simple("b", &[])]
        ));

        // A differing length or name is rejected; tau is always allowed.
        assert!(!is_allowed_by(
            &[&["a", "b"]],
            vec![simple("a", &[]), simple("b", &[]), simple("b", &[])]
        ));
        assert!(!is_allowed_by(&[&["a", "c"]], vec![simple("a", &[]), simple("b", &[])]));
        assert!(is_allowed_by(&[&["a", "b"]], vec![]));
    }

    /// Regression: when the first candidate for the first required name has
    /// incompatible args, the function must try subsequent candidates.
    ///
    /// Multi-action: a(1) | a(2) | b(2)
    /// Comm: a | b -> c
    ///
    /// Greedy first-pick would choose a(1), then fail to match b because
    /// b(2) has different args.  The correct match is a(2) | b(2) -> c(2).
    #[test]
    fn first_candidate_incompatible_retries() {
        let actions = vec![simple("a", &["1"]), simple("a", &["2"]), simple("b", &["2"])];
        let expr = comm(&["a", "b"], "c");

        let result = find_communication_match(&actions, &expr);
        assert!(result.is_some(), "should find a(2)|b(2) -> c(2)");

        let result = result.unwrap();
        // Remaining multi-action: a(1) and c(2)  (a(2) and b(2) consumed)
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|a| a == &simple("a", &["1"])));
        assert!(result.iter().any(|a| a == &simple("c", &["2"])));
    }

    /// When all candidates are genuinely incompatible, the function should
    /// still return `None`.
    #[test]
    fn no_compatible_pair_returns_none() {
        let actions = vec![simple("a", &["1"]), simple("b", &["2"])];
        let expr = comm(&["a", "b"], "c");

        assert!(find_communication_match(&actions, &expr).is_none());
    }

    /// Basic case with no data args still works.
    #[test]
    fn no_args_basic_comm() {
        let actions = vec![simple("a", &[]), simple("b", &[])];
        let expr = comm(&["a", "b"], "c");

        let result = find_communication_match(&actions, &expr).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], simple("c", &[]));
    }

    /// A summand moves only its components, the last one fastest, and the other LTSs stay in their state. A summand in
    /// which a component cannot move has no transitions.
    #[test]
    fn summand_enumerates_outgoing_transitions_of_components() {
        let ltss = vec![
            make_lts(&["a", "b"], 3, &[(0, 1, 1), (0, 2, 2)]),
            make_lts(&["c"], 2, &[(0, 1, 1)]),
            make_lts(&["d", "e"], 3, &[(0, 1, 1), (0, 2, 2)]),
        ];
        let allow = sort_allow(&parse_allow_action_names("{a|d, a|e, b|d, b|e}").unwrap());
        let lps = ComposedLts::new(&ltss, &[], &allow, &[]);

        // The summand of the subset {0, 2}, see [`non_empty_subsets`].
        let summand = &lps.summands()[4];
        assert_eq!(summand.read_positions(), &[0, 2]);

        let mut context = lps.create_context();
        assert_eq!(lps.prepare(&mut context, &[0, 0, 0]).count(), 7);
        let mut result = Vec::new();
        summand
            .enumerate(&mut context, &[0, 0, 0], |label, target| {
                result.push((label.to_string(), target.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            result,
            vec![
                ("a|d".to_string(), vec![1, 0, 1]),
                ("a|e".to_string(), vec![1, 0, 2]),
                ("b|d".to_string(), vec![2, 0, 1]),
                ("b|e".to_string(), vec![2, 0, 2]),
            ]
        );

        // LTS 0 cannot move in state 1.
        assert_eq!(lps.prepare(&mut context, &[1, 0, 0]).count(), 7);
        let mut result = Vec::new();
        summand
            .enumerate(&mut context, &[1, 0, 0], |label, target| {
                result.push((label.to_string(), target.to_vec()));
                Ok(())
            })
            .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn non_empty_subsets_counts_in_binary() {
        assert_eq!(non_empty_subsets(0).collect_vec(), Vec::<Vec<usize>>::new());
        assert_eq!(
            non_empty_subsets(3).collect_vec(),
            vec![
                vec![0],
                vec![1],
                vec![0, 1],
                vec![2],
                vec![0, 2],
                vec![1, 2],
                vec![0, 1, 2]
            ]
        );
    }

    /// Equal label indices in different LTSs must not share a cache entry.
    #[test]
    fn combined_labels_distinguishes_ltss() {
        let ltss = vec![make_lts(&["a"], 2, &[(0, 1, 1)]), make_lts(&["b"], 2, &[(0, 1, 1)])];
        let allow = sort_allow(&parse_allow_action_names("{a, a|b}").unwrap());
        let mut labels = CombinedLabels::new(&ltss, &[], &allow, &[]);

        assert_eq!(
            labels.label(&key(&[(0, 1)])),
            Some(&LtsMultiAction::from_string("a").unwrap())
        );
        assert_eq!(labels.label(&key(&[(1, 1)])), None);
        assert_eq!(
            labels.label(&key(&[(0, 1), (1, 1)])),
            Some(&LtsMultiAction::from_string("a|b").unwrap())
        );
        assert_eq!(labels.len(), 3);
    }

    /// Labels that are not allowed are cached as well, and a combination is only computed once.
    #[test]
    fn combined_labels_caches_blocked_combinations() {
        let ltss = vec![make_lts(&["a", "b"], 1, &[]), make_lts(&["b"], 1, &[])];
        let comm = parse_comm_expr_set("{a|b -> c}").unwrap();
        let allow = sort_allow(&parse_allow_action_names("{c}").unwrap());
        let hide = parse_action_names("{c}").unwrap();
        let mut labels = CombinedLabels::new(&ltss, &hide, &allow, &comm);

        for _ in 0..2 {
            // b|b is not allowed, a|b communicates to c, which is allowed and hidden.
            assert_eq!(labels.label(&key(&[(0, 2), (1, 1)])), None);
            assert_eq!(
                labels.label(&key(&[(0, 1), (1, 1)])),
                Some(&LtsMultiAction::tau_label())
            );
        }
        assert_eq!(labels.len(), 2);
    }

    /// The transitions of the composition are exactly the combinations of outgoing transitions, in which every LTS
    /// either stays or takes a transition, whose label computed without the cache is allowed.
    #[test]
    #[cfg_attr(miri, ignore)] // Ignored under Miri due to potential long execution time
    fn random_composition_matches_naive_enumeration() {
        random_test(100, |rng| {
            let ltss = vec![
                random_test_lts(rng, 5, 5),
                random_test_lts(rng, 5, 5),
                random_test_lts(rng, 5, 5),
            ];
            let comm = parse_comm_expr_set("{a|b -> c}").unwrap();
            // b is only allowed through the communication, and d is never allowed.
            let allow = sort_allow(&parse_allow_action_names("{a, c, a|c}").unwrap());
            let hide = parse_action_names("{a}").unwrap();
            let lps = ComposedLts::new(&ltss, &hide, &allow, &comm);

            for state in ltss.iter().map(|lts| 0..lts.num_of_states()).multi_cartesian_product() {
                // Every LTS either stays (None) or takes one of its outgoing transitions.
                let mut expected = Vec::new();
                let choices: Vec<Vec<Option<(LabelIndex, usize)>>> = ltss
                    .iter()
                    .zip(&state)
                    .map(|(lts, &s)| {
                        std::iter::once(None)
                            .chain(
                                lts.outgoing_transitions(StateIndex::new(s))
                                    .map(|t| Some((t.label, t.to.value()))),
                            )
                            .collect()
                    })
                    .collect();
                for combination in choices
                    .iter()
                    .map(|choice| choice.iter().copied())
                    .multi_cartesian_product()
                {
                    let pairs: Vec<(usize, LabelIndex)> = combination
                        .iter()
                        .enumerate()
                        .filter_map(|(i, choice)| choice.map(|(label, _)| (i, label)))
                        .collect();
                    if pairs.is_empty() {
                        continue;
                    }
                    let target = (0..ltss.len())
                        .map(|i| combination[i].map_or(state[i], |(_, to)| to))
                        .collect();
                    if let Some(label) = combined_label(&ltss, &pairs, &hide, &allow, &comm) {
                        expected.push((label, target));
                    }
                }

                let mut result = enumerate_all(&lps, &state);
                result.sort();
                expected.sort();
                assert_eq!(result, expected);
            }
        });
    }
}
