use merc_lps::read_lps_file;

#[test]
fn reads_a_simple_counter() {
    let lps = read_lps_file("tests/data/counter.lps").expect("must read the counter fixture");

    // proc P(n: Nat) = (n < 3) -> a(n) . P(n+1) <> delta; init P(0);
    assert_eq!(lps.process.parameters.len(), 1);
    // The linearizer suffixes parameter names with the source process's name.
    assert_eq!(lps.process.parameters[0].name(), "n_P");
    assert_eq!(lps.action_labels.len(), 1);
    assert_eq!(lps.action_labels[0].name().value(), "a");

    assert_eq!(lps.initial_process.expressions().to_vec().len(), 1);

    // One guarded action summand and one deadlock summand (the `<> delta` branch).
    assert_eq!(lps.process.action_summands.len(), 1);
    assert_eq!(lps.process.deadlock_summands.len(), 1);

    let summand = &lps.process.action_summands[0];
    assert!(summand.summation_variables.is_empty());
    assert_eq!(summand.actions.len(), 1);
    assert!(summand.time.is_none(), "the specification is untimed");
    assert_eq!(summand.assignments.len(), 1, "only `n` is written");
}

#[test]
fn reads_a_bounded_sum() {
    let lps = read_lps_file("tests/data/sum_bounded.lps").expect("must read the sum_bounded fixture");

    // proc P(n: Nat) = sum m: Nat . (m < 3 && n < 5) -> a(m) . P(n+1);
    assert_eq!(lps.process.parameters.len(), 1);
    assert_eq!(lps.process.action_summands.len(), 1);

    let summand = &lps.process.action_summands[0];
    assert_eq!(summand.summation_variables.len(), 1, "the `sum m: Nat` variable");
    assert_eq!(summand.summation_variables[0].name(), "m_P");
}

#[test]
fn reads_multiple_summands() {
    let lps = read_lps_file("tests/data/multi.lps").expect("must read the multi fixture");

    assert_eq!(lps.process.parameters.len(), 2);
    assert_eq!(lps.action_labels.len(), 2);

    // Three guarded branches in the source, all without sum variables.
    assert_eq!(
        lps.process.action_summands.len() + lps.process.deadlock_summands.len(),
        3
    );
    for summand in &lps.process.action_summands {
        assert!(summand.summation_variables.is_empty());
    }
}
