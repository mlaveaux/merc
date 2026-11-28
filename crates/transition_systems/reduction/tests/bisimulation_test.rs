//! Checks the reductions, comparisons and partitions against the textbook definitions of
//! strong, weak and branching bisimulation, computed as a greatest fixpoint over pairs of states.

use rand::Rng;
use rand::RngExt;
use rand::seq::SliceRandom;

use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use merc_lts::TransitionLabel;
use merc_lts::mutate_lts;
use merc_lts::random_lts;
use merc_reduction::DistinguishingFormula;
use merc_reduction::Equivalence;
use merc_reduction::Partition;
use merc_reduction::branching_bisim_sigref;
use merc_reduction::compare_lts;
use merc_reduction::reduce_lts;
use merc_reduction::strong_bisim_sigref;
use merc_utilities::Timing;
use merc_utilities::random_test;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Strong,
    Weak,
    Branching,
}

/// The equivalences without divergence preservation, with the bisimulation they compute.
const EQUIVALENCES: [(Equivalence, Kind); 8] = [
    (Equivalence::StrongBisim, Kind::Strong),
    (Equivalence::StrongBisimNaive, Kind::Strong),
    (Equivalence::BranchingBisim, Kind::Branching),
    (Equivalence::BranchingBisimNaive, Kind::Branching),
    (Equivalence::WeakBisim, Kind::Weak),
    (Equivalence::WeakBisimParallel, Kind::Weak),
    (Equivalence::WeakBisimSigref, Kind::Weak),
    (Equivalence::WeakBisimSigrefNaive, Kind::Weak),
];

/// Returns true iff the algorithm for `equivalence` can run under miri: the `WeakBisim`
/// algorithms use bitvec, which violates Stacked Borrows.
fn runs_under_miri(equivalence: Equivalence) -> bool {
    !cfg!(miri)
        || !matches!(
            equivalence,
            Equivalence::WeakBisim
                | Equivalence::WeakBisimParallel
                | Equivalence::WeakBisimDivergencePreserving
                | Equivalence::WeakBisimParallelDivergencePreserving
        )
}

/// Returns the `preprocess` flags to test for `equivalence`.
///
/// `WeakBisim` and `WeakBisimParallel` are not tested with preprocessing: they look up the
/// given state of the input in the branching-reduced LTS, which panics or compares the wrong
/// states once the reduction removes states.
fn preprocess_flags(equivalence: Equivalence) -> &'static [bool] {
    match equivalence {
        Equivalence::WeakBisim | Equivalence::WeakBisimParallel => &[false],
        _ => &[false, true],
    }
}

/// The divergence-preserving equivalences, grouped by the bisimulation they refine.
const DIVERGENCE_PRESERVING: [(Kind, &[Equivalence]); 2] = [
    (
        Kind::Branching,
        &[
            Equivalence::BranchingBisimDivergencePreserving,
            Equivalence::BranchingBisimDivergencePreservingNaive,
        ],
    ),
    (
        Kind::Weak,
        &[
            Equivalence::WeakBisimDivergencePreserving,
            Equivalence::WeakBisimParallelDivergencePreserving,
            Equivalence::WeakBisimSigrefDivergencePreserving,
            Equivalence::WeakBisimSigrefNaiveDivergencePreserving,
        ],
    ),
];

/// Returns `closure[s][t]` iff `s` reaches `t` with zero or more tau transitions.
fn tau_closure<L: LTS>(lts: &L) -> Vec<Vec<bool>> {
    let n = lts.num_of_states();
    let mut closure = vec![vec![false; n]; n];
    for s in lts.iter_states() {
        closure[s][s] = true;
        for t in lts.outgoing_transitions(s) {
            if lts.is_hidden_label(t.label) {
                closure[s][t.to] = true;
            }
        }
    }
    for k in 0..n {
        let via = closure[k].clone();
        for row in closure.iter_mut().filter(|row| row[k]) {
            for (reach, &via) in row.iter_mut().zip(&via) {
                *reach |= via;
            }
        }
    }
    closure
}

/// Returns `related[s][t]` iff `s` and `t` are bisimilar of the given kind: the largest
/// symmetric relation in which every step of one state is matched by the other.
fn bisimilar<L: LTS>(lts: &L, kind: Kind) -> Vec<Vec<bool>> {
    let n = lts.num_of_states();
    let closure = tau_closure(lts);
    let steps: Vec<Vec<(LabelIndex, usize)>> = lts
        .iter_states()
        .map(|s| lts.outgoing_transitions(s).map(|t| (t.label, t.to.value())).collect())
        .collect();
    let is_tau = |label: LabelIndex| lts.is_hidden_label(label);

    // The weak steps s =a=> t: tau* a tau* for visible a, and tau* for tau.
    let weak_steps: Vec<Vec<(LabelIndex, usize)>> = (0..n)
        .map(|s| {
            let mut result: Vec<(LabelIndex, usize)> = Vec::new();
            for t1 in (0..n).filter(|&t1| closure[s][t1]) {
                result.push((LabelIndex::new(0), t1));
                for &(label, t2) in steps[t1].iter().filter(|(label, _)| !is_tau(*label)) {
                    result.extend((0..n).filter(|&t| closure[t2][t]).map(|t| (label, t)));
                }
            }
            result.sort();
            result.dedup();
            result
        })
        .collect();

    let mut related = vec![vec![true; n]; n];
    let simulates = |related: &Vec<Vec<bool>>, s: usize, t: usize| {
        steps[s].iter().all(|&(a, s2)| match kind {
            Kind::Strong => steps[t].iter().any(|&(b, t2)| a == b && related[s2][t2]),
            Kind::Weak => {
                let a = if is_tau(a) { LabelIndex::new(0) } else { a };
                weak_steps[t].iter().any(|&(b, t2)| a == b && related[s2][t2])
            }
            Kind::Branching => {
                (is_tau(a) && related[s2][t])
                    || (0..n).any(|t1| {
                        closure[t][t1] && related[s][t1] && steps[t1].iter().any(|&(b, t2)| a == b && related[s2][t2])
                    })
            }
        })
    };

    let mut changed = true;
    while changed {
        changed = false;
        for s in 0..n {
            for t in 0..n {
                if related[s][t] && !(simulates(&related, s, t) && simulates(&related, t, s)) {
                    related[s][t] = false;
                    related[t][s] = false;
                    changed = true;
                }
            }
        }
    }
    related
}

/// Returns the number of equivalence classes of `related`.
fn num_of_classes(related: &[Vec<bool>]) -> usize {
    (0..related.len()).filter(|&s| (0..s).all(|t| !related[s][t])).count()
}

/// Asserts that the partition puts two states in the same block iff `related` relates them.
fn assert_partition_matches<P: Partition>(partition: &P, related: &[Vec<bool>]) {
    for (s, row) in related.iter().enumerate() {
        for (t, &expected) in row.iter().enumerate() {
            assert_eq!(
                partition.block_number(StateIndex::new(s)) == partition.block_number(StateIndex::new(t)),
                expected,
                "states {s} and {t}"
            );
        }
    }
}

/// Generates a second LTS for `left` that is a permutation (always bisimilar), a small
/// mutation (sometimes bisimilar) or unrelated.
fn related_lts(rng: &mut impl Rng, left: &LabelledTransitionSystem<String>) -> LabelledTransitionSystem<String> {
    match rng.random_range(0..3) {
        0 => {
            let mut permutation: Vec<StateIndex> = left.iter_states().collect();
            permutation.shuffle(rng);
            LabelledTransitionSystem::new_from_permutation(left.clone(), |i| permutation[i])
        }
        1 => mutate_lts(left, rng, 2).unwrap(),
        _ => random_lts::<String, _>(rng, left.num_of_states(), 3),
    }
}

/// Returns the oracle verdict of `kind` for the initial states of `left` and `right`.
fn oracle(left: &LabelledTransitionSystem<String>, right: &LabelledTransitionSystem<String>, kind: Kind) -> bool {
    let (merged, right_initial) = left.clone().merge_disjoint(right);
    bisimilar(&merged, kind)[merged.initial_state_index()][right_initial]
}

/// The quotient is equivalent to the input, and no two of its states are equivalent.
fn check_reduce(lts: &LabelledTransitionSystem<String>, equivalence: Equivalence, kind: Kind, preprocess: bool) {
    let timing = Timing::new();
    let reduced = reduce_lts(lts.clone(), equivalence, preprocess, &timing);

    let (merged, reduced_initial) = lts.clone().merge_disjoint(&reduced);
    let related = bisimilar(&merged, kind);
    assert!(
        related[merged.initial_state_index()][reduced_initial],
        "{equivalence:?} (preprocess {preprocess}): quotient is not equivalent to the input"
    );

    let offset = lts.num_of_states();
    for p in reduced.iter_states() {
        for q in reduced.iter_states().filter(|q| *q != p) {
            assert!(
                !related[offset + p.value()][offset + q.value()],
                "{equivalence:?} (preprocess {preprocess}): quotient states {p} and {q} are equivalent"
            );
        }
    }
}

/// A divergence-preserving quotient lies between the strong and the non-divergence-preserving
/// quotient, is a fixpoint of the reduction, and every algorithm of the family accepts it.
fn check_reduce_divergence_preserving(lts: &LabelledTransitionSystem<String>, kind: Kind, family: &[Equivalence]) {
    let timing = Timing::new();
    let num_of_strong_classes = num_of_classes(&bisimilar(lts, Kind::Strong));
    let family: Vec<Equivalence> = family.iter().copied().filter(|e| runs_under_miri(*e)).collect();

    for &equivalence in &family {
        let reduced = reduce_lts(lts.clone(), equivalence, false, &timing);

        assert!(
            oracle(lts, &reduced, kind),
            "{equivalence:?}: quotient is not {kind:?} bisimilar"
        );
        assert!(
            reduced.num_of_states() <= num_of_strong_classes,
            "{equivalence:?}: quotient has more states than the strong quotient"
        );
        assert_eq!(
            reduce_lts(reduced.clone(), equivalence, false, &timing).num_of_states(),
            reduced.num_of_states(),
            "{equivalence:?}: reducing the quotient again shrinks it"
        );
        for &other in &family {
            assert!(
                compare_lts(other, lts.clone(), reduced.clone(), false, false, &timing).0,
                "{other:?} rejects the {equivalence:?} quotient"
            );
        }
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_reduce_matches_oracle() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 10, 3);
        for (equivalence, kind) in EQUIVALENCES {
            for &preprocess in preprocess_flags(equivalence) {
                check_reduce(&lts, equivalence, kind, preprocess);
            }
        }
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_reduce_divergence_preserving() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 10, 3);
        for (kind, family) in DIVERGENCE_PRESERVING {
            check_reduce_divergence_preserving(&lts, kind, family);
        }
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_compare_matches_oracle() {
    let mut verdicts = [0usize; 2];
    random_test(100, |rng| {
        let timing = Timing::new();
        let left = random_lts::<String, _>(rng, 10, 3);
        let right = related_lts(rng, &left);

        for (equivalence, kind) in EQUIVALENCES {
            let expected = oracle(&left, &right, kind);
            verdicts[usize::from(expected)] += 1;
            for &preprocess in preprocess_flags(equivalence) {
                assert_eq!(
                    compare_lts(equivalence, left.clone(), right.clone(), preprocess, false, &timing).0,
                    expected,
                    "{equivalence:?} (preprocess {preprocess})"
                );
            }
        }
    });
    assert!(
        verdicts[0] > 0 && verdicts[1] > 0,
        "both verdicts must occur: {verdicts:?}"
    );
}

/// All algorithms of a divergence-preserving family agree, and their verdict lies between
/// strong bisimilarity and the bisimilarity they refine.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_compare_divergence_preserving() {
    random_test(100, |rng| {
        let timing = Timing::new();
        let left = random_lts::<String, _>(rng, 10, 3);
        let right = related_lts(rng, &left);
        let strong = oracle(&left, &right, Kind::Strong);

        for (kind, family) in DIVERGENCE_PRESERVING {
            let weaker = oracle(&left, &right, kind);
            let verdicts: Vec<bool> = family
                .iter()
                .map(|&equivalence| compare_lts(equivalence, left.clone(), right.clone(), false, false, &timing).0)
                .collect();

            assert!(
                verdicts.iter().all(|v| *v == verdicts[0]),
                "{family:?} disagree: {verdicts:?}"
            );
            assert!(!strong || verdicts[0], "{kind:?}: strongly bisimilar LTSs are rejected");
            assert!(
                !verdicts[0] || weaker,
                "{kind:?}: accepted LTSs that are not {kind:?} bisimilar"
            );
        }
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_strong_bisim_sigref_partition() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 30, 3);
        let expected = bisimilar(&lts, Kind::Strong);

        let (result_lts, partition) = strong_bisim_sigref(lts, &Timing::new());

        assert_partition_matches(&partition, &expected);
        assert_eq!(result_lts.num_of_states(), expected.len());
    });
}

/// The partition is branching bisimilarity on the returned (preprocessed) LTS, and the
/// returned state is branching bisimilar to the given one.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_branching_bisim_sigref_partition() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 20, 3);
        let initial = lts.initial_state_index();

        let (preprocessed, mapped, partition) = branching_bisim_sigref(lts.clone(), initial, false, &Timing::new());
        assert_partition_matches(&partition, &bisimilar(&preprocessed, Kind::Branching));

        // The states of the right LTS are offset by the number of states of the left one.
        let (merged, _) = lts.clone().merge_disjoint(&preprocessed);
        assert!(bisimilar(&merged, Kind::Branching)[initial][lts.num_of_states() + mapped.value()]);

        // Divergence preservation refines branching bisimilarity and is implied by strong bisimilarity.
        let (preprocessed, _, partition) = branching_bisim_sigref(lts, initial, true, &Timing::new());
        let branching = bisimilar(&preprocessed, Kind::Branching);
        let strong = bisimilar(&preprocessed, Kind::Strong);
        for s in preprocessed.iter_states() {
            for t in preprocessed.iter_states() {
                let same = partition.block_number(s) == partition.block_number(t);
                assert!(!strong[s][t] || same, "strongly bisimilar {s} and {t} are separated");
                assert!(!same || branching[s][t], "{s} and {t} are not branching bisimilar");
            }
        }
    });
}

/// Returns true iff `formula` holds in `state`.
fn holds<L: LTS>(lts: &L, state: StateIndex, formula: &DistinguishingFormula<L::Label>) -> bool {
    match formula {
        DistinguishingFormula::Diamond { label, conjuncts } => lts
            .outgoing_transitions(state)
            .any(|t| &lts.labels()[t.label.value()] == label && conjuncts.iter().all(|c| holds(lts, t.to, c))),
        DistinguishingFormula::Negate(inner) => !holds(lts, state, inner),
    }
}

/// A counter example is given exactly when the LTSs are not strongly bisimilar, and it
/// holds in the left initial state but not in the right one.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_counter_example() {
    random_test(100, |rng| {
        let left = random_lts::<String, _>(rng, 10, 3);
        let right = related_lts(rng, &left);

        let (equivalent, formula) = compare_lts(
            Equivalence::StrongBisimNaive,
            left.clone(),
            right.clone(),
            false,
            true,
            &Timing::new(),
        );

        assert_eq!(equivalent, oracle(&left, &right, Kind::Strong));
        match formula {
            None => assert!(equivalent, "a counter example is required when the LTSs differ"),
            Some(formula) => {
                assert!(!equivalent);
                assert!(
                    holds(&left, left.initial_state_index(), &formula),
                    "{formula} fails in left"
                );
                assert!(
                    !holds(&right, right.initial_state_index(), &formula),
                    "{formula} holds in right"
                );
            }
        }
    });
}

/// Builds an LTS from `(from, label, to)` triples, with label 0 the hidden label.
fn small_lts(transitions: &[(usize, usize, usize)]) -> LabelledTransitionSystem<String> {
    let transitions: Vec<_> = transitions
        .iter()
        .map(|&(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)))
        .collect();
    LabelledTransitionSystem::new(
        StateIndex::new(0),
        None,
        || transitions.iter().cloned(),
        vec![String::tau_label(), "a".to_string()],
    )
}

/// A small deterministic instance for miri: 0 -a-> 1, 0 -a-> 2, 1 -a-> 3, 2 -a-> 3, where
/// 1 and 2 are strongly bisimilar.
#[test]
fn test_strong_bisim_sigref_small() {
    let lts = small_lts(&[(0, 1, 1), (0, 1, 2), (1, 1, 3), (2, 1, 3)]);

    let (_, partition) = strong_bisim_sigref(lts, &Timing::new());

    assert_eq!(partition.num_of_blocks(), 3);
    let block = |s| partition.block_number(StateIndex::new(s));
    assert_eq!(block(1), block(2));
    assert_ne!(block(0), block(1));
    assert_ne!(block(0), block(3));
    assert_ne!(block(1), block(3));
}

/// 0 -tau-> 1 -a-> 2 and 0 -a-> 2: 0 and 1 are weakly and branching bisimilar but not
/// strongly bisimilar.
#[test]
fn test_reduce_small() {
    let lts = small_lts(&[(0, 0, 1), (1, 1, 2), (0, 1, 2)]);
    let timing = Timing::new();

    for (equivalence, kind) in EQUIVALENCES.into_iter().filter(|(e, _)| runs_under_miri(*e)) {
        let expected = if kind == Kind::Strong { 3 } else { 2 };
        assert_eq!(
            reduce_lts(lts.clone(), equivalence, false, &timing).num_of_states(),
            expected,
            "{equivalence:?}"
        );
    }
}

/// Every algorithm on a few tiny LTSs, small enough for miri to cover the arena reuse in the
/// signature refinement.
#[test]
fn test_random_reduce_tiny() {
    random_test(3, |rng| {
        let lts = random_lts::<String, _>(rng, 6, 3);
        for (equivalence, kind) in EQUIVALENCES.into_iter().filter(|(e, _)| runs_under_miri(*e)) {
            check_reduce(&lts, equivalence, kind, false);
        }
        for (kind, family) in DIVERGENCE_PRESERVING {
            check_reduce_divergence_preserving(&lts, kind, family);
        }
    });
}
