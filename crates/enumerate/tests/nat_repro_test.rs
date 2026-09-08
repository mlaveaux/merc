use std::ops::ControlFlow;
use std::rc::Rc;

use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::Enumerator;
use merc_enumerate::FreshVariableGenerator;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::NumberEncoding;

/// Regression test for a bug where `Enumerator::enumerate` found zero
/// solutions for a real, machine-word-encoded `Nat` sort.
#[test]
fn enumerate_finds_every_solution_for_real_machine_word_nat() {
    let untyped = UntypedDataSpecification::parse(
        "map goal: Nat -> Bool;
         var m: Nat;
         eqn goal(m) = m < 3;",
    )
    .unwrap();
    let typed = DataSpecification::from_untyped_with(untyped, NumberEncoding::MachineWord).unwrap();
    let spec = typed.lower_data_specification();

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

    let mut generator = FreshVariableGenerator::new(std::iter::once(m_var.name().to_string()));
    let mut enumerator = Enumerator::new(plans);
    let mut solutions = Vec::new();
    let outcome = enumerator.enumerate(
        &mut rewriter,
        &mut generator,
        &[m_var],
        &body,
        |_rewriter, sol| -> ControlFlow<()> {
            solutions.push(sol.values().to_vec());
            ControlFlow::Continue(())
        },
    );
    assert!(matches!(outcome, merc_enumerate::Outcome::Exhausted), "{outcome:?}");
    assert_eq!(solutions.len(), 3, "expected m = 0, 1, 2, got {solutions:?}");

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
