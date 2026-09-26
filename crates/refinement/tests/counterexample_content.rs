//! Checks what a `CounterExample`'s trace actually contains when a weak
//! refinement check fails, since the type's own doc comment says a weak
//! trace is `<tau*.a0.tau*.a1. ... .a_n.tau*>true` — implicit taus, not
//! explicit ones.

use merc_lts::read_aut;
use merc_refinement::CounterExample;
use merc_refinement::ExplorationStrategy;
use merc_refinement::RefinementType;
use merc_refinement::refines;
use merc_utilities::Timing;

#[test]
fn weak_trace_counterexample_reports_the_visible_trace_not_raw_impl_steps() {
    // spec: a single deadlocked state (no transitions at all): its only weak
    // trace is the empty trace.
    let spec = read_aut(b"des (0,0,1)" as &[u8]).unwrap();

    // impl: 0 -i-> 1 -a-> 2. Its only non-trivial weak trace is "a" (the
    // leading tau is unobservable). This must NOT weak-trace-refine spec,
    // since spec cannot do "a".
    let impl_lts = read_aut(
        br#"des (0,2,3)
        (0,"i",1)
        (1,"a",2)"# as &[u8],
    )
    .unwrap();

    let mut timing = Timing::new();
    let (result, ce) = refines(
        impl_lts,
        spec,
        RefinementType::Weaktrace,
        ExplorationStrategy::BFS,
        false,
        true,
        &mut timing,
    );

    assert!(
        !result,
        "impl performs a weak trace ('a') that spec cannot, so refinement must fail"
    );

    match ce.expect("a counter example must be produced when the check fails") {
        CounterExample::WeakTrace(trace) => {
            assert_eq!(
                trace,
                vec!["a".to_string()],
                "the weak-trace counter example must report only the visible trace \
                 (the leading tau is implicit, per CounterExample::WeakTrace's own doc \
                 comment: `<tau*.a0.tau*.a1...>true`), but got {trace:?}"
            );
        }
        _ => panic!("expected a WeakTrace counter example, got a different variant"),
    }
}
