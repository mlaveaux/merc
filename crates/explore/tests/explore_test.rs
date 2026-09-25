//! Validates the parallel and caching exploration drivers against the
//! single-threaded [`merc_explore::explore`], which is simple enough to act as
//! the naive reference. That baseline is in turn anchored to closed-form state
//! and transition counts so it cannot drift silently.

mod mock_lps;

use std::collections::BTreeSet;
use std::collections::HashMap;

use itertools::Itertools;
use merc_explore::CacheLPS;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_explore::LPS;
use merc_explore::PermutedLps;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_explore::explore;
use merc_explore::explore_parallel;
use merc_utilities::Timing;

use mock_lps::Edge;
use mock_lps::Label;
use mock_lps::MockLps;
use mock_lps::State;

/// Per-worker accumulator: the dense index -> state-vector map (from `on_state`)
/// and the raw `(from_index, label, to_index)` transitions (from `on_transition`).
#[derive(Default)]
struct Accumulator {
    states: HashMap<usize, State>,
    edges: Vec<(usize, Label, usize)>,
}

/// Translates the index-keyed accumulator into state-vector form, so runs are
/// compared by concrete state rather than by the dense index numbering the
/// drivers assign. Every transition endpoint is a discovered state, so it is
/// present in the map once exploration finishes.
fn resolve(states: HashMap<usize, State>, edges: Vec<(usize, Label, usize)>) -> (BTreeSet<State>, BTreeSet<Edge>) {
    let resolved_edges = edges
        .into_iter()
        .map(|(from, label, to)| {
            let from = states.get(&from).expect("source state was reported").clone();
            let to = states.get(&to).expect("target state was reported").clone();
            (from, label, to)
        })
        .collect();
    let states = states.into_values().collect();
    (states, resolved_edges)
}

/// Runs the sequential driver and returns its reachable states and transitions.
fn run_sequential<P>(lps: &P, strategy: ExplorationStrategy) -> (BTreeSet<State>, BTreeSet<Edge>)
where
    P: LPS<Value = usize, Label = Label, StateInfo = State>,
{
    let mut acc = Accumulator::default();
    explore(
        lps,
        strategy,
        &Timing::new(),
        &mut acc,
        |acc, index, info| {
            acc.states.insert(index.value(), info.clone());
            Ok(())
        },
        |acc, from, label, to| {
            acc.edges.push((from.value(), *label, to.value()));
            Ok(())
        },
    )
    .expect("exploration succeeds");
    resolve(acc.states, acc.edges)
}

/// Runs the parallel driver and merges the per-worker accumulators.
fn run_parallel<P>(lps: &P) -> (BTreeSet<State>, BTreeSet<Edge>)
where
    P: LPS<Value = usize, Label = Label, StateInfo = State> + Sync,
{
    let (_initial, locals) = explore_parallel(
        lps,
        Accumulator::default,
        |acc, index, info| {
            acc.states.insert(index.value(), info.clone());
            Ok(())
        },
        |acc, from, label, to| {
            acc.edges.push((from.value(), *label, to.value()));
            Ok(())
        },
    )
    .expect("parallel exploration succeeds");

    let mut states = HashMap::new();
    let mut edges = Vec::new();
    for acc in locals {
        states.extend(acc.states);
        edges.extend(acc.edges);
    }
    resolve(states, edges)
}

/// The fixtures every driver is checked against: plain grids of varying shape,
/// a grid carrying a conjunction-guarded diagonal summand, and a degenerate
/// grid whose only state is the initial one (all guards false from the start).
fn fixtures() -> Vec<MockLps> {
    vec![
        MockLps::grid(&[0]),
        MockLps::grid(&[3]),
        MockLps::grid(&[2, 3]),
        MockLps::grid(&[2, 2, 2]),
        MockLps::grid_with_diagonal(&[2, 3]),
        MockLps::grid_with_diagonal(&[3, 3, 2]),
        // Grids whose summands fan out to several successors, with a different
        // step bound per coordinate (a `0` step yields a guarded self-loop).
        MockLps::grid_with_steps(&[3], &[2]),
        MockLps::grid_with_steps(&[2, 3], &[1, 2]),
        MockLps::grid_with_steps(&[2, 2, 2], &[0, 1, 2]),
    ]
}

/// The naive baseline: a single-threaded depth-first exploration. Its reachable
/// states and transitions are what the other drivers are compared against.
fn baseline(lps: &MockLps) -> (BTreeSet<State>, BTreeSet<Edge>) {
    run_sequential(lps, ExplorationStrategy::Dfs)
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_breadth_first_matches_depth_first() {
    // The traversal order must not change the discovered state space.
    for lps in fixtures() {
        assert_eq!(
            run_sequential(&lps, ExplorationStrategy::Bfs),
            baseline(&lps),
            "BFS disagreed with DFS"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // rayon uses crossbeam-epoch which has Stacked Borrows violations under Miri
fn test_parallel_matches_sequential() {
    for lps in fixtures() {
        assert_eq!(
            run_parallel(&lps),
            baseline(&lps),
            "parallel driver disagreed with sequential"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // rayon uses crossbeam-epoch which has Stacked Borrows violations under Miri
fn test_cached_matches_sequential() {
    for strategy in [CachingStrategy::Local, CachingStrategy::None] {
        for lps in fixtures() {
            let expected = baseline(&lps);
            let cached = CacheLPS::new(lps, strategy);

            assert_eq!(
                run_sequential(&cached, ExplorationStrategy::Dfs),
                expected,
                "cached sequential driver disagreed with sequential (strategy {strategy:?})"
            );
            assert_eq!(
                run_parallel(&cached),
                expected,
                "cached parallel driver disagreed with sequential (strategy {strategy:?})"
            );
        }
    }
}

/// `explore_parallel`'s state indices come from `DiscoveredSet`, which hands
/// each worker a whole block of consecutive indices at once to keep index
/// allocation contention-free. This means that indices are likely sparse.
#[test]
#[cfg_attr(miri, ignore)] // rayon uses crossbeam-epoch which has Stacked Borrows violations under Miri
fn test_parallel_state_indices_may_be_sparse() {
    // Large enough that several workers insert states concurrently.
    let lps = MockLps::grid(&[50, 50]);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .expect("failed to build rayon pool");

    // Per-worker accumulator: every state index this worker's `on_state` saw.
    let (initial, locals) = pool
        .install(|| {
            explore_parallel(
                &lps,
                Vec::new,
                |acc: &mut Vec<usize>, index, _info: &State| {
                    acc.push(index.value());
                    Ok(())
                },
                |_acc: &mut Vec<usize>, _from, _label, _to| Ok(()),
            )
        })
        .expect("parallel exploration succeeds");

    let total: usize = locals.iter().map(Vec::len).sum();
    let all_indices: BTreeSet<usize> = locals.iter().flatten().copied().collect();
    let max_index = all_indices.iter().next_back().copied().unwrap_or(0);

    assert_eq!(total, 51 * 51, "sanity: every grid state is discovered exactly once");
    assert!(initial.value() < total, "the initial state index must be in range");
    // Every discovered state must receive its own index: two states sharing an
    // index would silently merge them in a built LTS.
    assert_eq!(
        all_indices.len(),
        total,
        "every discovered state must receive a distinct index, even though indices need not be dense"
    );
    // Indices are handed out in per-worker blocks, so the highest index seen
    // may exceed `total - 1`; it must never be lower, since every discovered
    // state needs an index.
    assert!(
        max_index + 1 >= total,
        "state indices must cover at least one index per discovered state"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // rayon uses crossbeam-epoch which has Stacked Borrows violations under Miri
fn test_permuted_matches_unpermuted() {
    // Permuting the state vector must not change the transition system, only the positions the
    // summands report it over. Every permutation of every fixture is checked, including the
    // identity, which the wrapper passes through untouched.
    for lps in fixtures() {
        let expected = baseline(&lps);
        let num_positions = lps.initial_state().len();

        for order in (0..num_positions).permutations(num_positions) {
            let permuted = PermutedLps::new(lps.clone(), order.clone()).expect("the order is a permutation");

            // The mock reports the live state vector as its `StateInfo`, and the wrapper hands the
            // wrapped LPS the un-permuted state, so the reported states are directly comparable.
            assert_eq!(
                run_sequential(&permuted, ExplorationStrategy::Dfs),
                expected,
                "permuting with {order:?} changed the transition system"
            );
            assert_eq!(
                run_parallel(&permuted),
                expected,
                "permuting with {order:?} changed the transition system (parallel driver)"
            );
        }
    }
}

#[test]
fn test_permuted_reports_permuted_positions() {
    // The metadata the wrapper exposes is in permuted positions: position `order[i]` of the
    // wrapped LPS becomes position `i`.
    let lps = MockLps::grid(&[2, 3, 4]);
    let permuted = PermutedLps::new(lps, vec![2, 0, 1]).expect("the order is a permutation");

    // The grid starts at the origin, so the initial state alone does not show the permutation;
    // the summand of coordinate `i` reads and writes it, which does.
    assert_eq!(permuted.initial_state(), vec![0, 0, 0]);
    assert_eq!(permuted.summands()[0].read_positions(), &[1]);
    assert_eq!(permuted.summands()[1].read_positions(), &[2]);
    assert_eq!(permuted.summands()[2].read_positions(), &[0]);
    assert_eq!(permuted.summands()[2].effect(), StateEffect::Positions(&[0]));

    // An order must be a permutation of the positions of the state vector.
    let lps = MockLps::grid(&[2, 3, 4]);
    assert!(PermutedLps::new(lps.clone(), vec![0, 1]).is_err());
    assert!(PermutedLps::new(lps.clone(), vec![0, 1, 1]).is_err());
    assert!(PermutedLps::new(lps, vec![0, 1, 3]).is_err());
}

#[test]
fn test_sequential_propagates_callback_error() {
    // A transition callback that always fails must surface as an error rather
    // than completing the exploration.
    let lps = MockLps::grid(&[3, 3]);
    let result = explore(
        &lps,
        ExplorationStrategy::Dfs,
        &Timing::new(),
        &mut (),
        |_ctx, _index, _info| Ok(()),
        |_ctx, _from, _label, _to| Err("boom".into()),
    );
    assert!(result.is_err(), "callback error must surface");
}

#[test]
#[cfg_attr(miri, ignore)] // rayon uses crossbeam-epoch which has Stacked Borrows violations under Miri
fn test_parallel_propagates_callback_error() {
    // Exercises the abort path: one worker's failing callback sets `aborted`,
    // and every worker stops and reports the error.
    let lps = MockLps::grid(&[3, 3]);
    let result = explore_parallel(
        &lps,
        || (),
        |_local, _index, _info| Ok(()),
        |_local, _from, _label, _to| Err("boom".into()),
    );
    assert!(result.is_err(), "callback error must surface from the parallel driver");
}

#[test]
fn test_grid_state_and_transition_counts() {
    // Anchor the naive baseline to closed-form counts, so the driver everything
    // else is compared against cannot drift undetected.
    let lps = MockLps::grid(&[2, 3]);
    let (states, edges) = baseline(&lps);
    // States: the full product grid (bound + 1 per axis).
    assert_eq!(states.len(), 3 * 4);
    // Transitions: along axis i, the source ranges over its bound values while
    // the others range freely: 2*4 + 3*3 = 17.
    assert_eq!(edges.len(), 2 * 4 + 3 * 3);
}

#[test]
fn test_stepped_grid_state_and_transition_counts() {
    // A one-dimensional grid whose only summand fires at the initial state and
    // adds any value in `0..=2`, producing three successors (`0`, `1`, `2`).
    let lps = MockLps::grid_with_steps(&[1], &[2]);
    let (states, edges) = baseline(&lps);
    // Reachable states: the initial `0` plus the two further targets `1` and `2`.
    assert_eq!(states, BTreeSet::from([vec![0], vec![1], vec![2]]));
    // Only state `0` satisfies the guard; it emits one edge per delta value.
    assert_eq!(
        edges,
        BTreeSet::from([(vec![0], 0, vec![0]), (vec![0], 0, vec![1]), (vec![0], 0, vec![2])])
    );
}
