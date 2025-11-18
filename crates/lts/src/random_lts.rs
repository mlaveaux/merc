use rand::Rng;
use rustc_hash::FxHashSet;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;

/// Generates a monolithic LTS with the desired number of states, labels, out
/// degree and in degree for all the states.
pub fn random_lts(
    rng: &mut impl Rng,
    num_of_states: usize,
    num_of_labels: u32,
    outdegree: usize,
) -> LabelledTransitionSystem {
    // Introduce lower case letters for the labels.
    let tau_label = "tau".to_string();

    let mut labels: Vec<String> = vec![tau_label.clone()];
    for i in 0..num_of_labels {
        labels.push(char::from_digit(i + 10, 36).unwrap().to_string());
    }

    let mut transitions: FxHashSet<(StateIndex, LabelIndex, StateIndex)> = FxHashSet::default();

    for state_index in 0..num_of_states {
        // Introduce outgoing transitions for this state based on the desired out degree.
        for _ in 0..rng.random_range(0..outdegree) {
            // Pick a random label and state.
            let label = rng.random_range(0..num_of_labels);
            let to = rng.random_range(0..num_of_states);

            transitions.insert((
                StateIndex::new(state_index),
                LabelIndex::new(label as usize),
                StateIndex::new(to),
            ));
        }
    }

    LabelledTransitionSystem::new(
        StateIndex::new(0),
        Some(num_of_states),
        || transitions.iter().cloned(),
        labels,
        vec![tau_label],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use merc_utilities::random_test;

    #[test]
    fn random_lts_test() {
        random_test(100, |rng| {
            // This test only checks the assertions of an LTS internally.
            let _lts = random_lts(rng, 10, 3, 3);
        });
    }
}
