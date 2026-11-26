use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::AHashSet;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::EnumeratorFrame;
use merc_enumerate::FreshVariableGenerator;
use merc_enumerate::NaiveEnumerator;
use merc_lowering::lower_data_specification;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::NumberEncoding;

/// Regression test for a bug where `Enumerator::enumerate` found zero
/// solutions for a real, machine-word-encoded `Nat` sort.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn enumerate_finds_every_solution_for_real_machine_word_nat() {
    let untyped = UntypedDataSpecification::parse(
        "map goal: Nat -> Bool;
         var m: Nat;
         eqn goal(m) = m < 3;",
    )
    .unwrap();
    let typed =
        DataSpecification::from_untyped_with(untyped, NumberEncoding::MachineWord, &mut SourceMap::new()).unwrap();
    let spec = lower_data_specification(&typed);

    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = Rc::new(EnumerationPlans::build(&spec));

    let goal_symbol = spec
        .mappings()
        .iter()
        .find(|f| f.name().value() == "goal")
        .unwrap()
        .clone();
    let m_var = spec
        .equations()
        .iter()
        .find(|e| e.lhs().to_string().starts_with("goal"))
        .unwrap()
        .variables()
        .to_vec()[0]
        .clone();
    let body = DataExpression::from(DataApplication::with_args(
        &goal_symbol,
        &[DataExpression::from(m_var.clone())],
    ));

    let generator = FreshVariableGenerator::new("v", std::iter::once(m_var.name().to_string()));
    let enumerator = Enumerator::new(plans, generator);
    let mut solutions = Vec::new();
    let outcome = enumerator.enumerate(
        &mut EnumeratorFrame::default(),
        &mut rewriter,
        &[m_var],
        &body,
        |_rewriter, sol| -> ControlFlow<()> {
            solutions.push(sol.values().to_vec());
            ControlFlow::Continue(())
        },
    );
    assert!(matches!(outcome, merc_enumerate::Outcome::Exhausted), "{outcome:?}");
    // Three distinct values that satisfy `m < 3` are exactly 0, 1 and 2.
    assert_eq!(solutions.len(), 3, "expected m = 0, 1, 2, got {solutions:?}");
    let distinct: AHashSet<&Vec<DataExpression>> = solutions.iter().collect();
    assert_eq!(distinct.len(), 3, "solutions must be distinct: {solutions:?}");
    for solution in &solutions {
        let goal = DataExpression::from(DataApplication::with_args(&goal_symbol, solution));
        assert_eq!(
            rewriter.rewrite(&goal),
            DataExpression::bool_literal(true),
            "{solution:?}"
        );
    }

    // Every reported value must itself be a normal form: callers (e.g. the
    // LPS explorer) splice these straight into further `rewrite_with` calls
    // as substitution images, which requires it.
    for solution in &solutions {
        for value in solution {
            assert_eq!(
                *value,
                rewriter.rewrite(value),
                "solution value {value} is not a normal form"
            );
        }
    }
}

/// The reference enumerator must also normalise `@c0` before substituting it,
/// or `m = 0` is missed under the machine-word `Nat` encoding.
#[test]
#[cfg_attr(miri, ignore)] // Lowering the machine-word specification is too slow under miri.
fn naive_enumerator_finds_every_solution_for_real_machine_word_nat() {
    let untyped = UntypedDataSpecification::parse(
        "map goal: Nat -> Bool;
         var m: Nat;
         eqn goal(m) = m < 3;",
    )
    .unwrap();
    let typed =
        DataSpecification::from_untyped_with(untyped, NumberEncoding::MachineWord, &mut SourceMap::new()).unwrap();
    let spec = lower_data_specification(&typed);

    let rewrite_spec = RewriteSpecification::from_data_specification(&spec);
    let mut rewriter = InnermostRewriter::new(&rewrite_spec);
    let plans = EnumerationPlans::build(&spec);

    let equation = spec
        .equations()
        .iter()
        .find(|e| e.lhs().to_string().starts_with("goal"))
        .unwrap();
    let vars = equation.variables().to_vec();
    let body = equation.rhs().protect();

    let solutions = NaiveEnumerator::new(&mut rewriter, &plans, 4).enumerate_all(&vars, &body);
    // Three distinct values that satisfy `m < 3` are exactly 0, 1 and 2.
    assert_eq!(solutions.len(), 3, "expected m = 0, 1, 2, got {solutions:?}");
    let goal_symbol = equation.lhs().data_function_symbol().protect();
    for solution in &solutions {
        let goal = DataExpression::from(DataApplication::with_args(&goal_symbol, solution));
        assert_eq!(
            rewriter.rewrite(&goal),
            DataExpression::bool_literal(true),
            "{solution:?}"
        );
    }
}
