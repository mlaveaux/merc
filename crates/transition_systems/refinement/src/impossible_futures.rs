use std::cell::RefCell;

use log::debug;
use merc_collections::VecSet;
use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_reduction::diverges;

use crate::AC;
use crate::Antichain;
use crate::CounterExampleConstructor;
use crate::CounterExampleTree;
use crate::ExplorationStrategy;
use crate::is_refinement_generic;
use crate::is_stable;

/// The outcome of an impossible-futures refinement check.
pub struct ImpossibleFuturesResult<Index, Label> {
    /// Whether the impossible-futures refinement holds.
    pub result: bool,

    /// The node in the counter-example tree that witnesses the failure, if any.
    pub counter_example: Option<Index>,

    /// The impossible futures (traces) witnessing the failure, if any.
    pub impossible_futures: Option<Vec<Vec<Label>>>,
}

/// Checks for the impossible futures refinement between the initial state of
/// the `lts` and the `initial_spec` state.
///
/// # Details
///
/// Impossible futures are defined in the following article:
///
/// > Marc Voorhoeve, Sjouke Mauw. Impossible futures and determinism, Inf. Process. Lett. 80, 2001.
pub fn is_impossible_futures_refinement<L: LTS, CE: CounterExampleTree>(
    lts: &L,
    initial_spec: StateIndex,
    strategy: ExplorationStrategy,
    counter_example: &mut CE,
) -> ImpossibleFuturesResult<CE::Index, L::Label> {
    let mut antichain = Antichain::new();

    // The inner antichain is reused between computations of the weak trace inclusion. The positive antichain contains all the
    // pairs that have passed the check so far.
    let negative_antichain = RefCell::new(Antichain::new());
    let mut positive_antichain = PositiveAntichain::new();

    let (holds, ce_index, impossible_futures) = is_refinement_generic(
        strategy,
        lts,
        lts.initial_state_index(),
        initial_spec,
        |impl_state, spec_states| {
            if !is_stable(lts, impl_state) {
                // We can skip unstable states as an optimisation.
                debug_assert!(!diverges(lts, impl_state), "Implementation states should not diverge.");
                return (None, true);
            }

            // Observe that the weak trace inclusion is inverted, this exactly
            // corresponds to checking for impossible futures.
            if !spec_states.iter().any(|t| {
                is_weak_trace_refinement(
                    lts,
                    *t,
                    impl_state,
                    strategy,
                    &mut positive_antichain,
                    &negative_antichain,
                )
            }) {
                let mut futures = Vec::new();

                for t in spec_states {
                    // Run the weak trace refinement again with a counter example.
                    let mut ce_constructor = CounterExampleConstructor::new();

                    let (result, ce) = is_weak_trace_refinement_ce(lts, *t, impl_state, strategy, &mut ce_constructor);
                    debug_assert!(
                        !result,
                        "The weak trace refinement should fail according to the previous check."
                    );

                    let trace = ce_constructor
                        .reconstruct_trace(ce.expect("A counter example was requested"))
                        .iter()
                        .map(|l| lts.labels()[*l].clone())
                        .collect();

                    futures.push(trace);
                }

                return (Some(futures), true);
            }

            (None, true)
        },
        |_, _| (),
        true,
        counter_example,
        &mut antichain,
    );

    debug!("Antichain stats: {:?}", antichain.metrics());
    debug!("Negative antichain stats: {:?}", negative_antichain.borrow().metrics());
    ImpossibleFuturesResult {
        result: holds,
        counter_example: ce_index,
        impossible_futures,
    }
}

/// A combined antichain that checks both the persistent positive antichain and
/// the current computation's antichain for inclusion at the same time. When a
/// weak trace check passes, the pairs from the current antichain are integrated
/// into the positive antichain so later checks can short-circuit on them.
struct PositiveAntichain {
    /// An antichain that is used for repeated weak trace inclusion checks,
    positive_antichain: Antichain<StateIndex, StateIndex>,

    /// The antichain used for the current computation.
    antichain: Antichain<StateIndex, StateIndex>,
}

impl PositiveAntichain {
    fn new() -> Self {
        Self {
            positive_antichain: Antichain::new(),
            antichain: Antichain::new(),
        }
    }
}

impl AC<StateIndex, StateIndex> for PositiveAntichain {
    fn insert(&mut self, key: StateIndex, value: VecSet<StateIndex>) -> bool {
        if self.positive_antichain.contains_subset(&key, &value) {
            // If the positive antichain contains a subset of the current pair, then we can immediately conclude that the check passes.
            return false;
        }

        self.antichain.insert(key, value)
    }

    fn clear(&mut self) {
        self.antichain.clear();
    }
}

/// Checks for the various weak trace refinement. This is a helper function for
/// the impossible futures refinement check.
fn is_weak_trace_refinement<L: LTS>(
    lts: &L,
    impl_state: StateIndex,
    spec_state: StateIndex,
    strategy: ExplorationStrategy,
    positive_antichain: &mut PositiveAntichain,
    negative_antichain: &RefCell<Antichain<StateIndex, StateIndex>>,
) -> bool {
    let (result, _counter_example, _inner_ce) = is_refinement_generic(
        strategy,
        lts,
        impl_state,
        spec_state,
        |impl_state, spec_states| {
            if negative_antichain.borrow().contains_superset(&impl_state, spec_states) {
                // If the negative antichain contains a superset of the current pair, then we can immediately conclude that the check fails.
                return (Some(()), false);
            }

            (None, true)
        },
        |impl_state, spec_states| {
            // If the check fails, then we can add the pair to the negative antichain, which is used for the impossible futures check.
            negative_antichain.borrow_mut().insert(impl_state, spec_states.clone());
        },
        true,
        &mut (),
        positive_antichain,
    );

    if result {
        // If the check passed, then we can add the pair to the positive antichain, which is used for the impossible futures check.
        for (impl_state, spec_states) in positive_antichain.antichain.iter() {
            positive_antichain
                .positive_antichain
                .insert(*impl_state, spec_states.clone());
        }
    }

    positive_antichain.clear();
    result
}

/// This is the [is_weak_trace_refinement] but can generate a counter example.
fn is_weak_trace_refinement_ce<L: LTS, CE: CounterExampleTree>(
    lts: &L,
    impl_state: StateIndex,
    spec_state: StateIndex,
    strategy: ExplorationStrategy,
    counter_example: &mut CE,
) -> (bool, Option<CE::Index>) {
    let mut antichain = Antichain::new();
    let (result, counter_example, inner_ce) = is_refinement_generic(
        strategy,
        lts,
        impl_state,
        spec_state,
        |_, _| (Option::<()>::None, true),
        |_, _| (),
        true,
        counter_example,
        &mut antichain,
    );

    debug_assert!(inner_ce.is_none(), "The counter example from check is trivial");
    (result, counter_example)
}
