//! Checks the refinement verdicts against a naive subset construction on small random LTSs.

use std::collections::BTreeSet;
use std::collections::VecDeque;

use rand::Rng;
use rand::RngExt;
use rand::seq::SliceRandom;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use merc_lts::mutate_lts;
use merc_lts::random_lts;
use merc_lts::read_aut;
use merc_refinement::CounterExample;
use merc_refinement::ExplorationStrategy;
use merc_refinement::RefinementType;
use merc_refinement::refines;
use merc_utilities::Timing;
use merc_utilities::random_test;
use merc_utilities::test_logger;

type Lts = LabelledTransitionSystem<String>;

const ALL: [RefinementType; 5] = [
    RefinementType::Trace,
    RefinementType::Weaktrace,
    RefinementType::StableFailures,
    RefinementType::FailuresDivergences,
    RefinementType::ImpossibleFutures,
];

/// Runs `refines` with every strategy, preprocessing flag and counter example flag, asserts
/// that they agree, and returns the verdict with a counter example when one was produced.
fn check(impl_lts: &Lts, spec_lts: &Lts, refinement: RefinementType) -> (bool, Option<CounterExample<String>>) {
    let mut timing = Timing::new();
    let mut verdict = None;
    let mut counter_example = None;
    for strategy in [ExplorationStrategy::BFS, ExplorationStrategy::DFS] {
        for preprocess in [false, true] {
            for with_counter_example in [false, true] {
                let (result, ce) = refines(
                    impl_lts.clone(),
                    spec_lts.clone(),
                    refinement,
                    strategy,
                    preprocess,
                    with_counter_example,
                    &mut timing,
                );
                assert_eq!(
                    *verdict.get_or_insert(result),
                    result,
                    "{refinement:?} {strategy:?} preprocess {preprocess} counter example {with_counter_example}"
                );
                if ce.is_some() {
                    counter_example = ce;
                }
            }
        }
    }
    (verdict.expect("at least one configuration ran"), counter_example)
}

/// Returns the labels of the transitions of `state`, with the target states.
fn steps(lts: &Lts, state: StateIndex) -> impl Iterator<Item = (&String, StateIndex)> {
    lts.outgoing_transitions(state)
        .map(move |t| (&lts.labels()[t.label.value()], t.to))
}

/// Returns `states` extended with every state reachable by tau transitions.
fn tau_closure(lts: &Lts, states: BTreeSet<StateIndex>) -> BTreeSet<StateIndex> {
    let mut result = states.clone();
    let mut todo: Vec<StateIndex> = states.into_iter().collect();
    while let Some(state) = todo.pop() {
        for t in lts.outgoing_transitions(state) {
            if lts.is_hidden_label(t.label) && result.insert(t.to) {
                todo.push(t.to);
            }
        }
    }
    result
}

/// Returns the states reached from `states` by `label`, tau-closed when `weak`.
fn after(lts: &Lts, states: &BTreeSet<StateIndex>, label: &str, weak: bool) -> BTreeSet<StateIndex> {
    let next = states
        .iter()
        .flat_map(|s| steps(lts, *s).filter(|(l, _)| *l == label).map(|(_, to)| to))
        .collect();
    if weak { tau_closure(lts, next) } else { next }
}

/// Returns the states reached by `trace` from the initial state, tau-closed when `weak`.
fn after_trace(lts: &Lts, trace: &[String], weak: bool) -> BTreeSet<StateIndex> {
    let initial = BTreeSet::from([lts.initial_state_index()]);
    let mut states = if weak { tau_closure(lts, initial) } else { initial };
    for label in trace {
        states = after(lts, &states, label, weak);
    }
    states
}

fn is_stable(lts: &Lts, state: StateIndex) -> bool {
    lts.outgoing_transitions(state).all(|t| !lts.is_hidden_label(t.label))
}

fn initials(lts: &Lts, state: StateIndex) -> BTreeSet<&String> {
    steps(lts, state).map(|(label, _)| label).collect()
}

/// Decides (weak) trace or stable failures refinement by exploring the pairs of an
/// implementation state and the set of specification states reached by the same trace.
fn naive_refines(impl_lts: &Lts, spec_lts: &Lts, refinement: RefinementType) -> bool {
    let weak = refinement != RefinementType::Trace;
    let failures = refinement == RefinementType::StableFailures;

    let initial = (impl_lts.initial_state_index(), after_trace(spec_lts, &[], weak));
    let mut visited = BTreeSet::from([initial.clone()]);
    let mut todo = VecDeque::from([initial]);
    while let Some((state, spec_states)) = todo.pop_front() {
        if failures && is_stable(impl_lts, state) {
            let offered = initials(impl_lts, state);
            let matched = spec_states
                .iter()
                .any(|q| is_stable(spec_lts, *q) && initials(spec_lts, *q).is_subset(&offered));
            if !matched {
                return false;
            }
        }

        for t in impl_lts.outgoing_transitions(state) {
            let next = if weak && impl_lts.is_hidden_label(t.label) {
                spec_states.clone()
            } else {
                after(spec_lts, &spec_states, &impl_lts.labels()[t.label.value()], weak)
            };
            if next.is_empty() {
                return false;
            }
            if visited.insert((t.to, next.clone())) {
                todo.push_back((t.to, next));
            }
        }
    }
    true
}

/// Generates an implementation for `spec`: a permutation (always refines), a small
/// mutation or an unrelated LTS.
fn implementation(rng: &mut impl Rng, spec: &Lts) -> Lts {
    match rng.random_range(0..3) {
        0 => {
            let mut permutation: Vec<StateIndex> = spec.iter_states().collect();
            permutation.shuffle(rng);
            LabelledTransitionSystem::new_from_permutation(spec.clone(), |i| permutation[i])
        }
        1 => mutate_lts(spec, rng, 2).unwrap(),
        _ => random_lts::<String, _>(rng, spec.num_of_states(), 3),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_refines_matches_oracle() {
    let mut verdicts = [0usize; 2];
    random_test(100, |rng| {
        let spec = random_lts::<String, _>(rng, 8, 3);
        let implementation = implementation(rng, &spec);

        for refinement in [
            RefinementType::Trace,
            RefinementType::Weaktrace,
            RefinementType::StableFailures,
        ] {
            let expected = naive_refines(&implementation, &spec, refinement);
            verdicts[usize::from(expected)] += 1;

            let (result, counter_example) = check(&implementation, &spec, refinement);
            assert_eq!(result, expected, "{refinement:?}");

            let weak = refinement != RefinementType::Trace;
            match counter_example {
                None => assert!(result, "{refinement:?}: a failed check must give a counter example"),
                Some(CounterExample::Trace(trace) | CounterExample::WeakTrace(trace)) => {
                    assert!(!result);
                    assert!(
                        !after_trace(&implementation, &trace, weak).is_empty(),
                        "{trace:?} is no trace of the implementation"
                    );
                    assert!(
                        after_trace(&spec, &trace, weak).is_empty(),
                        "{trace:?} is a trace of the specification"
                    );
                }
                Some(CounterExample::StableFailures(trace, refusal)) => {
                    assert!(!result);
                    let refuses = |lts: &Lts, state: StateIndex| {
                        is_stable(lts, state) && initials(lts, state).iter().all(|l| !refusal.contains(l))
                    };
                    assert!(
                        after_trace(&implementation, &trace, true)
                            .iter()
                            .any(|s| refuses(&implementation, *s))
                    );
                    assert!(!after_trace(&spec, &trace, true).iter().any(|s| refuses(&spec, *s)));
                }
                Some(_) => panic!("{refinement:?}: unexpected kind of counter example"),
            }
        }
    });
    assert!(
        verdicts[0] > 0 && verdicts[1] > 0,
        "both verdicts must occur: {verdicts:?}"
    );
}

/// Every LTS refines itself up to renumbering, and impossible futures refinement implies
/// weak trace refinement.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_refines_properties() {
    random_test(100, |rng| {
        let spec = random_lts::<String, _>(rng, 8, 3);

        let mut permutation: Vec<StateIndex> = spec.iter_states().collect();
        permutation.shuffle(rng);
        let permuted = LabelledTransitionSystem::new_from_permutation(spec.clone(), |i| permutation[i]);
        for refinement in ALL {
            assert!(check(&permuted, &spec, refinement).0, "{refinement:?} is not reflexive");
        }

        let implementation = implementation(rng, &spec);
        if check(&implementation, &spec, RefinementType::ImpossibleFutures).0 {
            assert!(naive_refines(&implementation, &spec, RefinementType::Weaktrace));
        }
    });
}

#[test]
fn test_example_2_12() {
    test_logger();

    let s0 = read_aut(
        br#"des(0, 6, 5)
            (0, "req", 1)
            (1, "i", 2)
            (2, "10", 3)
            (3, "10", 0)
            (1, "i", 5)
            (5, "20", 0)"# as &[u8],
    )
    .unwrap();
    let t0 = read_aut(
        br#"des(0, 3, 2)
            (0, "req", 1)
            (1, "20", 2)"# as &[u8],
    )
    .unwrap();
    let u0 = read_aut(
        br#"des(0, 4, 3)
            (0, "req", 1)
            (1, "i", 1)
            (1, "20", 2)
            (2, "i", 0)"# as &[u8],
    )
    .unwrap();

    assert!(check(&t0, &s0, RefinementType::Weaktrace).0);
    assert!(!check(&t0, &s0, RefinementType::StableFailures).0);
    assert!(check(&u0, &s0, RefinementType::StableFailures).0);
}

#[test]
fn test_impossible_futures_example() {
    test_logger();

    let impl_lts = read_aut(
        b"des (0,8,6)
            (0,newday,1)
            (1,i,2)
            (1,i,3)
            (2,teach,4)
            (3,lindyhop,0)
            (3,i,5)
            (4,newday,2)
            (5,teach,0)" as &[u8],
    )
    .unwrap();

    let spec_lts = read_aut(
        b"des (0,5,4)
            (0,newday,1)
            (1,i,2)
            (1,i,3)
            (2,teach,0)
            (3,lindyhop,0)" as &[u8],
    )
    .unwrap();

    // Weak trace equivalent, but the specification does not refine the implementation
    // modulo impossible futures.
    assert!(check(&impl_lts, &spec_lts, RefinementType::Weaktrace).0);
    assert!(check(&spec_lts, &impl_lts, RefinementType::Weaktrace).0);
    assert!(!check(&spec_lts, &impl_lts, RefinementType::ImpossibleFutures).0);
}
