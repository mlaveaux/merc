//! Property-based checks on `refines` that must hold regardless of the
//! specific refinement algorithm's internals: identical inputs must refine
//! themselves, and options that are documented as pure optimisations
//! (`preprocess`, `ExplorationStrategy`, requesting a counter example) must
//! never change the boolean verdict.

use merc_lts::mutate_lts;
use merc_lts::random_lts;
use merc_lts::write_mcrl2_aut;
use merc_refinement::ExplorationStrategy;
use merc_refinement::RefinementType;
use merc_refinement::refines;
use merc_utilities::Timing;
use merc_utilities::random_test;
use rand::RngExt;

/// Renders `lts` as a `.aut` file (in memory) for failure messages, since
/// `LabelledTransitionSystem` does not implement `Debug`.
fn describe(lts: &merc_lts::LabelledTransitionSystem<String>) -> String {
    let mut buffer = Vec::new();
    write_mcrl2_aut(&mut buffer, lts).expect("writing to a Vec<u8> cannot fail");
    String::from_utf8(buffer).expect("aut output is ASCII")
}

const REFINEMENT_TYPES: [RefinementType; 5] = [
    RefinementType::Trace,
    RefinementType::Weaktrace,
    RefinementType::StableFailures,
    RefinementType::FailuresDivergences,
    RefinementType::ImpossibleFutures,
];

/// A labelled transition system trivially refines an identical copy of
/// itself under every refinement relation: the identity relation on states
/// witnesses trace, failures, divergence and impossible-futures inclusion
/// alike. This must hold regardless of `preprocess`, `strategy` or whether a
/// counter example is requested.
#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn refines_is_reflexive_on_random_ltss() {
    random_test(200, |rng| {
        let num_states = rng.random_range(1..12);
        let num_labels = rng.random_range(1..5);
        let lts = random_lts::<String, _>(rng, num_states, num_labels);

        for &refinement in &REFINEMENT_TYPES {
            for preprocess in [false, true] {
                for strategy in [ExplorationStrategy::BFS, ExplorationStrategy::DFS] {
                    let mut timing = Timing::new();
                    let (result, _) = refines(
                        lts.clone(),
                        lts.clone(),
                        refinement,
                        strategy,
                        preprocess,
                        false,
                        &mut timing,
                    );

                    assert!(
                        result,
                        "A system must refine an identical copy of itself under {refinement:?} \
                         (preprocess={preprocess}, strategy={strategy:?}), but refines() returned false.\n\
                         LTS:\n{}",
                        describe(&lts)
                    );
                }
            }
        }
    });
}

/// `preprocess` is documented purely as a performance optimisation ("the
/// refinement checks often involve product constructions, and reducing the
/// state space beforehand can lead to significant performance
/// improvements") — it must never change the boolean verdict. Likewise
/// `ExplorationStrategy` only affects which counter example is found, not
/// whether one exists. Check both invariants together on mutated
/// (non-identical) impl/spec pairs so the checks actually explore
/// non-trivial state spaces instead of only the reflexive case above.
#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn refines_verdict_is_independent_of_preprocess_and_strategy() {
    random_test(200, |rng| {
        let num_states = rng.random_range(1..10);
        let num_labels = rng.random_range(1..4);
        let spec_lts = random_lts::<String, _>(rng, num_states, num_labels);
        let num_mutations = rng.random_range(0..10);
        let impl_lts = mutate_lts(&spec_lts, rng, num_mutations).expect("mutate_lts should not fail on a random LTS");

        for &refinement in &REFINEMENT_TYPES {
            let mut results = Vec::new();
            for preprocess in [false, true] {
                for strategy in [ExplorationStrategy::BFS, ExplorationStrategy::DFS] {
                    let mut timing = Timing::new();
                    let (result, _) = refines(
                        impl_lts.clone(),
                        spec_lts.clone(),
                        refinement,
                        strategy,
                        preprocess,
                        false,
                        &mut timing,
                    );
                    results.push(((preprocess, strategy), result));
                }
            }

            let (_, first_result) = results[0];
            assert!(
                results.iter().all(|(_, result)| *result == first_result),
                "refines() disagreed across preprocess/strategy combinations for {refinement:?}: {results:?}\n\
                 impl:\n{}\nspec:\n{}",
                describe(&impl_lts),
                describe(&spec_lts)
            );
        }
    });
}
