use std::collections::HashSet;
use std::collections::VecDeque;

use itertools::Itertools;
use log::trace;

use merc_collections::VecSet;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::StateIndex;
use merc_reduction::diverges;

use crate::AC;
use crate::Antichain;
use crate::CounterExampleTree;
use crate::ExplorationStrategy;
use crate::RefinementType;

/// The result of the inner check in the refinement algorithm.
pub enum InnerCe {
    Refusal(Vec<LabelIndex>),
    Diverges,
}

/// Checks for the various stable failures refinement relations.
///
/// These antichain-based algorithms cover trace, weak-trace, stable-failures and
/// failures-divergence inclusion, and are described in:
///
/// > M. Laveaux, J.F. Groote and T.A.C. Willemse. Correct and Efficient
/// > Antichain Algorithms for Refinement Checking. Logical Methods in Computer
/// > Science 17(1) 2021
///
/// Returns the result, and the state in the counter example tree that witnesses
/// the failure if the result is false. Finally, the result of the inner
/// (impl,spec) check is returned as well, this is used to construct the counter
/// example.
pub fn is_failures_refinement<L: LTS, CE: CounterExampleTree>(
    lts: &L,
    initial_spec: StateIndex,
    refinement: RefinementType,
    strategy: ExplorationStrategy,
    counter_example: &mut CE,
) -> (bool, Option<CE::Index>, Option<InnerCe>) {
    let mut antichain = Antichain::new();

    match refinement {
        RefinementType::Trace => is_refinement_generic(
            strategy,
            lts,
            lts.initial_state_index(),
            initial_spec,
            |_, _| (None, true),
            |_, _| (),
            false,
            counter_example,
            &mut antichain,
        ),
        RefinementType::Weaktrace => is_refinement_generic(
            strategy,
            lts,
            lts.initial_state_index(),
            initial_spec,
            |_, _| (None, true),
            |_, _| (),
            true,
            counter_example,
            &mut antichain,
        ),
        RefinementType::StableFailures => is_refinement_generic(
            strategy,
            lts,
            lts.initial_state_index(),
            initial_spec,
            |impl_state, spec_states| {
                (
                    refusals_contained_in(lts, impl_state, spec_states).map(InnerCe::Refusal),
                    true,
                )
            },
            |_, _| (),
            true,
            counter_example,
            &mut antichain,
        ),
        RefinementType::FailuresDivergences => is_refinement_generic(
            strategy,
            lts,
            lts.initial_state_index(),
            initial_spec,
            |impl_state, spec_states| {
                if !spec_states.iter().any(|s| diverges(lts, *s)) {
                    trace!("spec {:?} is convergent!", spec_states);
                    // If the implementation state diverges, then it can refuse any set of actions.
                    if diverges(lts, impl_state) {
                        trace!("impl {:?} diverges, return false", impl_state);
                        (Some(InnerCe::Diverges), true)
                    } else {
                        (
                            refusals_contained_in(lts, impl_state, spec_states).map(InnerCe::Refusal),
                            true,
                        )
                    }
                } else {
                    (None, false)
                }
            },
            |_, _| (),
            true,
            counter_example,
            &mut antichain,
        ),
        _ => unreachable!("This refinement variant {refinement:?} can not be checked by is_failures_refinement"),
    }
}

/// A generic implementation for antichain based refinement algorithm.
///
/// # Details
///
/// Given the (initial_impl, initial_spec) pair the algorithm explores the
/// product state space of the implementation and the normalised specification
/// LTSs.
///
/// If `weak_transition` is true then the algorithm considers weak transitions
/// for the specification LTS, otherwise it only considers the immediate
/// transitions.
///
/// The `check` function is applied to every pair (impl, spec) that is explored
/// during the algorithm, it should return `None` if the pair is valid, and
/// `Some(counter_example)` if the pair is invalid. Furthermore, a boolean is
/// returned to indicate that exploration should continue (true) for this pair.
/// This only applies when no counter example is returned.
///
/// The `CE` type parameter indicates the type of counter example tree that is
/// used to construct counter examples. If no counter examples are required,
/// this can be set to `()`. Avoiding the cost for keeping track of counter
/// example information.
///
/// The antichain data structure is used for storing explored states. However,
/// as opposed to a discovered set it allows for pruning additional pairs based
/// on the `antichain` property.
#[allow(clippy::too_many_arguments)]
pub fn is_refinement_generic<L: LTS, A: AC<StateIndex, StateIndex>, CE: CounterExampleTree, F, G, CC>(
    strategy: ExplorationStrategy,
    merged_lts: &L,
    initial_impl: StateIndex,
    initial_spec: StateIndex,
    mut check: F,
    mut failing_trace: G,
    weak_transition: bool,
    counter_example: &mut CE,
    antichain: &mut A,
) -> (bool, Option<CE::Index>, Option<CC>)
where
    F: FnMut(StateIndex, &VecSet<StateIndex>) -> (Option<CC>, bool),
    G: FnMut(StateIndex, &VecSet<StateIndex>),
{
    // A local cache used for the tau closure computations.
    let mut closure_cache = ClosureCache::new();

    let mut working = VecDeque::from([(
        initial_impl,
        if weak_transition {
            VecSet::from_vec(tau_closure(merged_lts, vec![initial_spec], &mut closure_cache))
        } else {
            VecSet::singleton(initial_spec)
        },
        counter_example.root_index(),
    )]);

    // pop (impl,spec) from working;
    while let Some((impl_state, spec, ce)) = working.pop_front() {
        trace!("Checking ({:?}, {:?})", impl_state, spec);
        let (inner_counter_example, continue_exploration) = check(impl_state, &spec);
        if let Some(counter_example) = inner_counter_example {
            // if not check(impl,spec) then return false.
            return (false, Some(ce), Some(counter_example));
        }

        if !continue_exploration {
            // If the check indicates that we should not continue exploration, then we can skip exploring the outgoing edges.
            continue;
        }

        // for every impl -[e]-> impl' do
        for impl_transition in merged_lts.outgoing_transitions(impl_state) {
            // Whether this is an impl-side tau step that is unobservable under
            // weak semantics. Such steps must not be recorded as literal edges
            // in the counter-example tree: `CounterExample::WeakTrace` and its
            // relatives document taus as implicit (`<tau*.a0.tau*...>true`),
            // not as explicit trace entries. Strong trace refinement
            // (`weak_transition == false`) legitimately treats tau as an
            // ordinary, observable action, so it is not elided there.
            let is_unobservable_tau = weak_transition && merged_lts.is_hidden_label(impl_transition.label);

            let spec_prime = if is_unobservable_tau {
                // spec' := spec if e == tau
                spec.clone()
            } else {
                // spec' := {s' | exists s in spec. s -[e]-> s'};
                let mut spec_prime = VecSet::new();

                if weak_transition {
                    // For weak trace refinement we need to consider
                    // tau-closures `s => s1 -[e]-> s2 => s'`, but only include
                    // the states after the `e` transition.
                    let closure = tau_closure(merged_lts, spec.clone().to_vec(), &mut closure_cache);

                    for s in &closure {
                        for spec_transition in merged_lts.outgoing_transitions(*s) {
                            if impl_transition.label == spec_transition.label {
                                spec_prime.insert(spec_transition.to);
                            }
                        }
                    }

                    spec_prime = VecSet::from_vec(tau_closure(merged_lts, spec_prime.to_vec(), &mut closure_cache));
                } else {
                    // Otherwise, simply consider direct transitions.
                    for s in &spec {
                        for spec_transition in merged_lts.outgoing_transitions(*s) {
                            if impl_transition.label == spec_transition.label {
                                spec_prime.insert(spec_transition.to);
                            }
                        }
                    }
                }

                spec_prime
            };

            trace!(
                " -[{}]-> ({}, {:?})",
                merged_lts.labels()[impl_transition.label],
                impl_transition.to,
                spec_prime
            );
            if spec_prime.is_empty() {
                // if spec' = {} then
                //
                // An unobservable tau step never leaves `spec_prime` empty: it
                // is set to `spec.clone()` above, and `spec` is always
                // non-empty here (the only way a pair with an empty spec
                // reaches `working` is via this same check returning early).
                debug_assert!(
                    !is_unobservable_tau,
                    "an unobservable tau step must not yield an empty spec'"
                );
                failing_trace(impl_state, &spec);
                let new_edge = counter_example.add_edge(impl_transition.label, ce);
                return (false, Some(new_edge), None);
            }

            if antichain.insert(impl_transition.to, spec_prime.clone()) {
                // if antichain_insert(impl,spec') then
                trace!("Added ({:?}, {:?}) to working", impl_transition.to, spec_prime);

                // Only record a literal edge in the counter-example tree for
                // observable steps. An unobservable tau step keeps the same
                // parent `ce`, so it does not appear in the reconstructed
                // trace, matching taus being implicit in the counter-example
                // contract (see `is_unobservable_tau` above).
                let next_ce = if is_unobservable_tau {
                    ce
                } else {
                    counter_example.add_edge(impl_transition.label, ce)
                };
                match strategy {
                    ExplorationStrategy::BFS => working.push_back((impl_transition.to, spec_prime, next_ce)),
                    ExplorationStrategy::DFS => working.push_front((impl_transition.to, spec_prime, next_ce)),
                }
            }
        }
    }

    (true, None, None)
}

/// This function checks that the refusals(impl) are contained in the refusals
/// of spec, it returns Some(refusal) iff the inclusion fails for the maximal refusal set.
///  
/// # Details
///
/// See [refusals_contained_in_naive] for the definition of refusals.
///
/// In practice it can be more efficient to look at the enabled set of
/// states:
///
/// > enabled(s) = { a | exists s'. s -a-> s' } if stable(s)
///
/// then we have that refusals(impl) ⊆ refusals(spec) iff there exists a stable
/// s in spec such that enabled(s) ⊆ enabled(impl), and the enabled sets are
/// more efficient to compute.
fn refusals_contained_in<L: LTS>(
    lts: &L,
    impl_state: StateIndex,
    spec_states: &VecSet<StateIndex>,
) -> Option<Vec<LabelIndex>> {
    if !is_stable(lts, impl_state) {
        // If the implementation state is not stable, then it cannot have any refusals (or is maximally accepting).
        return None;
    }

    // refusals(impl) ⊆ refusals(spec) iff there exists a stable spec state s with enabled(s) ⊆ enabled(impl).
    for s in spec_states.iter() {
        if !is_stable(lts, *s) {
            // Unstable spec states do not contribute to the refusals of the specification, so we can ignore them.
            continue;
        }

        // This is O(n^2), but it avoids allocating the refusal sets, which are
        // often small anyway.
        let mut is_witness = true;
        for transition_spec in lts.outgoing_transitions(*s) {
            if !lts
                .outgoing_transitions(impl_state)
                .any(|transition_impl| transition_impl.label == transition_spec.label)
            {
                // s has an action impl cannot do, so s is not a witness (enabled(s) ⊄ enabled(impl)).
                is_witness = false;
                break;
            }
        }

        if is_witness {
            // All of s's enabled actions are also enabled in impl: s is a witness.
            // Therefore refusals(impl) ⊆ refusals(s) ⊆ refusals_set(spec).
            debug_assert!(refusals_contained_in_naive(lts, impl_state, spec_states));
            return None;
        }
    }

    // No stable spec state can witness enabled(s) ⊆ enabled(impl), so refusal inclusion fails.
    debug_assert!(!refusals_contained_in_naive(lts, impl_state, spec_states));
    Some(maximal_refusals(lts, impl_state).to_vec())
}

/// A naive implementation for checking that the refusals of an implementation state are contained in the refusals of a set of specification states.
fn refusals_contained_in_naive<L: LTS>(lts: &L, impl_state: StateIndex, spec_states: &VecSet<StateIndex>) -> bool {
    if !is_stable(lts, impl_state) {
        // If the implementation state is not stable, then it cannot have any refusals.
        return true;
    }

    let impl_refusals = refusals(lts, impl_state);
    let spec_refusals = refusals_set(lts, spec_states);
    trace!("impl refusals: {:?}, spec refusals: {:?}", impl_refusals, spec_refusals);

    impl_refusals.is_subset(&spec_refusals)
}

/// Naive implementation for the refusals of a set of states spec:
///
/// > refusals(spec) = { r | exists s in spec. r in refusals(s) and stable(s) }
fn refusals_set<L: LTS>(lts: &L, spec_states: &VecSet<StateIndex>) -> VecSet<VecSet<LabelIndex>> {
    let mut result = VecSet::new();

    for s in spec_states.iter() {
        if is_stable(lts, *s) {
            result.extend(refusals(lts, *s).iter());
        }
    }

    result
}

/// Returns the maximal refusal set of a state s:
///
/// > maximal_refusals(s) = (Act \setminus enabled(s))
///
/// for stable states s. For unstable states this set is empty.
fn maximal_refusals<L: LTS>(lts: &L, state: StateIndex) -> VecSet<LabelIndex> {
    if !is_stable(lts, state) {
        return VecSet::new();
    }

    // The set of actions enabled in the given state.
    let enabled_labels: VecSet<LabelIndex> =
        VecSet::from_vec(lts.outgoing_transitions(state).map(|t| t.label).collect());

    // The set of all visible actions.
    let all_labels: VecSet<LabelIndex> = VecSet::from_vec(
        lts.labels()
            .iter()
            .enumerate()
            // We cannot refuse the tau action.
            .filter(|(i, _)| !lts.is_hidden_label(LabelIndex::new(*i)))
            .map(|(i, _)| LabelIndex::new(i))
            .collect(),
    );

    VecSet::from_iter(all_labels.difference(&enabled_labels).cloned())
}

/// Naive implementation of refusals of a state s:
///
/// A state s is stable, denoted by stable(s) iff `tau \not\in enabled(s)`, and
/// refusals are defined for stable states s by:
///
/// > refusals(s) = { r | r \subseteq (Act \setminus enabled(s)) }.
fn refusals<L: LTS>(lts: &L, state: StateIndex) -> VecSet<VecSet<LabelIndex>> {
    // The refusal set of a stable state includes all subsets of its maximal refusal set.
    let maximal_refusal = maximal_refusals(lts, state);

    // Take the powerset of `Act \setminus enabled(s)` to get all refusals.
    VecSet::from_iter(maximal_refusal.iter().cloned().powerset().map(VecSet::from_iter))
}

/// Returns true iff the given state is stable, i.e., it has no outgoing tau transitions.
pub fn is_stable<L: LTS>(lts: &L, state: StateIndex) -> bool {
    lts.outgoing_transitions(state).all(|t| !lts.is_hidden_label(t.label))
}

/// A cache that is used to reuse allocations during tau-closure computations.
pub struct ClosureCache {
    /// States that are still to be explored.
    working: Vec<StateIndex>,

    /// Set of already visited states.
    visited: HashSet<StateIndex>,
}

impl ClosureCache {
    /// Creates a new closure cache.
    pub fn new() -> Self {
        ClosureCache {
            working: Vec::new(),
            visited: HashSet::new(),
        }
    }
}

impl Default for ClosureCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the tau closure for a set of states in the given LTS.
///
/// # Details
///
/// The `states` parameter indicates the initial set of states for which the
/// tau-closure is to be computed; the original states are always included in the
/// returned closure. The `cache` parameter is used to avoid repeated allocations.
pub fn tau_closure<L: LTS>(lts: &L, mut states: Vec<StateIndex>, cache: &mut ClosureCache) -> Vec<StateIndex> {
    debug_assert!(
        cache.working.is_empty() && cache.visited.is_empty(),
        "Closure cache working not cleared before use."
    );

    // Initialize the working set with the initial states, note that states is
    // kept in tact. As such the original states are also returned.
    cache.working.extend(states.iter().cloned());

    // Keep track of states that are already in the closure.
    for s in &states {
        cache.visited.insert(*s);
    }

    while let Some(s) = cache.working.pop() {
        for t in lts.outgoing_transitions(s) {
            if lts.is_hidden_label(t.label) && !cache.visited.contains(&t.to) {
                cache.working.push(t.to);
                cache.visited.insert(t.to);
                states.push(t.to);
            }
        }
    }

    // Clear the cache for the next use, the working set is empty by now.
    cache.visited.clear();

    states
}

#[cfg(test)]
mod tests {
    use merc_lts::read_aut;
    use merc_utilities::Timing;
    use merc_utilities::test_logger;

    use crate::ExplorationStrategy;
    use crate::RefinementType;
    use crate::refines;

    #[test]
    fn test_example_2_12() {
        test_logger();

        let s0 = r#"des(0, 6, 5)
            (0, "req", 1)
            (1, "i", 2)
            (2, "10", 3)
            (3, "10", 0)
            (1, "i", 5)
            (5, "20", 0)"#;

        let t0 = r#"des(0, 3, 2)
            (0, "req", 1)
            (1, "20", 2)"#;

        let u0 = r#"des(0, 4, 3)
            (0, "req", 1)
            (1, "i", 1)
            (1, "20", 2)
            (2, "i", 0)"#;

        let s0 = read_aut(s0.as_bytes()).unwrap();
        let t0 = read_aut(t0.as_bytes()).unwrap();
        let u0 = read_aut(u0.as_bytes()).unwrap();

        let mut timing = Timing::new();
        assert!(
            refines(
                t0.clone(),
                s0.clone(),
                RefinementType::Weaktrace,
                ExplorationStrategy::BFS,
                false,
                false,
                &mut timing
            )
            .0
        );
        assert!(
            !refines(
                t0,
                s0.clone(),
                RefinementType::StableFailures,
                ExplorationStrategy::BFS,
                false,
                false,
                &mut timing
            )
            .0
        );
        assert!(
            refines(
                u0,
                s0,
                RefinementType::StableFailures,
                ExplorationStrategy::BFS,
                false,
                false,
                &mut timing
            )
            .0
        );
    }
}
