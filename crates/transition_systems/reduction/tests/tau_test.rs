//! Checks the tau-SCC decomposition and divergence against naive reachability.

use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;
use merc_lts::TransitionLabel;
use merc_lts::random_lts;
use merc_reduction::Partition;
use merc_reduction::diverges;
use merc_reduction::tau_scc_decomposition;
use merc_reduction::tau_scc_decomposition_iterative;
use merc_utilities::random_test;
use test_log::test;

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

/// Builds an LTS from `(from, label, to)` triples, with label 0 the hidden label.
fn small_lts(num_of_states: usize, transitions: &[(usize, usize, usize)]) -> LabelledTransitionSystem<String> {
    let transitions: Vec<_> = transitions
        .iter()
        .map(|&(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)))
        .collect();
    LabelledTransitionSystem::new(
        StateIndex::new(0),
        Some(num_of_states),
        || transitions.iter().cloned(),
        vec![String::tau_label(), "a".to_string()],
    )
}

/// Asserts that two states share a block iff they reach each other with tau transitions.
fn assert_tau_sccs<L: LTS, P: Partition>(lts: &L, partition: &P) {
    let closure = tau_closure(lts);
    for s in lts.iter_states() {
        for t in lts.iter_states() {
            assert_eq!(
                partition.block_number(s) == partition.block_number(t),
                closure[s][t] && closure[t][s],
                "states {s} and {t}"
            );
        }
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_random_tau_scc_decomposition() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 50, 2);

        assert_tau_sccs(&lts, &tau_scc_decomposition(&lts));
        assert_tau_sccs(&lts, &tau_scc_decomposition_iterative(&lts));
    });
}

#[test]
fn test_tau_scc_decomposition_small() {
    // 0, 1 and 2 form a tau cycle, 3 has an `a` loop through 4 and 4 is reached by tau only.
    let lts = small_lts(
        5,
        &[
            (0, 0, 2),
            (0, 0, 4),
            (1, 0, 0),
            (2, 0, 1),
            (2, 0, 0),
            (3, 1, 4),
            (4, 1, 3),
        ],
    );

    for partition in [tau_scc_decomposition(&lts), tau_scc_decomposition_iterative(&lts)] {
        assert_eq!(partition.num_of_blocks(), 3);
        assert_tau_sccs(&lts, &partition);
    }
}

/// A state diverges iff it reaches, with tau transitions, a state on a tau cycle.
#[test]
#[cfg_attr(miri, ignore)]
fn test_random_diverges() {
    random_test(100, |rng| {
        let lts = random_lts::<String, _>(rng, 30, 3);
        let closure = tau_closure(&lts);
        let on_cycle: Vec<bool> = lts
            .iter_states()
            .map(|s| {
                lts.outgoing_transitions(s)
                    .any(|t| lts.is_hidden_label(t.label) && closure[t.to][s])
            })
            .collect();

        for s in lts.iter_states() {
            let expected = lts.iter_states().any(|t| closure[s][t] && on_cycle[t]);
            assert_eq!(diverges(&lts, s), expected, "state {s}");
        }
    });
}

#[test]
fn test_diverges_small() {
    // A tau diamond 0 -> {1, 2} -> 3 without a cycle, then 4 -a-> 5 -tau-> 5 and 6 -tau-> 4.
    let lts = small_lts(
        7,
        &[
            (0, 0, 1),
            (0, 0, 2),
            (1, 0, 3),
            (2, 0, 3),
            (4, 1, 5),
            (5, 0, 5),
            (6, 0, 4),
        ],
    );

    let expected = [false, false, false, false, false, true, false];
    for (s, expected) in expected.into_iter().enumerate() {
        assert_eq!(diverges(&lts, StateIndex::new(s)), expected, "state {s}");
    }
}
