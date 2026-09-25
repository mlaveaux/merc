use std::ops::ControlFlow;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use log::debug;
use log::info;
use log::trace;
#[cfg(feature = "metrics")]
use merc_io::LargeFormatter;
#[cfg(feature = "metrics")]
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::SaturationEvent;

use merc_data::DataExpression;
use merc_io::TimeProgress;
use merc_lts::TransitionLabel;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::LddDisplay;
use crate::LddLenCache;
use crate::SatCount;
use crate::SymbolicLPS;
use crate::TransitionGroup;
use crate::ldd_len;

/// A symbolic LTS — extends [SymbolicLPS] with LTS-specific metadata.
pub trait SymbolicLTS: SymbolicLPS {
    /// The label type for transitions in this LTS.
    type Label: TransitionLabel;

    /// Returns the LDD representing the set of states.
    fn states(&self) -> &LDDFunction;

    /// Returns the action labels for the LTS.
    fn action_labels(&self) -> &[Self::Label];

    /// Returns the possible values for each process parameter.
    fn parameter_values(&self) -> &[Vec<DataExpression>];
}

/// The order in which transition groups are applied during reachability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum ExplorationStrategy {
    /// Plain breadth-first: every group computes successors of the original frontier.
    #[default]
    BreadthFirst,
    /// Successors found by earlier groups feed into later groups within the same iteration.
    Chaining,
    /// Each group is applied to a fixpoint before moving on to the next on.
    Fixpoint,
    /// Like [Self::Fixpoint], but after each group all earlier groups are re-applied to a fixpoint.
    FixpointChaining,
    /// Ciardo-style node-wise saturation. Every LDD node is brought to a fixed point under the events
    /// confined to its level and below, bottom-up, before it is used as anyone's child.
    Saturation,
}

/// Options controlling [reachability_with_options].
///
/// Build with the `metrics` cargo feature to also log manager node counts and oxidd's own per-op
/// apply-cache counters (calls/queries/hits) at `info` level once per outer iteration/round,
/// unconditionally — there is no separate runtime flag for this.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReachabilityOptions {
    /// The strategy used to apply the transition groups.
    pub strategy: ExplorationStrategy,

    /// Whether to detect and report deadlock states (states without outgoing transitions).
    pub detect_deadlocks: bool,

    /// Whether every transition group caches the domain of its learned relation, so that it only
    /// learns the successors of read projections it has not seen before.
    pub cached: bool,
}

/// The result of a reachability run.
pub struct ReachabilityResult {
    /// The set of reachable states.
    pub states: LDDFunction,

    /// The deadlock states (reachable states with no outgoing transition), or `None` when
    /// [ReachabilityOptions::detect_deadlocks] was not requested.
    ///
    /// If the exploration was stopped early (see [ReachabilityResult::complete]), these are only the
    /// deadlocks among the states that were explored, and saturation finds none.
    pub deadlocks: Option<LDDFunction>,

    /// Whether the fixpoint was reached. This is `false` if the callback of
    /// [reachability_with_callback] stopped the exploration first, in which case
    /// [ReachabilityResult::states] is only an under-approximation of the reachable states.
    pub complete: bool,
}

/// Performs reachability analysis using the given initial state and transitions.
///
/// Uses the default [ReachabilityOptions]; see [reachability_with_options] for strategies and
/// deadlock detection. Returns only the reachable states.
pub fn reachability<L: SymbolicLPS>(
    storage: &LDDManagerRef,
    lts: &mut L,
    timing: &Timing,
) -> Result<LDDFunction, MercError> {
    let mut context = lts.create_context();
    Ok(reachability_with_options(storage, lts, &mut context, &ReachabilityOptions::default(), timing)?.states)
}

/// Performs reachability analysis using the given initial state, transitions and [ReachabilityOptions].
///
/// `context` is created once by the caller (via [`SymbolicLPS::create_context`]) and threaded through
/// every learning call, so that its interned state — e.g. the value/label interning some
/// implementations use — is still available to the caller once reachability finishes.
pub fn reachability_with_options<L: SymbolicLPS>(
    storage: &LDDManagerRef,
    lts: &mut L,
    context: &mut <L::Group as TransitionGroup>::Context,
    options: &ReachabilityOptions,
    timing: &Timing,
) -> Result<ReachabilityResult, MercError> {
    reachability_with_callback(storage, lts, context, options, timing, |_, _| ControlFlow::Continue(()))
}

/// Like [reachability_with_options], but calls `on_iteration` after every iteration of the outer loop.
///
/// The arguments are the number of iterations that have completed so far, starting at one, and the
/// states that have been found up to and including that iteration. What an iteration is depends on
/// [ExplorationStrategy]: for the breadth-first, chaining and fixpoint strategies it is one step over
/// the frontier, and for saturation it is one round of learning relations and saturating.
///
/// If `on_iteration` returns [ControlFlow::Break] the exploration stops, and the states found so far
/// are returned with [ReachabilityResult::complete] set to `false`, unless that iteration had
/// already reached the fixpoint. This allows, for example, to limit the number of iterations or the
/// time spent, or to report on the progress.
pub fn reachability_with_callback<L: SymbolicLPS, F: FnMut(usize, &LDDFunction) -> ControlFlow<()>>(
    storage: &LDDManagerRef,
    lts: &mut L,
    context: &mut <L::Group as TransitionGroup>::Context,
    options: &ReachabilityOptions,
    timing: &Timing,
    mut on_iteration: F,
) -> Result<ReachabilityResult, MercError> {
    if options.strategy == ExplorationStrategy::Saturation {
        return saturation_reachability(storage, lts, context, options, timing, on_iteration);
    }

    let mut todo = lts.initial_state().clone();
    let mut states = lts.initial_state().clone();
    let mut deadlocks: Option<LDDFunction> = if options.detect_deadlocks {
        Some(storage.with_manager_shared(|m| LDDFunction::empty_set(m))?)
    } else {
        None
    };
    let mut iteration = 0;
    let mut complete = true;
    let mut len_cache = LddLenCache::new();

    trace!("states = {}", LddDisplay::new(&states));
    let progress = TimeProgress::new(
        |(iteration, num_of_states)| {
            info!("explored {} state(s) after {} iteration(s)", num_of_states, iteration);
        },
        1,
    );

    // The chaining and saturation strategies compute an entire fixpoint inside a single step, so the
    // progress above can stay silent for a very long time. This one reports the frontier as it grows.
    let step_progress = TimeProgress::new(
        |(group, num_of_states)| {
            info!("found {} todo state(s) up to transition group {}", num_of_states, group);
        },
        10,
    );

    timing.measure("reachability", || {
        while !todo.is_empty() {
            debug!(
                "Iteration {}: todo size = {}",
                iteration,
                ldd_len(&todo, &mut len_cache)
            );

            let (todo1, step_deadlocks) = step(storage, lts, context, &todo, options, timing, &step_progress)?;

            trace!("todo1 = {}", LddDisplay::new(&todo1));

            todo = todo1.minus(&states)?;
            states = states.union(&todo)?;

            if let Some(accumulated) = &mut deadlocks {
                *accumulated = accumulated.union(&step_deadlocks)?;
            }

            #[cfg(feature = "metrics")]
            {
                let nodes = storage.with_manager_shared(|m| m.num_inner_nodes());
                info!(
                    "iteration {iteration}: manager has {} inner node(s)",
                    LargeFormatter(nodes)
                );
                oxidd::ldd::print_stats();
            }

            if progress.is_due() {
                progress.print((iteration, ldd_len(&states, &mut len_cache)));
            }

            iteration += 1;

            if on_iteration(iteration, &states).is_break() {
                // The iteration that is stopped can also be the one that found nothing new.
                complete = todo.is_empty();
                break;
            }
        }

        Ok(ReachabilityResult {
            states,
            deadlocks,
            complete,
        })
    })
}

/// The epoch of the next set of events that is saturated, see [saturation_reachability].
static NEXT_SATURATION_EPOCH: AtomicU32 = AtomicU32::new(0);

/// Returns an epoch that no earlier call of this function returned, until the counter wraps after
/// 2^32 calls.
fn fresh_saturation_epoch() -> u32 {
    NEXT_SATURATION_EPOCH.fetch_add(1, Ordering::Relaxed)
}

/// Performs reachability via repeated [`LDDFunction::saturate`] calls over the
/// growing state set, rather than the whole-set fixpoint schedules the other
/// strategies use.
///
/// **On-the-fly relation learning.**
///
/// Inside a single `saturate` call no new local value can appear: firing only
/// ever installs values already present on the write side of an already-learned
/// relation. New values, and new transitions out of them, only show up between
/// calls, so the driver alternates learning and saturating:
///
/// ```text
/// states = initial; saturated = false
/// loop:
///   changed = for each group: group.learn_successors(context, storage, &states, options.cached)
///   if !changed && saturated: break     // states is closed under exactly these events
///   if changed: epoch = fresh_epoch()   // the events differ from those of the previous saturate call
///   states = states.saturate(&events, epoch)
///   saturated = true
/// ```
///
/// If learning finds nothing new, then `states` (the result of the previous
/// call) is already closed under the current events, so there is nothing left
/// to do and the fixpoint check is free.
///
/// This is a coarse-grained version of the paper's per-local-state `Confirm`
/// (§4): learning is triggered per whole state set per group rather than per
/// single newly-touched local value, so it does strictly more enumeration work
/// than necessary. It is correct and terminating regardless.
///
/// **The saturation cache.**
///
/// `saturate` memoises on node identity, which does not change when a group's relation grows, so a
/// node cached as saturated under an earlier, smaller relation must never be reused. The cache entries
/// are keyed on the `epoch` that is passed to `saturate`, which is a new one whenever any relation
/// grew since the previous call, and the same one otherwise, so that the entries of that call stay
/// usable. Nothing has to be cleared, and the cost of a round does not depend on the size of the
/// manager's caches.
///
/// Epochs are drawn from a counter of the whole process, so that explorations that share a manager
/// (which have different relations) never share an epoch either.
fn saturation_reachability<L: SymbolicLPS, F: FnMut(usize, &LDDFunction) -> ControlFlow<()>>(
    storage: &LDDManagerRef,
    lts: &mut L,
    context: &mut <L::Group as TransitionGroup>::Context,
    options: &ReachabilityOptions,
    timing: &Timing,
    mut on_iteration: F,
) -> Result<ReachabilityResult, MercError> {
    let progress = TimeProgress::new(
        |(iteration, num_of_states)| {
            info!("explored {} state(s) after {} iteration(s)", num_of_states, iteration);
        },
        1,
    );

    timing.measure("reachability", || {
        let mut states = lts.initial_state().clone();
        let mut round: u32 = 0;
        let mut len_cache = LddLenCache::new();
        // Whether `states` is the result of a `saturate` call under the current relations.
        let mut saturated = false;
        let mut complete = true;
        // Identifies the relations that `states` was last saturated under, see `saturate`.
        let mut epoch = fresh_saturation_epoch();

        loop {
            let mut relation_changed = false;
            for group in lts.transition_groups_mut() {
                let before = group.relation().clone();
                group.learn_successors(context, storage, &states, options.cached)?;
                if *group.relation() != before {
                    relation_changed = true;
                }
            }

            if !relation_changed && saturated {
                break;
            }

            // Every cached result depends on the relations, so those of an earlier epoch are of no use
            // once one has grown.
            if relation_changed {
                epoch = fresh_saturation_epoch();
            }

            let events = saturation_events(lts);
            states = states.saturate(&events, epoch)?;
            saturated = true;
            round += 1;

            if progress.is_due() {
                progress.print((round, ldd_len(&states, &mut len_cache)));
            }

            #[cfg(feature = "metrics")]
            {
                let nodes = storage.with_manager_shared(|m| m.num_inner_nodes());
                info!("round {round}: manager has {} inner node(s)", LargeFormatter(nodes));
                oxidd::ldd::print_stats();
            }

            if on_iteration(round as usize, &states).is_break() {
                // Whether `states` is closed under the relations is only known after learning again.
                complete = false;
                break;
            }
        }

        let deadlocks = if !options.detect_deadlocks {
            None
        } else if !complete {
            // The relations only cover the states that were learned from so far, so every state that
            // was not would show up as a deadlock. Nothing is reported instead.
            Some(storage.with_manager_shared(|m| LDDFunction::empty_set(m))?)
        } else {
            let mut candidates = states.clone();
            for group in lts.transition_groups() {
                candidates = remove_states_with_successor(&states, group, &candidates)?;
            }
            Some(candidates)
        };

        Ok(ReachabilityResult {
            states,
            deadlocks,
            complete,
        })
    })
}

/// Builds the [`SaturationEvent`]s used by [`saturation_reachability`] from `lts`'s transition
/// groups, in the same order as [`SymbolicLPS::transition_groups`]. A group that neither reads nor
/// writes any position is skipped: its relation is the identity and it has no well-defined
/// `top`/`bot`, so it contributes nothing to saturation.
fn saturation_events<L: SymbolicLPS>(lts: &L) -> Vec<SaturationEvent> {
    let graph = lts.dependency_graph();

    lts.transition_groups()
        .iter()
        .zip(graph.relations())
        .filter_map(|(group, relation)| {
            let top = relation.top()?;
            let bot = relation.bot()?;

            // `meta_at_top` is the group's meta LDD descended `top` times, so that its own root
            // describes state position `top` (see [`LDDFunction::saturate`]).
            let mut meta_at_top = group.meta().clone();
            for _ in 0..top {
                let (_, down, _) = meta_at_top
                    .node()
                    .expect("a group's meta must have at least `top + 1` levels");
                meta_at_top = down;
            }

            Some(SaturationEvent {
                relation: group.relation().clone(),
                meta_at_top,
                top: top as u32,
                bot: bot as u32,
            })
        })
        .collect()
}

/// Performs a single exploration step from the frontier `todo`.
///
/// Returns `(todo1, deadlocks)`: the states reachable this step (the caller subtracts the already
/// visited states) and, when [ReachabilityOptions::detect_deadlocks] is set, the subset of `todo`
/// with no outgoing transition in any group. The transition relations are learned on the fly.
///
/// For the chaining and saturation strategies a single step is a fixpoint computation that can take
/// arbitrarily long, so `progress` reports `(group, frontier size)` while that fixpoint is computed.
fn step<L: SymbolicLPS>(
    storage: &LDDManagerRef,
    lts: &mut L,
    context: &mut <L::Group as TransitionGroup>::Context,
    todo: &LDDFunction,
    options: &ReachabilityOptions,
    timing: &Timing,
    progress: &TimeProgress<(usize, SatCount)>,
) -> Result<(LDDFunction, LDDFunction), MercError> {
    // We only print a message when this step takes a significant amount of time.
    progress.reset();

    let chaining = matches!(
        options.strategy,
        ExplorationStrategy::Chaining | ExplorationStrategy::FixpointChaining
    );
    let fixpoint = matches!(
        options.strategy,
        ExplorationStrategy::Fixpoint | ExplorationStrategy::FixpointChaining
    );
    let detect_deadlocks = options.detect_deadlocks;

    let groups = lts.transition_groups_mut();

    // Potential deadlocks start as the whole frontier; a state is removed as soon as a group is found
    // that takes it to a successor. Only tracked when requested.
    let mut deadlocks = if detect_deadlocks {
        todo.clone()
    } else {
        storage.with_manager_shared(|m| LDDFunction::empty_set(m))?
    };

    if !fixpoint {
        // Regular breadth-first, or chaining where successors found by earlier groups feed later groups.
        let mut todo1 = if chaining {
            todo.clone()
        } else {
            storage.with_manager_shared(|m| LDDFunction::empty_set(m))?
        };

        for (i, transition) in groups.iter_mut().enumerate() {
            trace!("Learning successors for transition group {}:", i);
            let source = if chaining { todo1.clone() } else { todo.clone() };
            timing.measure(&format!("learn_successors_{}", i), || {
                transition.learn_successors(context, storage, &source, options.cached)
            })?;

            let result = source.relational_product(transition.relation(), transition.meta())?;
            todo1 = todo1.union(&result)?;

            if detect_deadlocks {
                deadlocks = remove_states_with_successor(&todo1, transition, &deadlocks)?;
            }

            // Only chaining accumulates the frontier across groups; for breadth-first every group
            // starts from `todo` again and the per-iteration progress already covers it.
            if chaining && progress.is_due() {
                progress.print((i, ldd_len(&todo1, &mut LddLenCache::new())));
            }
        }

        Ok((todo1, deadlocks))
    } else {
        // Fixpoint: apply each group to a fixpoint before the next, optionally re-applying earlier
        // groups (chaining) after every group.
        let mut todo1 = todo.clone();

        for i in 0..groups.len() {
            trace!("Learning successors for transition group {}:", i);
            timing.measure(&format!("learn_successors_{}", i), || {
                groups[i].learn_successors(context, storage, &todo1, options.cached)
            })?;

            // Apply group i repeatedly until it no longer adds new states.
            loop {
                let old = todo1.clone();
                let result = todo1.relational_product(groups[i].relation(), groups[i].meta())?;
                todo1 = todo1.union(&result)?;
                if todo1 == old {
                    break;
                }

                if progress.is_due() {
                    progress.print((i, ldd_len(&todo1, &mut LddLenCache::new())));
                }
            }

            if detect_deadlocks {
                deadlocks = remove_states_with_successor(&todo1, &groups[i], &deadlocks)?;
            }

            // Apply all previously learned groups repeatedly until a fixpoint.
            if chaining {
                loop {
                    let old = todo1.clone();
                    for group in groups.iter().take(i + 1) {
                        let result = todo1.relational_product(group.relation(), group.meta())?;
                        todo1 = todo1.union(&result)?;
                    }
                    if todo1 == old {
                        break;
                    }

                    if progress.is_due() {
                        progress.print((i, ldd_len(&todo1, &mut LddLenCache::new())));
                    }
                }
            }
        }

        Ok((todo1, deadlocks))
    }
}

/// Removes from `deadlocks` every state that has a `group` transition into `todo1`, i.e. every state
/// that is not actually a deadlock with respect to `group`. Uses the relational predecessor (the
/// inverse of the relational product) restricted to the current deadlock candidates.
fn remove_states_with_successor(
    todo1: &LDDFunction,
    group: &impl TransitionGroup,
    deadlocks: &LDDFunction,
) -> Result<LDDFunction, MercError> {
    let with_successor = todo1.relational_predecessor(group.relation(), group.meta(), deadlocks)?;
    Ok(deadlocks.minus(&with_successor)?)
}

#[cfg(test)]
mod test {
    use std::ops::ControlFlow;

    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use merc_aterm::ATermString;
    use merc_data::DataExpression;
    use merc_data::DataVariable;
    use merc_data::Mcrl2DataSpecification;
    use merc_lts::LTS;
    use merc_lts::LtsAction;
    use merc_lts::LtsBuilderMem;
    use merc_lts::LtsMultiAction;
    use merc_lts::TransitionLabel;
    use merc_utilities::Timing;
    use merc_utilities::random_test;
    use oxidd::ManagerRef;
    use oxidd::ldd::LDDFunction;
    use oxidd::ldd::LDDManagerRef;
    use oxidd::ldd::RelationProductMeta;
    use oxidd::ldd::Value;
    use rand::RngExt;

    use crate::ExplorationStrategy;
    use crate::LddLenCache;
    use crate::ReachabilityOptions;
    use crate::ReachabilityResult;
    use crate::SummandGroup;
    use crate::SylvanLts;
    use crate::SylvanTransitionGroup;
    use crate::SymbolicLPS;
    use crate::SymbolicLTS;
    use crate::SymbolicLts;
    use crate::convert_symbolic_lts;
    use crate::from_iter;
    use crate::ldd_len;
    use crate::random_symbolic_lts;
    use crate::reachability_with_callback;
    use crate::reachability_with_options;
    use crate::read_sylvan;

    /// Capacities of the managers for the tiny models below; allocating the default capacities for every
    /// scenario would dominate the running time.
    const SMALL_NODE_CAPACITY: usize = 1 << 16;
    const SMALL_CACHE_CAPACITY: usize = 1 << 16;

    /// The number of vectors in `ldd`, which is small enough for every test in this module to be exact.
    fn count(ldd: &LDDFunction) -> usize {
        ldd_len(ldd, &mut LddLenCache::new())
            .exact()
            .expect("the count of a test set is exact") as usize
    }

    /// All exploration strategies; every one of them must compute the same reachable set.
    const ALL_STRATEGIES: [ExplorationStrategy; 5] = [
        ExplorationStrategy::BreadthFirst,
        ExplorationStrategy::Chaining,
        ExplorationStrategy::Fixpoint,
        ExplorationStrategy::FixpointChaining,
        ExplorationStrategy::Saturation,
    ];

    /// Runs [reachability_with_options] on `lts` with a fresh context.
    fn reach<L: SymbolicLPS>(
        manager: &LDDManagerRef,
        lts: &mut L,
        options: &ReachabilityOptions,
    ) -> ReachabilityResult {
        let mut context = lts.create_context();
        reachability_with_options(manager, lts, &mut context, options, &Timing::new())
            .expect("Reachability should work correctly")
    }

    /// Explores the Sylvan fixture in `bytes` with the given strategy in a fresh manager, and returns the
    /// manager, the LTS and its reachable states.
    fn explore_fixture(bytes: &[u8], strategy: ExplorationStrategy) -> (LDDManagerRef, SylvanLts, LDDFunction) {
        let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let mut lts = read_sylvan(&ldd_manager, &mut &bytes[..]).expect("Loading should work correctly");

        let options = ReachabilityOptions {
            strategy,
            detect_deadlocks: false,
            // The groups of a Sylvan fixture are fully explored, so there is nothing to cache.
            cached: false,
        };
        let states = reach(&ldd_manager, &mut lts, &options).states;

        (ldd_manager, lts, states)
    }

    /// A [SylvanLts] together with its state space, so that it can be converted to an explicit LTS. The groups
    /// of a Sylvan file have no action label position, so all transitions get the single default label.
    struct SylvanLtsWithStates {
        lts: SylvanLts,
        states: LDDFunction,
        default_label: Vec<LtsMultiAction<LtsAction>>,
    }

    impl SymbolicLPS for SylvanLtsWithStates {
        type Group = SylvanTransitionGroup;

        fn initial_state(&self) -> &LDDFunction {
            self.lts.initial_state()
        }

        fn transition_groups(&self) -> &[Self::Group] {
            self.lts.transition_groups()
        }

        fn transition_groups_mut(&mut self) -> &mut [Self::Group] {
            self.lts.transition_groups_mut()
        }

        fn create_context(&self) {}
    }

    impl SymbolicLTS for SylvanLtsWithStates {
        type Label = LtsMultiAction<LtsAction>;

        fn states(&self) -> &LDDFunction {
            &self.states
        }

        fn action_labels(&self) -> &[Self::Label] {
            &self.default_label
        }

        fn parameter_values(&self) -> &[Vec<DataExpression>] {
            &[]
        }
    }

    /// Asserts that every strategy finds as many reachable states as `BreadthFirst` on the Sylvan fixture in
    /// `bytes`.
    fn assert_strategies_agree(name: &str, bytes: &[u8]) {
        let expected = count(&explore_fixture(bytes, ExplorationStrategy::BreadthFirst).2);
        for strategy in ALL_STRATEGIES {
            assert_eq!(
                expected,
                count(&explore_fixture(bytes, strategy).2),
                "{name}: {strategy:?} disagrees with BreadthFirst"
            );
        }
    }

    /// Asserts that the states found by `BreadthFirst` on the Sylvan fixture in `bytes` are exactly the
    /// reachable ones according to [assert_is_reachable_set]. Converting is far slower than exploring.
    fn assert_fixture_is_reachable_set(name: &str, bytes: &[u8]) {
        let (manager, lts, states) = explore_fixture(bytes, ExplorationStrategy::BreadthFirst);
        let lts = SylvanLtsWithStates {
            lts,
            states,
            default_label: vec![LtsMultiAction::from_index(0)],
        };
        assert_is_reachable_set(name, &manager, &lts, None);
    }

    /// The callback is called once after every iteration, and can stop the exploration: what is found
    /// up to that iteration is returned, and the result says that it is not complete.
    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_reachability_callback() {
        let bytes = include_bytes!("../../../../examples/ldd/anderson.4.ldd");

        for strategy in ALL_STRATEGIES {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
            let options = ReachabilityOptions {
                strategy,
                detect_deadlocks: true,
                cached: false,
            };

            // Without stopping, every iteration is reported once, in order, and the sets only grow.
            let mut lts = read_sylvan(&manager, &mut &bytes[..]).expect("Loading should work correctly");
            let mut context = lts.create_context();
            let mut seen: Vec<(usize, usize)> = Vec::new();
            let full = reachability_with_callback(
                &manager,
                &mut lts,
                &mut context,
                &options,
                &Timing::new(),
                |iteration, states| {
                    seen.push((iteration, count(states)));
                    ControlFlow::Continue(())
                },
            )
            .expect("Reachability should work correctly");

            assert!(
                full.complete,
                "{strategy:?}: an exploration that is not stopped is complete"
            );
            assert!(!seen.is_empty(), "{strategy:?}: the callback is never called");
            assert!(
                seen.iter()
                    .enumerate()
                    .all(|(index, (iteration, _))| *iteration == index + 1),
                "{strategy:?}: the iterations are not numbered consecutively from one: {seen:?}"
            );
            assert!(
                seen.windows(2).all(|pair| pair[0].1 <= pair[1].1),
                "{strategy:?}: the states shrink: {seen:?}"
            );
            assert_eq!(seen.last().unwrap().1, count(&full.states), "{strategy:?}");
            let iterations = seen.len();

            // Stopping after the first iteration returns exactly what the callback saw.
            let mut lts = read_sylvan(&manager, &mut &bytes[..]).expect("Loading should work correctly");
            let mut context = lts.create_context();
            let mut calls = 0;
            let partial = reachability_with_callback(
                &manager,
                &mut lts,
                &mut context,
                &options,
                &Timing::new(),
                |iteration, _| {
                    calls += 1;
                    assert_eq!(iteration, 1, "{strategy:?}: called again after breaking");
                    ControlFlow::Break(())
                },
            )
            .expect("Reachability should work correctly");

            assert_eq!(calls, 1, "{strategy:?}");
            assert_eq!(count(&partial.states), seen[0].1, "{strategy:?}");
            assert!(
                partial.states.minus(&full.states).expect("minus works").is_empty(),
                "{strategy:?}: the states found so far are not reachable"
            );
            // Saturation only knows that it is done after another round that learns nothing new, the
            // other strategies know when the iteration that is stopped was the last one.
            assert_eq!(
                partial.complete,
                iterations == 1 && strategy != ExplorationStrategy::Saturation,
                "{strategy:?}"
            );
            if strategy == ExplorationStrategy::Saturation {
                // The relations only cover what was learned from so far, so nothing is reported.
                assert_eq!(count(partial.deadlocks.as_ref().expect("requested")), 0);
            }

            // Stopping in the last iteration still gives the complete result, except for saturation.
            if strategy != ExplorationStrategy::Saturation {
                let mut lts = read_sylvan(&manager, &mut &bytes[..]).expect("Loading should work correctly");
                let mut context = lts.create_context();
                let last = reachability_with_callback(
                    &manager,
                    &mut lts,
                    &mut context,
                    &options,
                    &Timing::new(),
                    |iteration, _| {
                        if iteration == iterations {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    },
                )
                .expect("Reachability should work correctly");

                assert!(last.complete, "{strategy:?}");
                assert!(last.states == full.states, "{strategy:?}");
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_reachability_strategies_agree() {
        // All strategies must compute the same reachable set, only the convergence speed differs.
        //
        // Only fixtures that `BreadthFirst` explores within the default manager capacity in reasonable
        // time can be used as an oracle: e.g. `anderson.6` and `anderson.8` run out of nodes.
        assert_strategies_agree("anderson.4", include_bytes!("../../../../examples/ldd/anderson.4.ldd"));
        assert_strategies_agree("blocks.2", include_bytes!("../../../../examples/ldd/blocks.2.ldd"));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_reachability_fixture_is_reachable_set() {
        assert_fixture_is_reachable_set("blocks.2", include_bytes!("../../../../examples/ldd/blocks.2.ldd"));
    }

    /// The larger fixtures take minutes in a debug build, and CI runs the debug tests with `--include-ignored`,
    /// so instead of being ignored this test only exists in release builds, which the nightly CI runs.
    #[test]
    #[cfg(not(debug_assertions))]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_reachability_fixtures_slow() {
        let anderson = include_bytes!("../../../../examples/ldd/anderson.4.ldd");
        let bakery = include_bytes!("../../../../examples/ldd/bakery.4.ldd");

        assert_fixture_is_reachable_set("anderson.4", anderson);
        assert_strategies_agree("bakery.4", bakery);
        assert_fixture_is_reachable_set("bakery.4", bakery);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_reachability_strategies_agree() {
        // Differential test: random groups with random read/write patterns (including write-only and
        // read-only groups, and read and write sets that only partially overlap) exercise every branch
        // of the `meta` dispatch.
        random_test(100, |rng| {
            let manager = oxidd::ldd::new_manager(1 << 20, 1 << 18, 1);
            let num_state_variables = rng.random_range(2..=8);
            let mut lts = random_symbolic_lts(rng, &manager, num_state_variables, 3).unwrap();

            assert_strategies_correct(&format!("{num_state_variables} variables"), &manager, &mut lts);
        });
    }

    /// Minimal hand-built LTS over a single parameter with transitions `0 -> 1 -> 2`, so the only
    /// reachable deadlock is state `2`.
    struct LineLts {
        initial: LDDFunction,
        groups: Vec<SylvanTransitionGroup>,
    }

    impl SymbolicLPS for LineLts {
        type Group = SylvanTransitionGroup;

        fn initial_state(&self) -> &LDDFunction {
            &self.initial
        }

        fn transition_groups(&self) -> &[Self::Group] {
            &self.groups
        }

        fn transition_groups_mut(&mut self) -> &mut [Self::Group] {
            &mut self.groups
        }

        fn create_context(&self) {}
    }

    fn line_lts(manager: &oxidd::ldd::LDDManagerRef) -> LineLts {
        // One read+write of parameter 0; relation short vectors place the read/write values at the
        // positions reported by `relation_product_meta`.
        let RelationProductMeta {
            meta,
            read_positions,
            write_positions,
        } = manager
            .with_manager_shared(|m| LDDFunction::relation_product_meta(m, &[0], &[0]))
            .expect("meta");
        let transition = |from: Value, to: Value| {
            let mut vector: Vec<Value> = vec![0; read_positions.len() + write_positions.len()];
            vector[read_positions[0]] = from;
            vector[write_positions[0]] = to;
            vector
        };

        let relation = from_iter(manager, [transition(0, 1), transition(1, 2)].iter());
        let group = SylvanTransitionGroup::new(relation, meta, vec![0], vec![0]);

        LineLts {
            initial: from_iter(manager, std::iter::once(&vec![0])),
            groups: vec![group],
        }
    }

    /// Explores `lts` with the given strategy, detecting deadlocks.
    fn explore(manager: &LDDManagerRef, lts: &mut TestLts, strategy: ExplorationStrategy) -> ReachabilityResult {
        let options = ReachabilityOptions {
            strategy,
            detect_deadlocks: true,
            cached: false,
        };
        reach(manager, lts, &options)
    }

    /// Checks that the states of `lts` are exactly the states reachable from its initial state and, when given,
    /// that `deadlocks` are exactly the ones among them without outgoing transitions.
    ///
    /// The oracle does not use any exploration strategy, nor `relational_product` or `saturate`: the LTS is
    /// converted to an explicit one by [convert_symbolic_lts], which applies the relation vectors to every
    /// individual state.
    fn assert_is_reachable_set<L: SymbolicLTS>(
        name: &str,
        manager: &LDDManagerRef,
        lts: &L,
        deadlocks: Option<&LDDFunction>,
    ) {
        // The conversion fails when a state has a successor outside of the states, i.e., when they are not
        // closed under the transitions, or when the initial state is missing.
        let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
        let explicit = convert_symbolic_lts(manager, &mut builder, lts)
            .unwrap_or_else(|error| panic!("{name}: not a valid state space: {error}"));

        // Every state must be reachable from the initial state, otherwise the states contain spurious ones.
        let initial = explicit.initial_state_index();
        let mut visited = vec![false; explicit.num_of_states().max(initial.value() + 1)];
        visited[initial.value()] = true;
        let mut todo = vec![initial];
        let mut num_reachable = 0;
        let mut num_deadlocks = 0;
        while let Some(state) = todo.pop() {
            num_reachable += 1;

            let mut has_successor = false;
            for transition in explicit.outgoing_transitions(state) {
                has_successor = true;
                if !std::mem::replace(&mut visited[transition.to.value()], true) {
                    todo.push(transition.to);
                }
            }

            if !has_successor {
                num_deadlocks += 1;
            }
        }

        // A state that does not occur in any transition is not part of the explicit LTS, so it is counted
        // as well by comparing against the size of the states.
        assert_eq!(num_reachable, count(lts.states()), "{name}: reachable states");

        if let Some(deadlocks) = deadlocks {
            assert_eq!(num_deadlocks, count(deadlocks), "{name}: deadlocks");
            assert!(
                deadlocks.minus(lts.states()).unwrap().is_empty(),
                "{name}: deadlocks outside of the reachable states"
            );
        }
    }

    /// Checks all strategies on `lts`: they must all agree with `BreadthFirst`, whose result must be correct
    /// according to [assert_is_reachable_set].
    ///
    /// All strategies share the manager, which also checks that the saturation cache is properly reset
    /// between explorations.
    fn assert_strategies_correct(name: &str, manager: &LDDManagerRef, lts: &mut TestLts) {
        let expected = explore(manager, lts, ExplorationStrategy::BreadthFirst);
        let expected_deadlocks = expected.deadlocks.expect("detect_deadlocks was requested");

        // All strategies find the same states, so it suffices to check the oracle once.
        lts.set_states(expected.states.clone());
        assert_is_reachable_set(name, manager, lts, Some(&expected_deadlocks));

        for strategy in ALL_STRATEGIES {
            let result = explore(manager, lts, strategy);
            let deadlocks = result.deadlocks.expect("detect_deadlocks was requested");

            assert!(
                result.states == expected.states,
                "{name}: {strategy:?} found {} reachable state(s), BreadthFirst found {}",
                count(&result.states),
                count(&expected.states),
            );
            assert!(
                deadlocks == expected_deadlocks,
                "{name}: {strategy:?} found {} deadlock(s), BreadthFirst found {}",
                count(&deadlocks),
                count(&expected_deadlocks),
            );
        }
    }

    /// A transition of a summand group.
    struct TransitionSpec {
        /// The values of the read parameters, in the order of [GroupSpec::read].
        read: Vec<Value>,
        /// The values written to the write parameters, in the order of [GroupSpec::write].
        write: Vec<Value>,
        /// The action label, which is stored in a trailing position that `meta` does not cover.
        label: Value,
    }

    /// A summand group, described explicitly by its read and write parameters and its transitions.
    struct GroupSpec {
        read: Vec<u32>,
        write: Vec<u32>,
        transitions: Vec<TransitionSpec>,
    }

    /// Shorthand for a [TransitionSpec].
    fn transition(read: &[Value], write: &[Value], label: Value) -> TransitionSpec {
        TransitionSpec {
            read: read.to_vec(),
            write: write.to_vec(),
            label,
        }
    }

    /// Builds a [SymbolicLts] over `num_parameters` parameters with the given `initial` state and `groups`,
    /// laid out the way `SymbolicLpsGroup` does: relation vectors hold the read/write positions given by the
    /// meta, followed by a single action label position that is not covered by the meta.
    fn summand_lts(manager: &LDDManagerRef, num_parameters: usize, initial: &[Value], groups: &[GroupSpec]) -> TestLts {
        let parameters = (0..num_parameters)
            .map(|i| DataVariable::new(ATermString::new(format!("p{i}"))))
            .collect::<Vec<_>>();

        let groups = groups
            .iter()
            .map(|group| {
                let RelationProductMeta {
                    read_positions,
                    write_positions,
                    ..
                } = manager
                    .with_manager_shared(|m| LDDFunction::relation_product_meta(m, &group.read, &group.write))
                    .expect("meta");

                let vectors: Vec<Vec<Value>> = group
                    .transitions
                    .iter()
                    .map(|t| {
                        let mut vector = vec![0; read_positions.len() + write_positions.len() + 1];
                        for (&position, &value) in read_positions.iter().zip(&t.read) {
                            vector[position] = value;
                        }
                        for (&position, &value) in write_positions.iter().zip(&t.write) {
                            vector[position] = value;
                        }
                        *vector.last_mut().unwrap() = t.label;
                        vector
                    })
                    .collect();

                SummandGroup::new(
                    manager,
                    &parameters,
                    group.read.iter().map(|&i| parameters[i as usize].clone()).collect(),
                    group.write.iter().map(|&i| parameters[i as usize].clone()).collect(),
                    from_iter(manager, vectors.iter()),
                )
                .expect("parameters exist")
            })
            .collect();

        let initial = from_iter(manager, std::iter::once(&initial.to_vec()));
        let value = DataExpression::from_string("1").unwrap();

        SymbolicLts::new(
            Mcrl2DataSpecification::default(),
            parameters,
            initial.clone(),
            initial,
            groups,
            (0..NUM_LABELS).map(LtsMultiAction::from_index).collect(),
            vec![vec![value; NUM_VALUES]; num_parameters],
        )
    }

    /// The type of LTS that the random generator produces.
    type TestLts = SymbolicLts<LtsMultiAction<LtsAction>>;

    /// The number of action labels and parameter values used by the specified groups below.
    const NUM_LABELS: usize = 3;
    const NUM_VALUES: usize = 3;

    /// Runs [assert_is_reachable_set] on the line `0 -> 1 -> 2` with the given state set.
    fn check_line_states(states: &[Vec<Value>]) {
        let manager = oxidd::ldd::new_manager(SMALL_NODE_CAPACITY, SMALL_CACHE_CAPACITY, 1);
        let group = GroupSpec {
            read: vec![0],
            write: vec![0],
            transitions: vec![transition(&[0], &[1], 0), transition(&[1], &[2], 0)],
        };

        let mut lts = summand_lts(&manager, 1, &[0], &[group]);
        lts.set_states(from_iter(&manager, states.iter()));
        assert_is_reachable_set("line", &manager, &lts, None);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_oracle_accepts_reachable_states() {
        check_line_states(&[vec![0], vec![1], vec![2]]);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    #[should_panic(expected = "not a valid state space")]
    fn test_oracle_rejects_missing_states() {
        // State 1 has a successor, state 2, that is not in the state set.
        check_line_states(&[vec![0], vec![1]]);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    #[should_panic(expected = "reachable states")]
    fn test_oracle_rejects_spurious_states() {
        // State 3 is closed under the transitions, but not reachable from the initial state.
        check_line_states(&[vec![0], vec![1], vec![2], vec![3]]);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_reachability_degenerate_groups() {
        // A group without read and write parameters: its relation only holds the trailing action label, so
        // it is a self-loop on every state that must neither add states nor be reported as a deadlock.
        let unconditional = || GroupSpec {
            read: vec![],
            write: vec![],
            transitions: vec![transition(&[], &[], 0)],
        };
        // Write-only: fires on every state. The transitions differ only in the value written and the label.
        let write_only = || GroupSpec {
            read: vec![],
            write: vec![0],
            transitions: vec![transition(&[], &[1], 1), transition(&[], &[2], 2)],
        };
        // Read-only: a guard that never changes the state.
        let read_only = || GroupSpec {
            read: vec![0],
            write: vec![],
            transitions: vec![transition(&[2], &[], 1)],
        };
        // Reads and writes disjoint parameters.
        let disjoint = || GroupSpec {
            read: vec![0],
            write: vec![2],
            transitions: vec![transition(&[1], &[1], 0), transition(&[2], &[2], 1)],
        };
        // Reads and writes the same parameter. The same read/write pair occurs with several labels, so that
        // the trailing position must be ignored by the product and only deduplicated by the relation.
        let read_write = || GroupSpec {
            read: vec![1],
            write: vec![1],
            transitions: (0..2)
                .flat_map(|v| (0..3).map(move |label| transition(&[v], &[v + 1], label)))
                .collect(),
        };
        // Reads two parameters and writes one of them, the other being read-only.
        let overlapping = || GroupSpec {
            read: vec![1, 2],
            write: vec![2],
            transitions: vec![transition(&[2, 0], &[1], 0), transition(&[1, 1], &[2], 1)],
        };
        // A group that never fires.
        let empty = || GroupSpec {
            read: vec![0],
            write: vec![1],
            transitions: vec![],
        };

        // The models are tiny, so a small manager is enough.
        let check = |name: &str, groups: &[GroupSpec]| {
            let manager = oxidd::ldd::new_manager(SMALL_NODE_CAPACITY, SMALL_CACHE_CAPACITY, 1);
            let mut lts = summand_lts(&manager, 3, &[0, 0, 0], groups);
            assert_strategies_correct(name, &manager, &mut lts);
        };

        check(
            "all",
            &[
                unconditional(),
                write_only(),
                read_only(),
                disjoint(),
                read_write(),
                overlapping(),
                empty(),
            ],
        );

        // Without the groups that fire in every state there are deadlocks, and every subset of the
        // remaining groups is a different mix of the `meta` branches.
        let optional: [fn() -> GroupSpec; 6] = [read_only, disjoint, read_write, overlapping, empty, unconditional];
        for mask in 1..(1u32 << optional.len()) {
            let groups = optional
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, group)| group())
                .collect::<Vec<_>>();
            check(&format!("mask {mask:#b}"), &groups);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_reachability_detect_deadlocks() {
        for strategy in [
            ExplorationStrategy::BreadthFirst,
            ExplorationStrategy::Chaining,
            ExplorationStrategy::Fixpoint,
            ExplorationStrategy::FixpointChaining,
            ExplorationStrategy::Saturation,
        ] {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
            let mut lts = line_lts(&manager);

            let options = ReachabilityOptions {
                strategy,
                detect_deadlocks: true,
                cached: false,
                ..ReachabilityOptions::default()
            };
            let mut context = lts.create_context();
            let result = reachability_with_options(&manager, &mut lts, &mut context, &options, &Timing::new())
                .expect("Reachability should work correctly");

            // States 0, 1, 2 are reachable and only state 2 has no outgoing transition.
            assert_eq!(count(&result.states), 3, "{strategy:?}");
            let deadlocks = result.deadlocks.expect("detect_deadlocks was requested");
            assert_eq!(count(&deadlocks), 1, "{strategy:?}");
        }
    }

    /// Same as [test_reachability_detect_deadlocks], but with [`ReachabilityOptions::cached`] set:
    /// caching must not change which states are reported as deadlocks, only avoid re-enumerating
    /// read-projections `learn_successors` already learned from. No existing test combines
    /// `detect_deadlocks` with `cached`.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_reachability_detect_deadlocks_cached() {
        for strategy in [
            ExplorationStrategy::BreadthFirst,
            ExplorationStrategy::Chaining,
            ExplorationStrategy::Fixpoint,
            ExplorationStrategy::FixpointChaining,
            ExplorationStrategy::Saturation,
        ] {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
            let mut lts = line_lts(&manager);

            let options = ReachabilityOptions {
                strategy,
                detect_deadlocks: true,
                cached: true,
            };
            let mut context = lts.create_context();
            let result = reachability_with_options(&manager, &mut lts, &mut context, &options, &Timing::new())
                .expect("Reachability should work correctly");

            // States 0, 1, 2 are reachable and only state 2 has no outgoing transition.
            assert_eq!(count(&result.states), 3, "{strategy:?}");
            let deadlocks = result.deadlocks.expect("detect_deadlocks was requested");
            assert_eq!(count(&deadlocks), 1, "{strategy:?}");
        }
    }

    /// Same as [test_reachability_degenerate_groups], but with `cached: true`: every mix of the
    /// `meta` branches (write-only, read-only, disjoint, overlapping read/write, unconditional,
    /// never-firing) must still find the same deadlocks once caching skips re-enumerating known
    /// read-projections.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_reachability_degenerate_groups_cached() {
        let unconditional = || GroupSpec {
            read: vec![],
            write: vec![],
            transitions: vec![transition(&[], &[], 0)],
        };
        let write_only = || GroupSpec {
            read: vec![],
            write: vec![0],
            transitions: vec![transition(&[], &[1], 1), transition(&[], &[2], 2)],
        };
        let read_only = || GroupSpec {
            read: vec![0],
            write: vec![],
            transitions: vec![transition(&[2], &[], 1)],
        };
        let disjoint = || GroupSpec {
            read: vec![0],
            write: vec![2],
            transitions: vec![transition(&[1], &[1], 0), transition(&[2], &[2], 1)],
        };
        let read_write = || GroupSpec {
            read: vec![1],
            write: vec![1],
            transitions: (0..2)
                .flat_map(|v| (0..3).map(move |label| transition(&[v], &[v + 1], label)))
                .collect(),
        };
        let overlapping = || GroupSpec {
            read: vec![1, 2],
            write: vec![2],
            transitions: vec![transition(&[2, 0], &[1], 0), transition(&[1, 1], &[2], 1)],
        };

        let check_cached = |name: &str, groups: &[GroupSpec]| {
            let manager = oxidd::ldd::new_manager(SMALL_NODE_CAPACITY, SMALL_CACHE_CAPACITY, 1);
            let mut lts = summand_lts(&manager, 3, &[0, 0, 0], groups);

            // Oracle: BreadthFirst, uncached (already checked correct by the uncached test).
            let expected = {
                let options = ReachabilityOptions {
                    strategy: ExplorationStrategy::BreadthFirst,
                    detect_deadlocks: true,
                    cached: false,
                };
                reach(&manager, &mut lts, &options)
            };
            let expected_deadlocks = expected.deadlocks.expect("detect_deadlocks was requested");

            for strategy in ALL_STRATEGIES {
                let options = ReachabilityOptions {
                    strategy,
                    detect_deadlocks: true,
                    cached: true,
                };
                let result = reach(&manager, &mut lts, &options);
                let deadlocks = result.deadlocks.expect("detect_deadlocks was requested");

                assert!(
                    result.states == expected.states,
                    "{name}: {strategy:?} (cached) found {} reachable state(s), expected {}",
                    count(&result.states),
                    count(&expected.states),
                );
                assert!(
                    deadlocks == expected_deadlocks,
                    "{name}: {strategy:?} (cached) found {} deadlock(s), expected {}",
                    count(&deadlocks),
                    count(&expected_deadlocks),
                );
            }
        };

        check_cached(
            "all",
            &[
                unconditional(),
                write_only(),
                read_only(),
                disjoint(),
                read_write(),
                overlapping(),
            ],
        );
    }
}
