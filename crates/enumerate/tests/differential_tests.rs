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
use merc_data::bool_literal;
use merc_data::make_and;
use merc_data::make_equal;
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
        dzero == dzero = true;
        dsucc(x) == dzero = false;
        dzero == dsucc(y) = false;
        dsucc(x) == dsucc(y) = x == y;
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

fn generator_for(vars: &[DataVariable]) -> FreshVariableGenerator {
    FreshVariableGenerator::new(vars.iter().map(|v| v.name().to_string()))
}

/// Which binary predicate a generated clause uses.
///
/// `==` is included because it is the shape the one-point rule keys on: it
/// survives normalisation while its argument is still free (`D` has no
/// built-in equality equations, so [`PRELUDE`] supplies them), yet decides on
/// ground terms.
#[derive(Clone, Copy)]
enum Predicate {
    Lt,
    Eq,
    Equality,
}

impl Predicate {
    fn symbol(self) -> Option<DataFunctionSymbol> {
        match self {
            Predicate::Lt => Some(lt_symbol()),
            Predicate::Eq => Some(eq_symbol()),
            Predicate::Equality => None,
        }
    }

    fn apply(self, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        match self.symbol() {
            Some(symbol) => DataApplication::with_args(&symbol, &[lhs, rhs]).into(),
            None => make_equal(d_sort(), lhs, rhs),
        }
    }
}

/// One randomly generated `op(var, numeral(bound))` conjunct. Restricted to
/// this shape so every generated goal is provably decidable within a known
/// bound: `lt(x, dzero) = false` holds regardless of how `x` is later
/// instantiated, so a clause with bound `k` can never contribute a solution
/// beyond `k`, and `eq`/`==` pin their variable outright.
struct Clause {
    variable: DataVariable,
    predicate: Predicate,
    bound: u32,
}

impl Clause {
    fn to_term(&self) -> DataExpression {
        self.predicate
            .apply(DataExpression::from(self.variable.clone()), numeral(self.bound))
    }
}

fn random_clause(rng: &mut StdRng, variable: DataVariable, max_bound: u32) -> Clause {
    Clause {
        variable,
        predicate: match rng.random_range(0..3) {
            0 => Predicate::Lt,
            1 => Predicate::Eq,
            _ => Predicate::Equality,
        },
        bound: rng.random_range(0..=max_bound),
    }
}

/// A randomly generated clause relating two *different* variables, e.g.
/// `lt(var0, var1)`.
///
/// Every variable already carries its own bounding clause, so conjoining one
/// of these only ever removes solutions and cannot make the goal unbounded.
fn random_cross_clause(rng: &mut StdRng, left: &DataVariable, right: &DataVariable) -> DataExpression {
    let predicate = match rng.random_range(0..3) {
        0 => Predicate::Lt,
        1 => Predicate::Eq,
        _ => Predicate::Equality,
    };
    predicate.apply(DataExpression::from(left.clone()), DataExpression::from(right.clone()))
}

/// Conjoins `terms` with `&&`, left to right. Panics on an empty slice (every
/// goal here has at least one variable).
fn conjunction(terms: &[DataExpression]) -> DataExpression {
    terms.iter().cloned().reduce(make_and).expect("at least one clause")
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
    let mut terms: Vec<DataExpression> = clauses.iter().map(Clause::to_term).collect();
    if let [left, right] = vars.as_slice()
        && rng.random_bool(0.5)
    {
        terms.push(random_cross_clause(rng, left, right));
    }
    let body = conjunction(&terms);

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
            bool_literal(true),
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
            predicate: Predicate::Equality,
            bound: 1,
        }
        .to_term(),
        Clause {
            variable: n,
            predicate: Predicate::Lt,
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
