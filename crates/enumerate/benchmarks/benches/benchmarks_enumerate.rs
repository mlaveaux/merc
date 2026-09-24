#![forbid(unsafe_code)]

use std::hint::black_box;
use std::ops::ControlFlow;
use std::rc::Rc;

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
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteSpecification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// A small recursive sort mirroring `Nat`'s shape (`dzero`/`dsucc`), avoiding
/// the real `Nat`/`Pos` prelude.
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
/// of bounds, with one rewriter and enumerator shared across all iterations as
/// a state space exploration would.
fn criterion_benchmark_bounded_enumeration(c: &mut Criterion) {
    let spec = lower(PRELUDE);
    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let mut enumerator = Enumerator::new(plans);

    let mut group = c.benchmark_group("bounded sum enumeration");
    for bound in [10, 50, 200] {
        let (vars, body) = lt_goal(bound);
        group.bench_function(format!("n < {bound}"), |bencher| {
            bencher.iter(|| {
                let mut count = 0usize;
                enumerator.enumerate(
                    &mut rewriter,
                    &mut generator_for(&vars),
                    &vars,
                    &body,
                    |_rewriter, _solution| -> ControlFlow<()> {
                        count += 1;
                        ControlFlow::Continue(())
                    },
                );
                black_box(count);
            });
        });
    }
    group.finish();
}

/// Cost of constructing the enumeration plans for a specification.
fn criterion_benchmark_enumeration_plans_build(c: &mut Criterion) {
    let spec = lower(PRELUDE);

    c.bench_function("EnumerationPlans::build (D + Bool prelude)", |bencher| {
        bencher.iter(|| {
            black_box(EnumerationPlans::build(&spec));
        });
    });
}

criterion_group!(
    benches,
    criterion_benchmark_bounded_enumeration,
    criterion_benchmark_enumeration_plans_build,
);
criterion_main!(benches);
