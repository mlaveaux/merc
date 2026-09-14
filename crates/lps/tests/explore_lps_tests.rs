//! Integration tests driving `merc_explore::explore` over a native
//! [`ExploreLinearProcessSpecification`] built from a real `.lps` fixture.

use merc_data::Mcrl2DataSpecification;
use merc_explore::ExplorationStrategy;
use merc_explore::explore;
use merc_lps::ExploreLinearProcessSpecification;
use merc_lps::read_lps_file;
use merc_utilities::Timing;

#[test]
fn explores_the_bounded_counter() {
    // proc P(n: Nat) = (n < 3) -> a(n) . P(n+1) <> delta; init P(0);
    //
    // States: n = 0, 1, 2, 3 (four states); the n = 3 state only enables the
    // deadlock summand, so exploration stops there. Three `a` transitions
    // connect them in a single chain.
    let lps = read_lps_file("tests/data/counter.lps").expect("must read the counter fixture");
    let native = ExploreLinearProcessSpecification::new(lps);

    let mut state_count = 0usize;
    let mut transition_count = 0usize;

    explore(
        &native,
        ExplorationStrategy::Bfs,
        &Timing::new(),
        &mut (),
        |_ctx, _state, _info| {
            state_count += 1;
            Ok(())
        },
        |_ctx, _from, _label, _to| {
            transition_count += 1;
            Ok(())
        },
    )
    .expect("exploration must succeed");

    assert_eq!(state_count, 4, "n = 0, 1, 2, 3");
    assert_eq!(transition_count, 3, "one `a` transition per non-terminal state");
}

#[test]
fn explores_a_bounded_sum_variable() {
    // proc P(n: Nat) = sum m: Nat . (m < 3 && n < 5) -> a(m) . P(n+1);
    //
    // From n, the sum variable m ranges over {0, 1, 2} whenever n < 5, giving
    // three outgoing transitions per state; n counts 0..=5 (six states, the
    // last with no outgoing transitions since n < 5 fails).
    let lps = read_lps_file("tests/data/sum_bounded.lps").expect("must read the sum_bounded fixture");
    let native = ExploreLinearProcessSpecification::new(lps);

    let mut state_count = 0usize;
    let mut transition_count = 0usize;

    explore(
        &native,
        ExplorationStrategy::Bfs,
        &Timing::new(),
        &mut (),
        |_ctx, _state, _info| {
            state_count += 1;
            Ok(())
        },
        |_ctx, _from, _label, _to| {
            transition_count += 1;
            Ok(())
        },
    )
    .expect("exploration must succeed");

    assert_eq!(state_count, 6, "n = 0..=5");
    assert_eq!(
        transition_count, 15,
        "3 sum-variable solutions for each of the 5 states with n < 5"
    );
}

#[test]
fn undecidable_summand_condition_is_an_error_not_a_missing_transition() {
    // The same `(n < 3) -> a(n)` fixture, but with every equation stripped
    // from its data specification, so the guard can no longer rewrite to
    // `true` or `false`.
    //
    // Dropping such a branch would leave a state space that is missing
    // transitions with nothing to say so, which `docs/enumeration-crate-plan.md`
    // §4.6 rules out: exploration must fail instead.
    let mut lps = read_lps_file("tests/data/counter.lps").expect("must read the counter fixture");
    lps.data_spec = Mcrl2DataSpecification::new(
        lps.data_spec.sorts().to_vec(),
        lps.data_spec.aliases().to_vec(),
        lps.data_spec.constructors().to_vec(),
        lps.data_spec.mappings().to_vec(),
        Vec::new(),
    );

    let native = ExploreLinearProcessSpecification::new(lps);
    let error = explore(
        &native,
        ExplorationStrategy::Bfs,
        &Timing::new(),
        &mut (),
        |_ctx, _state, _info| Ok(()),
        |_ctx, _from, _label, _to| Ok(()),
    )
    .expect_err("an undecidable guard must abort exploration, not silently drop transitions");

    let message = error.to_string();
    assert!(
        message.contains("rather than true or false"),
        "the error must name the undecided condition, got: {message}"
    );
}
