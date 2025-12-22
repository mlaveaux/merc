use rand::Rng;

use crate::product_lts;
use crate::LabelIndex;
use crate::LabelledTransitionSystem;
use crate::LtsBuilderFast;
use crate::StateIndex;

/// Generates a random LTS with the desired number of states, labels and out
/// degree by composing three smaller random LTSs using the synchronous product.
/// This is often a more realistic structure then fully random LTSs, but
/// otherwise see [`random_lts_monolithic`].
pub fn random_lts(
    rng: &mut impl Rng,
    num_of_states: usize,
    num_of_labels: u32,
    outdegree: usize,
) -> LabelledTransitionSystem<String> {
    let components: Vec<LabelledTransitionSystem<String>> = (0..3)
        .map(|_| random_lts_monolithic(rng, num_of_states, num_of_labels, outdegree))
        .collect();

    // Synchronize on some of the labels.
    let synchronized_labels: Vec<String> = (1..num_of_labels.min(3))
        .map(|i| {
            char::from_digit(i, 36)
                .expect("Radix is less than 37, so should not panic")
                .to_string()
        })
        .collect();

    components
        .into_iter()
        .reduce(|acc, lts| product_lts(&acc, &lts, Some(synchronized_labels.clone())))
        .expect("At least one component should be present")
}

/// Generates a monolithic LTS with the desired number of states, labels, out
/// degree and in degree for all the states.
pub fn random_lts_monolithic(
    rng: &mut impl Rng,
    num_of_states: usize,
    num_of_labels: u32,
    outdegree: usize,
) -> LabelledTransitionSystem<String> {
    assert!(
        num_of_labels < 26,
        "Too many labels requested, we only support alphabetic labels."
    );

    // Introduce lower case letters for the labels.
    let mut labels: Vec<String> = Vec::new();
    labels.push("tau".to_string()); // The initial hidden label, assumed to be index 0.
    for i in 0..(num_of_labels - 1) {
        labels.push(
            char::from_digit(i, 36)
                .expect("Radix is less than 37, so should not panic")
                .to_string(),
        );
    }

    let mut builder = LtsBuilderFast::with_capacity(labels.clone(), Vec::new(), num_of_states);

    for state_index in 0..num_of_states {
        // Introduce outgoing transitions for this state based on the desired out degree.
        for _ in 0..rng.random_range(0..outdegree) {
            // Pick a random label and state.
            let label = rng.random_range(0..num_of_labels);
            let to = rng.random_range(0..num_of_states);

            builder.add_transition(
                StateIndex::new(state_index),
                &labels[label as usize],
                StateIndex::new(to),
            );
        }
    }

    if builder.num_of_states() == 0 {
        // Ensure there is at least one state (otherwise it would be an LTS without initial state).
        builder.require_num_of_states(1);
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
