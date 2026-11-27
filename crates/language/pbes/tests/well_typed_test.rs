use merc_data::BinderType;
use merc_data::DataAbstraction;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::DataWhereClause;
use merc_data::DataWhrDecl;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortExpression;

use merc_pbes::FixpointSymbol;
use merc_pbes::Pbes;
use merc_pbes::PbesEquation;
use merc_pbes::PbesExpression;
use merc_pbes::PbesLocation;
use merc_pbes::PbesWellTypedError;
use merc_pbes::PropositionalVariable;
use merc_pbes::PropositionalVariableInstantiation;

fn check(globals: &[DataVariable], equations: Vec<PbesEquation>) -> Result<(), PbesWellTypedError> {
    Pbes::new(
        Mcrl2DataSpecification::default(),
        globals.to_vec(),
        equations,
        PropositionalVariableInstantiation::new("X", &[]),
    )
    .check_well_typed()
}

fn at_x() -> PbesLocation {
    PbesLocation::Equation("X".to_string())
}

#[test]
fn test_well_typed_pbes_is_accepted() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let c = DataVariable::with_sort("c", SortExpression::bool().copy());
    let g = DataVariable::with_sort("g", SortExpression::bool().copy());
    // mu X = forall b: Bool . Y(b) && exists c: Bool . val(c) || val(g); sibling quantifiers
    // may reuse a name.
    let formula = PbesExpression::and(
        &PbesExpression::forall(
            std::slice::from_ref(&b),
            &PbesExpression::from(PropositionalVariableInstantiation::new("Y", &[b.clone().into()])),
        ),
        &PbesExpression::or(
            &PbesExpression::exists(
                std::slice::from_ref(&c),
                &PbesExpression::from(DataExpression::from(c.clone())),
            ),
            &PbesExpression::exists(
                std::slice::from_ref(&c),
                &PbesExpression::from(DataExpression::from(g.clone())),
            ),
        ),
    );
    let y = PbesEquation::new(
        FixpointSymbol::Mu,
        PropositionalVariable::new("Y", std::slice::from_ref(&b)),
        PbesExpression::from(DataExpression::from(b.clone())),
    );
    assert_eq!(
        check(
            &[g],
            vec![
                PbesEquation::new(FixpointSymbol::Mu, PropositionalVariable::new("X", &[]), formula),
                y
            ]
        ),
        Ok(())
    );
}

#[test]
fn test_data_binders_and_where_clauses_bind_their_variables() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let c = DataVariable::with_sort("c", SortExpression::bool().copy());
    // val(exists b: Bool . b) && val(c whr c = true end): neither b nor c is free.
    let binder = DataAbstraction::new(BinderType::Exists, std::slice::from_ref(&b), b.clone().into());
    let true_ = DataExpression::bool_literal(true);
    let clause = DataWhereClause::new(c.clone().into(), &[DataWhrDecl::new(c.clone(), true_)]);
    let formula = PbesExpression::and(
        &DataExpression::from(binder).into(),
        &DataExpression::from(clause).into(),
    );
    assert_eq!(
        check(
            &[],
            vec![PbesEquation::new(
                FixpointSymbol::Mu,
                PropositionalVariable::new("X", &[]),
                formula
            )]
        ),
        Ok(())
    );
}

#[test]
fn test_duplicate_equation_is_rejected() {
    let error = check(
        &[],
        vec![
            PbesEquation::new(
                FixpointSymbol::Mu,
                PropositionalVariable::new("X", &[]),
                PbesExpression::true_constant()
            );
            2
        ],
    );
    assert_eq!(error, Err(PbesWellTypedError::DuplicateEquation { name: "X".into() }));
}

#[test]
fn test_duplicate_parameter_is_rejected() {
    let parameters = [
        DataVariable::with_sort("b", SortExpression::bool().copy()),
        DataVariable::with_sort("b", SortExpression::nat().copy()),
    ];
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &parameters),
            PbesExpression::true_constant(),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::DuplicateParameter {
            location: at_x(),
            parameter: "b".into()
        })
    );
}

#[test]
fn test_duplicate_global_variable_is_rejected() {
    let globals = [
        DataVariable::with_sort("g", SortExpression::bool().copy()),
        DataVariable::with_sort("g", SortExpression::nat().copy()),
    ];
    let error = check(
        &globals,
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[]),
            PbesExpression::true_constant(),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::DuplicateGlobalVariable { name: "g".into() })
    );
}

#[test]
fn test_quantifier_shadowing_parameter_is_rejected() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let formula = PbesExpression::exists(
        &[DataVariable::with_sort("b", SortExpression::nat().copy())],
        &PbesExpression::true_constant(),
    );
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[b]),
            formula,
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::QuantifierShadowsParameter {
            location: at_x(),
            variable: "b".into()
        })
    );
}

#[test]
fn test_quantifier_shadowing_quantifier_is_rejected() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let inner = PbesExpression::forall(
        std::slice::from_ref(&b),
        &PbesExpression::from(DataExpression::from(b.clone())),
    );
    let formula = PbesExpression::exists(
        std::slice::from_ref(&b),
        &PbesExpression::and(&PbesExpression::from(DataExpression::from(b.clone())), &inner),
    );
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[]),
            formula,
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::QuantifierShadowsQuantifier {
            location: at_x(),
            variable: "b".into()
        })
    );

    // The same name twice in one quantifier.
    let formula = PbesExpression::exists(
        &[b.clone(), DataVariable::with_sort("b", SortExpression::nat().copy())],
        &PbesExpression::from(DataExpression::from(b.clone())),
    );
    assert_eq!(
        check(
            &[],
            vec![PbesEquation::new(
                FixpointSymbol::Mu,
                PropositionalVariable::new("X", &[]),
                formula
            )]
        ),
        Err(PbesWellTypedError::QuantifierShadowsQuantifier {
            location: at_x(),
            variable: "b".into()
        })
    );
}

#[test]
fn test_quantifier_shadowing_global_is_rejected() {
    let g = DataVariable::with_sort("g", SortExpression::bool().copy());
    let formula = PbesExpression::exists(
        std::slice::from_ref(&g),
        &PbesExpression::from(DataExpression::from(g.clone())),
    );
    let error = check(
        std::slice::from_ref(&g),
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[]),
            formula,
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::QuantifierShadowsGlobal {
            location: at_x(),
            variable: "g".into()
        })
    );
}

#[test]
fn test_undeclared_free_variable_is_rejected() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[]),
            PbesExpression::from(DataExpression::from(b.clone())),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::UndeclaredFreeVariable {
            location: at_x(),
            variable: "b".into()
        })
    );

    // A variable is bound by its name and its sort.
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[DataVariable::with_sort("b", SortExpression::nat().copy())]),
            PbesExpression::from(DataExpression::from(b.clone())),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::UndeclaredFreeVariable {
            location: at_x(),
            variable: "b".into()
        })
    );
}

#[test]
fn test_undeclared_propositional_variable_is_rejected() {
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", &[]),
            PbesExpression::from(PropositionalVariableInstantiation::new("Y", &[])),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::UndeclaredPropositionalVariable {
            location: at_x(),
            name: "Y".into()
        })
    );
}

#[test]
fn test_instantiation_mismatch_is_rejected() {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());

    // Too few arguments, in the initial state.
    let error = check(
        &[],
        vec![PbesEquation::new(
            FixpointSymbol::Mu,
            PropositionalVariable::new("X", std::slice::from_ref(&b)),
            PbesExpression::true_constant(),
        )],
    );
    assert_eq!(
        error,
        Err(PbesWellTypedError::InstantiationMismatch {
            location: PbesLocation::Init,
            instantiation: "X".into(),
            declaration: "X(b: Bool)".into(),
        })
    );

    // An argument of the wrong sort.
    let x = PbesEquation::new(
        FixpointSymbol::Mu,
        PropositionalVariable::new("X", &[]),
        PbesExpression::from(PropositionalVariableInstantiation::new("Y", &[n.clone().into()])),
    );
    let y = PbesEquation::new(
        FixpointSymbol::Mu,
        PropositionalVariable::new("Y", std::slice::from_ref(&b)),
        PbesExpression::true_constant(),
    );
    let error = check(std::slice::from_ref(&n), vec![x, y]);
    assert_eq!(
        error,
        Err(PbesWellTypedError::InstantiationMismatch {
            location: at_x(),
            instantiation: "Y(n)".into(),
            declaration: "Y(b: Bool)".into(),
        })
    );
}
