use rand::Rng;

use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::StateIndex;
use crate::lts_builder;

/// Generates a monolithic LTS with the desired number of states, labels, out
/// degree and in degree for all the states.
pub fn random_lts(
    rng: &mut impl Rng,
    num_of_states: usize,
    num_of_labels: u32,
    outdegree: usize,
) -> LabelledTransitionSystem {
    // Introduce lower case letters for the labels.
    let mut labels: Vec<String> = Vec::new();
    for i in 0..num_of_labels {
        labels.push(char::from_digit(i + 10, 36).unwrap().to_string());
    }

    let mut builder: lts_builder::LtsBuilder = lts_builder::LtsBuilder::with_capacity(
        labels,
        Vec::new(),
        num_of_states,
        num_of_labels as usize,
        num_of_states * outdegree,
    );

    for state_index in 0..num_of_states {
        // Introduce outgoing transitions for this state based on the desired out degree.
        for _ in 0..rng.random_range(0..outdegree) {
            // Pick a random label and state.
            let label = rng.random_range(0..num_of_labels);
            let to = rng.random_range(0..num_of_states);

            builder.add_transition_index(
                StateIndex::new(state_index),
                LabelIndex::new(label as usize),
                StateIndex::new(to),
            );
        }
    }

    builder.finish(StateIndex::new(0), true)
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
