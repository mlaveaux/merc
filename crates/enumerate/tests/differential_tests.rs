//! Differential/property-based tests: [`Enumerator`] and [`NaiveEnumerator`]
//! must agree on the same result set for the same goal, even though they use
//! completely different algorithms (incremental breadth-first work queue vs.
//! brute-force full instantiation — see [`NaiveEnumerator`]'s doc comment).
//!
//! Random goals are built through the `merc_data`/`merc_sabre` API rather than
//! parsed as source text, so that `merc_syntax`/`merc_typecheck` do not have
//! to run once per goal; see `enumerator_tests.rs` for why the sort is a small
//! hand-rolled `D` rather than the real `Nat`.

use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::AHashSet;
use merc_data::BasicSort;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortArrow;
use merc_data::SortExpression;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_enumerate::NaiveEnumerator;
use merc_enumerate::Outcome;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_utilities::random_test;
use rand::RngExt;
use rand::rngs::StdRng;

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

fn lower(source: &str) -> Mcrl2DataSpecification {
    let untyped = UntypedDataSpecification::parse(source).unwrap();
    let data_spec = DataSpecification::from_untyped(untyped).unwrap();
    data_spec.lower_data_specification()
}

fn d_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("D"))
}

fn bool_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("Bool"))
}

/// Builds the numeral `D` term for `value` (`dsucc(dsucc(...dzero...))`),
/// via the API rather than parsing — see the module doc comment.
fn numeral(value: u32) -> DataExpression {
    let dzero = DataFunctionSymbol::with_sort("dzero", d_sort().copy());
    let succ_sort: SortExpression = SortArrow::new(&[d_sort()], d_sort()).into();
    let dsucc = DataFunctionSymbol::with_sort("dsucc", succ_sort.copy());
    let mut term: DataExpression = dzero.into();
    for _ in 0..value {
        term = DataApplication::with_args(&dsucc, &[term]).into();
    }
    term
}

fn lt_symbol() -> DataFunctionSymbol {
    let sort: SortExpression = SortArrow::new(&[d_sort(), d_sort()], bool_sort()).into();
    DataFunctionSymbol::with_sort("lt", sort.copy())
}

fn eq_symbol() -> DataFunctionSymbol {
    let sort: SortExpression = SortArrow::new(&[d_sort(), d_sort()], bool_sort()).into();
    DataFunctionSymbol::with_sort("eq", sort.copy())
}

fn and_symbol() -> DataFunctionSymbol {
    let sort: SortExpression = SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()).into();
    DataFunctionSymbol::with_sort("&&", sort.copy())
}

fn true_literal() -> DataExpression {
    DataFunctionSymbol::with_sort("true", bool_sort().copy()).into()
}

fn generator_for(vars: &[DataVariable]) -> FreshVariableGenerator {
    FreshVariableGenerator::new(vars.iter().map(|v| v.name().to_string()))
}

/// One randomly generated `op(var, numeral(bound))` conjunct (`op` is `lt` or
/// `eq`). Restricted to this shape so every generated goal is provably
/// decidable within a known bound: `lt(x, dzero) = false` holds regardless of
/// how `x` is later instantiated, so a clause with bound `k` can never
/// contribute a solution beyond `k`.
struct Clause {
    variable: DataVariable,
    is_lt: bool,
    bound: u32,
}

impl Clause {
    fn to_term(&self) -> DataExpression {
        let symbol = if self.is_lt { lt_symbol() } else { eq_symbol() };
        DataApplication::with_args(
            &symbol,
            &[DataExpression::from(self.variable.clone()), numeral(self.bound)],
        )
        .into()
    }
}

fn random_clause(rng: &mut StdRng, variable: DataVariable, max_bound: u32) -> Clause {
    Clause {
        variable,
        is_lt: rng.random_bool(0.5),
        bound: rng.random_range(0..=max_bound),
    }
}

/// Conjoins `terms` with `&&`, left to right. Panics on an empty slice (every
/// goal here has at least one variable).
fn conjunction(terms: &[DataExpression]) -> DataExpression {
    let and = and_symbol();
    terms
        .iter()
        .cloned()
        .reduce(|acc, term| DataApplication::with_args(&and, &[acc, term]).into())
        .expect("at least one clause")
}

/// Generates a random 1- or 2-variable goal, runs both an [`Enumerator`] and
/// a [`NaiveEnumerator`] over it (against fixed, shared rewriters/`plans`),
/// and asserts they find the same solution set.
///
/// The three [`InnermostRewriter`]s are built once, outside `random_test`'s
/// loop: `InnermostRewriter::new` compiles a `SetAutomaton` over every
/// equation `spec` carries, including the whole lowered built-in library, so
/// rebuilding one per goal dominates the test's runtime.
#[allow(clippy::too_many_arguments)]
fn check_one_random_goal(
    rng: &mut StdRng,
    plans: &Rc<EnumerationPlans>,
    enumerator_rewriter: &mut InnermostRewriter,
    naive_rewriter: &mut InnermostRewriter,
    checker: &mut InnermostRewriter,
) {
    const MAX_BOUND: u32 = 8;

    let num_vars = rng.random_range(1..=2);
    let vars: Vec<DataVariable> = (0..num_vars)
        .map(|i| DataVariable::with_sort(format!("var{i}").as_str(), d_sort().copy()))
        .collect();
    let clauses: Vec<Clause> = vars.iter().map(|v| random_clause(rng, v.clone(), MAX_BOUND)).collect();
    let body = conjunction(&clauses.iter().map(Clause::to_term).collect::<Vec<_>>());

    let mut enumerator = Enumerator::new(plans.clone());
    let mut enumerator_results: AHashSet<Vec<DataExpression>> = AHashSet::new();
    let outcome = enumerator.enumerate(
        enumerator_rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, solution| -> ControlFlow<()> {
            enumerator_results.insert(solution.values().to_vec());
            ControlFlow::Continue(())
        },
    );
    assert!(
        matches!(outcome, Outcome::Exhausted),
        "Enumerator: {outcome:?}, goal: {body}"
    );

    let max_bound = clauses.iter().map(|c| c.bound).max().unwrap_or(0);
    let mut naive = NaiveEnumerator::new(naive_rewriter, plans, max_bound + 1);
    let naive_results = naive.enumerate_all(&vars, &body);

    assert_eq!(
        enumerator_results, naive_results,
        "Enumerator and NaiveEnumerator disagree on goal `{body}`"
    );

    // Sanity-check the rewriter itself: every reported Enumerator solution
    // really does satisfy the goal when substituted back in, independent of
    // both enumerators' own bookkeeping.
    for solution in &enumerator_results {
        let mut sigma = ahash::HashMap::default();
        for (variable, value) in vars.iter().zip(solution) {
            sigma.insert(variable.clone(), value.clone());
        }
        assert_eq!(
            checker.rewrite_with(&body, &sigma),
            true_literal(),
            "reported solution {solution:?} does not satisfy `{body}`"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerator_agrees_with_naive_enumerator() {
    let spec = lower(PRELUDE);
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));
    let mut enumerator_rewriter = InnermostRewriter::new(&rewrite_spec);
    let mut naive_rewriter = InnermostRewriter::new(&rewrite_spec);
    let mut checker = InnermostRewriter::new(&rewrite_spec);

    random_test(200, |rng| {
        check_one_random_goal(rng, &plans, &mut enumerator_rewriter, &mut naive_rewriter, &mut checker)
    });
}

/// A specific goal worth pinning down explicitly, on top of the random
/// coverage above.
#[test]
#[cfg_attr(miri, ignore)]
fn test_enumerator_agrees_with_naive_enumerator_on_an_unsatisfiable_goal() {
    let spec = lower(PRELUDE);
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let n = DataVariable::with_sort("n", d_sort().copy());
    let vars = vec![n.clone()];
    // n == 1 && n < 0: unsatisfiable.
    let body = conjunction(&[
        Clause {
            variable: n.clone(),
            is_lt: false,
            bound: 1,
        }
        .to_term(),
        Clause {
            variable: n,
            is_lt: true,
            bound: 0,
        }
        .to_term(),
    ]);

    let mut enumerator_rewriter = InnermostRewriter::new(&rewrite_spec);
    let mut enumerator = Enumerator::new(plans.clone());
    let mut enumerator_results: AHashSet<Vec<DataExpression>> = AHashSet::new();
    let outcome = enumerator.enumerate(
        &mut enumerator_rewriter,
        &mut generator_for(&vars),
        &vars,
        &body,
        |_rewriter, solution| -> ControlFlow<()> {
            enumerator_results.insert(solution.values().to_vec());
            ControlFlow::Continue(())
        },
    );
    assert!(matches!(outcome, Outcome::Exhausted), "{outcome:?}");
    assert!(enumerator_results.is_empty(), "{enumerator_results:?}");

    let mut naive_rewriter = InnermostRewriter::new(&rewrite_spec);
    let mut naive = NaiveEnumerator::new(&mut naive_rewriter, &plans, 3);
    assert_eq!(naive.enumerate_all(&vars, &body), enumerator_results);
}
