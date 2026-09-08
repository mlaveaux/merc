use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::AHashSet;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_enumerate::EnumerationLimits;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_enumerate::Outcome;
use merc_enumerate::QuantifierKind;
use merc_enumerate::WitnessOutcome;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteSpecification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// `D`'s shape mirrors `Nat`'s (`dzero`/`dsucc`, `lt`/`eq` defined by
/// recursion on both constructors), so it exercises the same "infinite
/// enumerable sort with a bounded guard" pattern the real `Nat` prelude would,
/// without pulling that prelude in.
const PRELUDE: &str = "
    sort D;
    cons dzero: D;
         dsucc: D -> D;
    map lt: D # D -> Bool;
        eq: D # D -> Bool;
    var x, y: D;
    eqn lt(x, dzero) = false;
        lt(dzero, dsucc(y)) = true;
        lt(dsucc(x), dsucc(y)) = lt(x, y);
        eq(dzero, dzero) = true;
        eq(dsucc(x), dzero) = false;
        eq(dzero, dsucc(y)) = false;
        eq(dsucc(x), dsucc(y)) = eq(x, y);
";

/// Builds the numeral `D` term for `value` (`dsucc(dsucc(...dzero...))`), for
/// asserting on solutions found by an [`Enumerator`] built over [`PRELUDE`].
fn numeral(value: usize) -> DataExpression {
    let dzero = merc_data::DataFunctionSymbol::with_sort("dzero", d_sort().copy());
    let succ_sort: merc_data::SortExpression = merc_data::SortArrow::new(&[d_sort()], d_sort()).into();
    let dsucc = merc_data::DataFunctionSymbol::with_sort("dsucc", succ_sort.copy());
    let mut term: DataExpression = dzero.into();
    for _ in 0..value {
        term = merc_data::DataApplication::with_args(&dsucc, &[term]).into();
    }
    term
}

fn d_sort() -> merc_data::SortExpression {
    merc_data::SortExpression::from(merc_data::BasicSort::new("D"))
}

/// Parses and type-checks `PRELUDE` followed by `source`.
fn lower(source: &str) -> Mcrl2DataSpecification {
    let untyped = UntypedDataSpecification::parse(&format!("{PRELUDE}\n{source}")).unwrap();
    let data_spec = DataSpecification::from_untyped(untyped).unwrap();
    data_spec.lower_data_specification()
}

/// Returns the `(variables, rhs)` of the single equation defining mapping
/// `name`, i.e. treats `map name: ... -> Bool; eqn name(vars...) = body;` as a
/// `(vars, body)` enumeration goal.
fn goal(spec: &Mcrl2DataSpecification, name: &str) -> (Vec<DataVariable>, DataExpression) {
    let equation = spec
        .equations()
        .iter()
        .find(|eq| eq.lhs().data_function_symbol().name().value() == name)
        .unwrap_or_else(|| panic!("no equation for {name}"));
    (equation.variables().to_vec(), equation.rhs().protect())
}

/// A generator seeded from exactly the names in scope for one goal — see
/// [`FreshVariableGenerator`]'s doc comment on why this is the caller's job.
fn generator_for(vars: &[DataVariable]) -> FreshVariableGenerator {
    FreshVariableGenerator::new(vars.iter().map(|v| v.name().to_string()))
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerate_finds_every_bounded_solution() {
    let spec = lower(
        "map goal: D -> Bool;
         var n: D;
         eqn goal(n) = lt(n, dsucc(dsucc(dsucc(dsucc(dsucc(dzero))))));", // n < 5
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    let mut results: Vec<DataExpression> = Vec::new();
    let outcome = enumerator.enumerate(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, solution| -> ControlFlow<()> {
            results.push(solution.values()[0].clone());
            ControlFlow::Continue(())
        },
    );

    assert!(matches!(outcome, Outcome::Exhausted), "{outcome:?}");

    let expected: AHashSet<DataExpression> = (0..5).map(numeral).collect();
    let actual: AHashSet<DataExpression> = results.iter().cloned().collect();
    assert_eq!(actual, expected, "{results:?}");
    assert_eq!(results.len(), 5, "solutions must be pairwise distinct: {results:?}");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerate_never_truncates_past_the_default_limits() {
    // `Enumerator::enumerate` (the `sum`-exploration path) must ignore
    // `EnumerationLimits` entirely: a truncated result is an unsound LTS.
    let target = 20;
    let tiny_limits = EnumerationLimits {
        max_items: 3,
        max_depth: 2,
    };

    let bound: String = numeral_source(target);
    let spec = lower(&format!(
        "map goal: D -> Bool;
         var n: D;
         eqn goal(n) = lt(n, {bound});"
    ));
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans).with_limits(tiny_limits);
    let mut count = 0usize;
    let outcome = enumerator.enumerate(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, _solution| -> ControlFlow<()> {
            count += 1;
            ControlFlow::Continue(())
        },
    );

    assert!(matches!(outcome, Outcome::Exhausted), "{outcome:?}");
    assert_eq!(
        count, target,
        "must find every solution despite max_items={}",
        tiny_limits.max_items
    );
}

/// Renders `dsucc(dsucc(...dzero...))` as mCRL2 source text, for embedding a
/// numeral literal directly in a parsed spec.
fn numeral_source(value: usize) -> String {
    let mut text = "dzero".to_string();
    for _ in 0..value {
        text = format!("dsucc({text})");
    }
    text
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerate_is_fair_across_two_infinite_variables() {
    // A depth-first "finish `n` before touching `m`" strategy would never
    // reach any solution with `m > 0`, since `n` alone has infinitely many
    // values. The 3x3 = 9 solutions here can only all be found if both
    // dimensions are explored — see `docs/enumeration-crate-plan.md` §4.5.
    let spec = lower(
        "map goal: D # D -> Bool;
         var n: D;
             m: D;
         eqn goal(n, m) = lt(n, dsucc(dsucc(dsucc(dzero)))) && lt(m, dsucc(dsucc(dsucc(dzero))));", // n<3 && m<3
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    let mut results: AHashSet<(DataExpression, DataExpression)> = AHashSet::new();
    let outcome = enumerator.enumerate(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, solution| -> ControlFlow<()> {
            let values = solution.values();
            results.insert((values[0].clone(), values[1].clone()));
            ControlFlow::Continue(())
        },
    );

    assert!(matches!(outcome, Outcome::Exhausted), "{outcome:?}");
    assert_eq!(results.len(), 9, "{results:?}");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerate_applies_the_one_point_rule() {
    // `b == true` narrows `b` to a single substitution instead of a search
    // over `Bool`, leaving only the (still genuinely searched) `n < 3` over
    // `D`.
    let spec = lower(
        "map goal: Bool # D -> Bool;
         var b: Bool;
             n: D;
         eqn goal(b, n) = (b == true) && lt(n, dsucc(dsucc(dsucc(dzero))));", // b == true && n < 3
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    let mut results: Vec<(DataExpression, DataExpression)> = Vec::new();
    let outcome = enumerator.enumerate(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, solution| -> ControlFlow<()> {
            let values = solution.values();
            results.push((values[0].clone(), values[1].clone()));
            ControlFlow::Continue(())
        },
    );

    assert!(matches!(outcome, Outcome::Exhausted), "{outcome:?}");

    let true_literal: DataExpression = merc_data::DataFunctionSymbol::with_sort(
        "true",
        merc_data::SortExpression::from(merc_data::BasicSort::new("Bool")).copy(),
    )
    .into();
    let expected: AHashSet<(DataExpression, DataExpression)> =
        (0..3).map(|n| (true_literal.clone(), numeral(n))).collect();
    let actual: AHashSet<(DataExpression, DataExpression)> = results.iter().cloned().collect();
    assert_eq!(actual, expected, "{results:?}");
    assert_eq!(
        results.len(),
        3,
        "one-point elimination of `b` must not multiply out solutions: {results:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_constraint_directed_ordering_promotes_the_constrained_variable() {
    // `m` never occurs in `goal`'s body at all — a vacuous bound variable.
    let tiny_limits = EnumerationLimits {
        max_items: 20,
        max_depth: 64,
    };

    let spec = lower(&format!(
        "map goal: D # D -> Bool;
         var m, n: D;
         eqn goal(m, n) = eq(n, {});", // `eq`, not `==`: keep the one-point rule out of this.
        numeral_source(3)
    ));
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");
    assert_eq!(
        vars[0].name().to_string(),
        "m",
        "test assumes `m` is listed before `n` in `vars`"
    );

    let mut enumerator = Enumerator::new(plans).with_limits(tiny_limits);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Exists,
    ) {
        WitnessOutcome::Found(values) => assert_eq!(values[1], numeral(3)),
        other => panic!(
            "expected a witness (ordering should promote the constrained `n` ahead of vacuous `m`), got {other:?}"
        ),
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_find_witness_exists_finds_a_small_witness() {
    let spec = lower(&format!(
        "map goal: D -> Bool;
         var n: D;
         eqn goal(n) = eq(n, {});",
        numeral_source(7)
    ));
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Exists,
    ) {
        WitnessOutcome::Found(values) => assert_eq!(values, vec![numeral(7)]),
        other => panic!("expected a witness, got {other:?}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_find_witness_gives_up_past_the_bound() {
    // The witness exists (`eq(n, target)`) but lies beyond the configured
    // `max_items`, so the bounded search must give up rather than claim
    // `NoneExists` — for `∃` that distinction is `false` versus "unknown".
    let target = 20;
    let tiny_limits = EnumerationLimits {
        max_items: 3,
        max_depth: 64,
    };

    let spec = lower(&format!(
        "map goal: D -> Bool;
         var n: D;
         eqn goal(n) = eq(n, {});",
        numeral_source(target)
    ));
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans).with_limits(tiny_limits);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Exists,
    ) {
        WitnessOutcome::GaveUp => {}
        other => panic!("expected GaveUp, got {other:?}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_find_witness_forall_holds_over_a_finite_sort() {
    // `b || !b` is a Bool tautology: searching for a *counterexample* over the
    // finite `Bool` sort must exhaust without finding one, which is what
    // proves the quantifier holds.
    let spec = lower(
        "map goal: Bool -> Bool;
         var b: Bool;
         eqn goal(b) = b || !b;",
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Forall,
    ) {
        WitnessOutcome::NoneExists => {}
        other => panic!("expected NoneExists (the quantifier holds), got {other:?}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_find_witness_forall_is_not_broken_by_the_one_point_rule() {
    // `∀ n:D . n == dzero` is false — `dsucc(dzero)` is a counterexample.
    let spec = lower(
        "map goal: D -> Bool;
         var n: D;
         eqn goal(n) = n == dzero;",
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Forall,
    ) {
        WitnessOutcome::Found(values) => {
            assert_ne!(values[0], numeral(0), "dzero is not a counterexample to `n == dzero`");
        }
        other => panic!("expected a counterexample to `forall n. n == dzero`, got {other:?}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_find_witness_forall_does_not_claim_an_undecided_body_holds() {
    // `p` has no equations, so `p(true)`/`p(false)` are stuck: neither is the
    // literal `false` (no counterexample found) nor the literal `true` (no
    // proof either). `NoneExists` means "the quantifier holds", which is not
    // something this search established — `GaveUp` is the honest answer, per
    // `WitnessOutcome::GaveUp`'s own doc comment.
    let spec = lower(
        "map goal: Bool -> Bool;
             p: Bool -> Bool;
         var b: Bool;
         eqn goal(b) = p(b);",
    );
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let (vars, body) = goal(&spec, "goal");

    let mut enumerator = Enumerator::new(plans);
    match enumerator.find_witness(
        &mut rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        QuantifierKind::Forall,
    ) {
        WitnessOutcome::GaveUp => {}
        other => panic!("expected GaveUp for an undecidable body, got {other:?}"),
    }
}
