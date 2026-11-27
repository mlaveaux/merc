//! The fixpoint equation system of a checked modal formula.

use std::ops::ControlFlow;

use merc_formula::FormulaType;
use merc_formula::ModalSpecification;
use merc_syntax::StateFrmKind;
use merc_syntax::Traverse;
use merc_syntax::UntypedStateFrmSpec;

/// Type checks `text` as a boolean modal formula.
fn check(text: &str) -> ModalSpecification {
    let spec = UntypedStateFrmSpec::parse(text).expect("the formula should parse");
    ModalSpecification::from_untyped(spec, FormulaType::Bool).expect("the formula should type check")
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_fixpoint_equation_system_construction() {
    let spec = check("mu X. [a]X && nu Y. <b>true");
    let system = spec.equation_system();

    assert_eq!(system.to_string(), "0: mu X = ([a]X && Y)\n1: nu Y = <b>true");
    assert_eq!(system.len(), 2);
    assert_eq!(system.alternation_depth(0), 1);
    assert_eq!(system.alternation_depth(1), 0);
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_fixpoint_equation_system_example() {
    let spec = check(include_str!("../../../../examples/vpg/running_example.mcf"));
    let system = spec.equation_system();

    assert_eq!(
        system.to_string(),
        "0: nu X = Y\n1: mu Y = ([ins]Y && ([xxl]Y && [std]X))"
    );
    assert_eq!(system.alternation_depth(0), 2);
    assert_eq!(system.alternation_depth(1), 1);
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_formula_without_fixpoint_gets_a_placeholder_equation() {
    let spec = check("[a]true");
    let system = spec.equation_system();

    assert_eq!(system.to_string(), "0: mu X0 = [a]true");
    assert_eq!(system.alternation_depth(0), 0);
}

/// `mu X. (nu X. X)` is legal: the inner `X` shadows the outer one, and both get an equation.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_fixpoint_equation_system_shadowed_names_do_not_collide() {
    let spec = check("mu X. [true](nu X. X)");
    let system = spec.equation_system();

    assert_eq!(system.to_string(), "0: mu X = [true]X\n1: nu X = X");
    let ids: Vec<_> = system.iter().map(|equation| equation.variable().id).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    // Both occurrences of `X` refer to the inner equation.
    for equation in system.iter() {
        let referenced = equation.body().visit(|formula| match &formula.node {
            StateFrmKind::Resolved(_, _, id) => ControlFlow::Break(*id),
            _ => ControlFlow::Continue(()),
        });
        assert_eq!(referenced, ids[1]);
    }
}
