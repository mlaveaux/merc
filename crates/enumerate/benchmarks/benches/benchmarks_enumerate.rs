#![forbid(unsafe_code)]

use std::hint::black_box;
use std::ops::ControlFlow;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use merc_data::BasicSort;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortArrow;
use merc_data::SortExpression;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_enumerate::SortPlans;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteSpecification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// A small recursive sort mirroring `Nat`'s shape (`dzero`/`dsucc`), avoiding
/// the real `Nat`/`Pos` prelude for the reason in the module doc comment.
const PRELUDE: &str = "
    sort D;
    cons dzero: D;
         dsucc: D -> D;
    map lt: D # D -> Bool;
    var x, y: D;
    eqn lt(x, dzero) = false;
        lt(dzero, dsucc(y)) = true;
        lt(dsucc(x), dsucc(y)) = lt(x, y);
";

fn lower(source: &str) -> Mcrl2DataSpecification {
    let untyped = UntypedDataSpecification::parse(source).unwrap();
    let data_spec = DataSpecification::from_untyped(untyped).unwrap();
    data_spec.lower_data_specification()
}

fn d_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("D"))
}

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

fn lt_goal(bound: u32) -> (Vec<DataVariable>, DataExpression) {
    let n = DataVariable::with_sort("n", d_sort().copy());
    let lt_sort: SortExpression =
        SortArrow::new(&[d_sort(), d_sort()], SortExpression::from(BasicSort::new("Bool"))).into();
    let lt = DataFunctionSymbol::with_sort("lt", lt_sort.copy());
    let body: DataExpression =
        DataApplication::with_args(&lt, &[DataExpression::from(n.clone()), numeral(bound)]).into();
    (vec![n], body)
}

fn generator_for(vars: &[DataVariable]) -> FreshVariableGenerator {
    FreshVariableGenerator::new(vars.iter().map(|v| v.name().to_string()))
}

/// Constructor-expansion throughput for `sum n:D . n < bound`, across a range
/// of bounds — isolates the cost of `SortPlans`-indexed constructor lookup
/// and per-step normalisation (§6.1), since the enumerator processes roughly
/// `2 * bound` work items regardless of `bound`'s size.
fn criterion_benchmark_bounded_enumeration(c: &mut Criterion) {
    let spec = lower(PRELUDE);
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let plans = SortPlans::build(&spec);

    let mut group = c.benchmark_group("bounded sum enumeration");
    for bound in [10, 50, 200] {
        let (vars, body) = lt_goal(bound);
        group.bench_function(format!("n < {bound}"), |bencher| {
            bencher.iter(|| {
                let mut rewriter = InnermostRewriter::new(&rewrite_spec);
                let mut enumerator = Enumerator::new(&mut rewriter, &plans, generator_for(&vars));
                let mut count = 0usize;
                enumerator.enumerate(&vars, &body, |_solution| {
                    count += 1;
                    ControlFlow::Continue(())
                });
                black_box(count);
            });
        });
    }
    group.finish();
}

/// The `SortPlans::build` win the constructor-index buys, isolated:
/// re-running it once per call is what the enumerator would pay without it
/// (§6.1's "no constructor lookup" claim).
fn criterion_benchmark_sort_plans_build(c: &mut Criterion) {
    let spec = lower(PRELUDE);

    c.bench_function("SortPlans::build (D + Bool prelude)", |bencher| {
        bencher.iter(|| {
            black_box(SortPlans::build(&spec));
        });
    });
}

/// Finite-sort enumeration throughput, materialised once and served from the
/// per-`Enumerator` cache (§6.2) on every subsequent call within the same
/// `bencher.iter()` batch — a single `Enumerator` is reused across
/// iterations here specifically to measure the cache-hit path, unlike the
/// other benchmarks which rebuild one per iteration to isolate a single
/// `enumerate` call's cost.
fn criterion_benchmark_finite_sort_cache_hit(c: &mut Criterion) {
    let spec = lower(PRELUDE);
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let plans = SortPlans::build(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);

    let b = DataVariable::with_sort("b", SortExpression::from(BasicSort::new("Bool")).copy());
    let vars = vec![b];
    // `goal(b) = true`: every Bool value is a solution, so this repeatedly
    // drains the cached 2-element `Bool` list.
    let body: DataExpression =
        DataFunctionSymbol::with_sort("true", SortExpression::from(BasicSort::new("Bool")).copy()).into();

    let mut enumerator = Enumerator::new(&mut rewriter, &plans, generator_for(&vars));
    c.bench_function("Bool sum enumeration (cached after first call)", |bencher| {
        bencher.iter(|| {
            let mut count = 0usize;
            enumerator.enumerate(&vars, &body, |_solution| {
                count += 1;
                ControlFlow::Continue(())
            });
            black_box(count);
        });
    });
}

criterion_group!(
    benches,
    criterion_benchmark_bounded_enumeration,
    criterion_benchmark_sort_plans_build,
    criterion_benchmark_finite_sort_cache_hit,
);
criterion_main!(benches);
