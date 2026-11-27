//! The structural cases are mostly ported from `libraries/pbes/test/srf_pbes_test.cpp` of mCRL2.
//! The PBESs are built directly rather than parsed, since the conversion only looks at their
//! structure: data expressions are applications of function symbols that merely carry the right
//! sorts. [test_random_srf_preserves_the_solution] checks the semantics: on random PBESs with
//! `Bool` parameters, which a small solver below can solve, the solution must not change.

use std::collections::HashMap;

use merc_data::BoolKind;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionKind;
use merc_data::DataExpressionRef;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortExpression;
use merc_data::is_bool_literal;
use merc_pbes::FixpointSymbol;
use merc_pbes::Pbes;
use merc_pbes::PbesEquation;
use merc_pbes::PbesExpression;
use merc_pbes::PbesExpressionRef;
use merc_pbes::PbesKind;
use merc_pbes::PbesWellTypedError;
use merc_pbes::PropositionalVariable;
use merc_pbes::PropositionalVariableInstantiation;
use merc_pbes::SrfEquation;
use merc_pbes::SrfPbes;
use merc_pbes::is_simple_expression;
use merc_pbes::is_srf;
use merc_pbes::pbes_to_pre_srf;
use merc_pbes::pbes_to_srf;
use merc_utilities::Boolean;
use merc_utilities::MercError;
use merc_utilities::optimized_conjunction;
use merc_utilities::random_test;
use rand::RngExt;
use rand::rngs::StdRng;

/// The PBES `sigma X(b: Bool) = formula; init X(true);` of most tests.
fn single(symbol: FixpointSymbol, formula: PbesExpression) -> Pbes {
    let b = DataVariable::with_sort("b", SortExpression::bool().copy());
    Pbes::new(
        Mcrl2DataSpecification::default(),
        Vec::new(),
        vec![PbesEquation::new(
            symbol,
            PropositionalVariable::new("X", &[b]),
            formula,
        )],
        PropositionalVariableInstantiation::new("X", &[DataExpression::bool_literal(true)]),
    )
}

/// The parameter `b` of [single].
fn b() -> DataExpression {
    DataVariable::with_sort("b", SortExpression::bool().copy()).into()
}

/// `X(arguments)`.
fn x(arguments: &[DataExpression]) -> PbesExpression {
    PropositionalVariableInstantiation::new("X", arguments).into()
}

/// The application of the binary `Nat` operator `name`, of the given result sort.
fn nat_operator(name: &str, codomain: &SortExpression, lhs: &DataExpression, rhs: &DataExpression) -> DataExpression {
    let nat = SortExpression::nat();
    let symbol = DataFunctionSymbol::with_signature(name, &[nat.clone(), nat], codomain);
    DataApplication::with_args(&symbol, &[lhs.copy(), rhs.copy()]).into()
}

fn equation<'a>(srf: &'a SrfPbes, name: &str) -> &'a SrfEquation {
    srf.equations()
        .find(|equation| equation.variable().name().value() == name)
        .unwrap_or_else(|| panic!("equation {name} not found"))
}

/// The equation of `X`, which is the first one.
fn main_equation(srf: &SrfPbes) -> &SrfEquation {
    let main = srf.equations().next().unwrap();
    assert_eq!(main.variable().name().value(), "X");
    main
}

// A bare PVI in a disjunctive equation produces summand ([], true, PVI).
#[test]
fn test_srf_or_pvi() {
    let srf = pbes_to_srf(&single(FixpointSymbol::Nu, x(&[b()])), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    assert_eq!(main.summands().len(), 1);
    let summand = &main.summands()[0];
    assert!(summand.parameters().is_empty());
    assert!(summand.condition().is_true());
    assert_eq!(
        summand.variable(),
        &PropositionalVariableInstantiation::new("X", &[b()])
    );
}

// A disjunction of two PVIs produces two summands, each with condition true.
#[test]
fn test_srf_or_two_pvis() {
    let formula = PbesExpression::or(
        &x(&[DataExpression::bool_literal(true)]),
        &x(&[DataExpression::bool_literal(false)]),
    );
    let srf = pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    assert_eq!(main.summands().len(), 2);
    for summand in main.summands() {
        assert!(summand.parameters().is_empty());
        assert!(summand.condition().is_true());
        assert_eq!(summand.variable().name().value(), "X");
    }
}

// An exists with a PVI body in a disjunctive equation recurses into the body and prepends the
// bound variable as the summand parameter.
#[test]
fn test_srf_or_exists_pvi_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let formula = PbesExpression::exists(
        std::slice::from_ref(&n),
        &x(&[DataExpression::equal(
            &SortExpression::nat(),
            &n.clone().into(),
            &DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into(),
        )]),
    );
    let srf = pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    assert_eq!(main.summands().len(), 1);
    let summand = &main.summands()[0];
    assert_eq!(summand.parameters(), [n]);
    assert!(summand.condition().is_true());
    assert_eq!(summand.variable().name().value(), "X");
}

// A forall with a PVI body in a conjunctive equation recurses into the body and prepends the
// bound variable as the summand parameter.
#[test]
fn test_srf_and_forall_pvi_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let formula = PbesExpression::forall(
        std::slice::from_ref(&n),
        &x(&[DataExpression::equal(
            &SortExpression::nat(),
            &n.clone().into(),
            &DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into(),
        )]),
    );
    let srf = pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(main.is_conjunctive());
    assert_eq!(main.summands().len(), 1);
    let summand = &main.summands()[0];
    assert_eq!(summand.parameters(), [n]);
    assert!(summand.condition().is_true());
    assert_eq!(summand.variable().name().value(), "X");
}

// A conjunction of two PVIs is conjunctive; both PVIs become separate summands ([], true, PVI).
#[test]
fn test_srf_and_two_pvis() {
    let formula = PbesExpression::and(
        &x(&[DataExpression::bool_literal(true)]),
        &x(&[DataExpression::bool_literal(false)]),
    );
    let srf = pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(main.is_conjunctive());
    assert_eq!(main.summands().len(), 2);
    for summand in main.summands() {
        assert!(summand.parameters().is_empty());
        assert!(summand.condition().is_true());
        assert_eq!(summand.variable().name().value(), "X");
    }
}

/// Checks that the main equation has two summands, one for `X(b)` and one, with condition true
/// and no parameters, for the single equation that was spawned with the same fixpoint symbol.
fn assert_spawned_one(srf: &SrfPbes, conjunctive: bool) {
    // The original equation, the spawned one, X_false and X_true.
    assert_eq!(srf.equations().count(), 4);

    let main = main_equation(srf);
    assert_eq!(main.is_conjunctive(), conjunctive);
    assert_eq!(main.summands().len(), 2);
    let fresh: Vec<_> = main
        .summands()
        .iter()
        .filter(|summand| summand.variable().name().value() != "X")
        .collect();
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].parameters().is_empty());
    assert!(fresh[0].condition().is_true());

    // The spawned equation directly follows its origin, so it lies in the same fixpoint block.
    let spawned = srf.equations().nth(1).unwrap();
    assert_eq!(spawned.variable().name(), fresh[0].variable().name());
    assert_eq!(spawned.symbol(), main.symbol());
    assert_eq!(spawned.is_conjunctive(), !conjunctive);
}

// A forall with a non-simple body in a disjunctive equation cannot be inlined; a fresh
// equation is introduced and the main equation gets a summand pointing to it.
#[test]
fn test_srf_or_forall_non_simple_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let formula = PbesExpression::or(
        &x(&[b()]),
        &PbesExpression::forall(
            std::slice::from_ref(&n),
            &x(&[DataExpression::equal(
                &SortExpression::nat(),
                &n.clone().into(),
                &DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into(),
            )]),
        ),
    );
    assert_spawned_one(&pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap(), false);
}

// Dually, an exists with a non-simple body in a conjunctive equation.
#[test]
fn test_srf_and_exists_non_simple_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let formula = PbesExpression::and(
        &x(&[b()]),
        &PbesExpression::exists(
            std::slice::from_ref(&n),
            &x(&[DataExpression::equal(
                &SortExpression::nat(),
                &n.clone().into(),
                &DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into(),
            )]),
        ),
    );
    assert_spawned_one(&pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap(), true);
}

// `val(b) && X(b)` is disjunctive (simple && PVI does not make an `&&` conjunctive); the simple
// side becomes the condition of the summand rather than a fresh equation.
#[test]
fn test_srf_or_and_simple_left() {
    let formula = PbesExpression::and(&b().into(), &x(&[b()]));
    let srf = pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    assert_eq!(main.summands().len(), 1);
    let summand = &main.summands()[0];
    assert!(summand.parameters().is_empty());
    assert_eq!(summand.condition(), &PbesExpression::from(b()));
    assert_eq!(summand.variable().name().value(), "X");
}

// `val(b) || X(b)` is conjunctive (simple || PVI); the negated simple side becomes the
// condition of the summand.
#[test]
fn test_srf_and_or_simple_left() {
    let formula = PbesExpression::or(&b().into(), &x(&[b()]));
    let srf = pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(main.is_conjunctive());
    assert_eq!(main.summands().len(), 1);
    let summand = &main.summands()[0];
    assert!(summand.parameters().is_empty());
    assert_eq!(summand.condition(), &PbesExpression::not(&b().into()));
    assert_eq!(summand.variable().name().value(), "X");
}

// `X(b) || (X(true) && X(false))`: the `&&` in a disjunctive equation has no simple side, so a
// fresh equation is introduced for it.
#[test]
fn test_srf_or_and_both_non_simple() {
    let formula = PbesExpression::or(
        &x(&[b()]),
        &PbesExpression::and(
            &x(&[DataExpression::bool_literal(true)]),
            &x(&[DataExpression::bool_literal(false)]),
        ),
    );
    assert_spawned_one(&pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap(), false);
}

// Dually, `X(b) && (X(true) || X(false))`.
#[test]
fn test_srf_and_or_both_non_simple() {
    let formula = PbesExpression::and(
        &x(&[b()]),
        &PbesExpression::or(
            &x(&[DataExpression::bool_literal(true)]),
            &x(&[DataExpression::bool_literal(false)]),
        ),
    );
    assert_spawned_one(&pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap(), true);
}

// In pre-SRF, a simple clause of an `||` chain in a disjunctive equation gets its own summand
// (to X_true), and the other clauses get its negation added to their condition.
#[test]
fn test_pre_srf_or_simple_clause() {
    let formula = PbesExpression::or(
        &PbesExpression::or(&b().into(), &x(&[DataExpression::bool_literal(true)])),
        &x(&[DataExpression::bool_literal(false)]),
    );
    let srf = pbes_to_pre_srf(&single(FixpointSymbol::Nu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    assert_eq!(main.summands().len(), 3);
    let x_true = srf.true_equation().variable().name();
    for summand in main.summands() {
        if summand.variable().name() == x_true {
            assert_eq!(summand.condition(), &PbesExpression::from(b()));
        } else {
            assert_eq!(summand.condition(), &PbesExpression::not(&b().into()));
        }
    }
    let to_true = main.summands().iter().filter(|s| s.variable().name() == x_true);
    assert_eq!(to_true.count(), 1);
}

// Dually, a simple clause of an `&&` chain in a conjunctive equation gets its own summand (to
// X_false, negated), and the other clauses get it added to their condition.
#[test]
fn test_pre_srf_and_simple_clause() {
    let formula = PbesExpression::and(
        &PbesExpression::and(&b().into(), &x(&[DataExpression::bool_literal(true)])),
        &x(&[DataExpression::bool_literal(false)]),
    );
    let srf = pbes_to_pre_srf(&single(FixpointSymbol::Mu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(main.is_conjunctive());
    assert_eq!(main.summands().len(), 3);
    let x_false = srf.false_equation().variable().name();
    for summand in main.summands() {
        if summand.variable().name() == x_false {
            assert_eq!(summand.condition(), &PbesExpression::not(&b().into()));
        } else {
            assert_eq!(summand.condition(), &PbesExpression::from(b()));
        }
    }
    let to_false = main.summands().iter().filter(|s| s.variable().name() == x_false);
    assert_eq!(to_false.count(), 1);
}

// A simple exists inside a conjunctive equation is a leaf: it becomes the negated condition
// of a summand to X_false, under the enclosing forall variable.
// See also https://github.com/mCRL2org/mCRL2/issues/1913.
#[test]
fn test_srf_and_exists_simple_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let k = DataVariable::with_sort("k", SortExpression::nat().copy());
    let less = nat_operator("<", &SortExpression::bool(), &k.clone().into(), &n.clone().into());
    let simple = PbesExpression::exists(std::slice::from_ref(&k), &less.into());
    let formula = PbesExpression::forall(std::slice::from_ref(&n), &PbesExpression::and(&simple, &x(&[b()])));
    let srf = pbes_to_srf(&single(FixpointSymbol::Mu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(main.is_conjunctive());
    assert_eq!(main.summands().len(), 2);
    let x_false = srf.false_equation().variable().name();
    let to_false: Vec<_> = main
        .summands()
        .iter()
        .filter(|s| s.variable().name() == x_false)
        .collect();
    assert_eq!(to_false.len(), 1);
    assert_eq!(to_false[0].parameters(), [n]);
    assert_eq!(to_false[0].condition(), &PbesExpression::not(&simple));
}

// A simple forall inside a disjunctive equation is a leaf: it becomes the condition of a
// summand to X_true, without parameters.
// See also https://github.com/mCRL2org/mCRL2/issues/1913.
#[test]
fn test_srf_or_forall_simple_body() {
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let less = nat_operator(
        "<",
        &SortExpression::bool(),
        &n.clone().into(),
        &DataFunctionSymbol::with_sort("5", SortExpression::nat().copy()).into(),
    );
    let simple = PbesExpression::forall(std::slice::from_ref(&n), &less.into());
    let formula = PbesExpression::or(&PbesExpression::or(&x(&[b()]), &simple), &x(&[b()]));
    let srf = pbes_to_srf(&single(FixpointSymbol::Nu, formula), true).unwrap();
    assert_eq!(srf.equations().count(), 3);

    let main = main_equation(&srf);
    assert!(!main.is_conjunctive());
    // mCRL2 has 3 summands here: for plain SRF it neither flattens the `||` chain nor merges its
    // simple clauses, so `X(b)` occurs twice. Here the chain is split into distinct clauses.
    assert_eq!(main.summands().len(), 2);
    let x_true = srf.true_equation().variable().name();
    let leaf: Vec<_> = main
        .summands()
        .iter()
        .filter(|s| s.variable().name() == x_true)
        .collect();
    assert_eq!(leaf.len(), 1);
    assert!(leaf[0].parameters().is_empty());
    assert_eq!(leaf[0].condition(), &simple);
}

/// The first part of the example from A. Stramaglia, J.J.A. Keiren, M. Laveaux and
/// T.A.C. Willemse, "Efficient Evidence Generation for Modal mu-Calculus Model Checking",
/// TACAS 2025: in pre-SRF the counter-example variables stay in the conditions, so `X` gets
/// exactly one summand per instantiation of `X` and `Y` and one to `X_true`; in SRF they are
/// propositional variables like any other.
#[test]
fn test_example_stramaglia_paper() {
    let s = DataVariable::with_sort("s", SortExpression::nat().copy());
    let s1 = DataVariable::with_sort("s1", SortExpression::nat().copy());
    let n = DataVariable::with_sort("n", SortExpression::nat().copy());
    let s_: DataExpression = s.clone().into();
    let n_: DataExpression = n.clone().into();
    let sn = nat_operator("+", &SortExpression::nat(), &s_, &n_);
    let less = |lhs: &DataExpression, rhs: &DataExpression| nat_operator("<", &SortExpression::bool(), lhs, rhs);
    let instantiation = |name: &str, arguments: &[DataExpression]| -> PbesExpression {
        PropositionalVariableInstantiation::new(name, arguments).into()
    };

    // mu X(s: Nat) = (exists n: Nat. val(s == 1) && val(0 < n && n < 3)
    //                  && (X(s+n) || Zneg_0_(s, s+n)) && Zpos_0_(s, s+n)) || Y(s);
    let body = optimized_conjunction([
        DataExpression::equal(
            &SortExpression::nat(),
            &s_,
            &DataFunctionSymbol::with_sort("1", SortExpression::nat().copy()).into(),
        )
        .into(),
        DataExpression::and(
            &less(
                &DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into(),
                &n_,
            ),
            &less(
                &n_,
                &DataFunctionSymbol::with_sort("3", SortExpression::nat().copy()).into(),
            ),
        )
        .into(),
        PbesExpression::or(
            &instantiation("X", std::slice::from_ref(&sn)),
            &instantiation("Zneg_0_", &[s_.clone(), sn.clone()]),
        ),
        instantiation("Zpos_0_", &[s_.clone(), sn.clone()]),
    ]);
    let x = PbesExpression::or(
        &PbesExpression::exists(std::slice::from_ref(&n), &body),
        &instantiation("Y", std::slice::from_ref(&s_)),
    );

    // nu Y(s: Nat) = val(s == 3) && (Y(s) || Zneg_1_(s, s)) && Zpos_1_(s, s);
    let y = optimized_conjunction([
        DataExpression::equal(
            &SortExpression::nat(),
            &s_,
            &DataFunctionSymbol::with_sort("3", SortExpression::nat().copy()).into(),
        )
        .into(),
        PbesExpression::or(
            &instantiation("Y", std::slice::from_ref(&s_)),
            &instantiation("Zneg_1_", &[s_.clone(), s_.clone()]),
        ),
        instantiation("Zpos_1_", &[s_.clone(), s_.clone()]),
    ]);

    let parameters = [s.clone(), s1];
    let equation_of = |symbol, name: &str, parameters: &[DataVariable], formula| {
        PbesEquation::new(symbol, PropositionalVariable::new(name, parameters), formula)
    };
    let pbes = Pbes::new(
        Mcrl2DataSpecification::default(),
        Vec::new(),
        vec![
            equation_of(FixpointSymbol::Mu, "X", std::slice::from_ref(&s), x),
            equation_of(FixpointSymbol::Nu, "Y", std::slice::from_ref(&s), y),
            equation_of(
                FixpointSymbol::Nu,
                "Zpos_0_",
                &parameters,
                PbesExpression::true_constant(),
            ),
            equation_of(
                FixpointSymbol::Nu,
                "Zpos_1_",
                &parameters,
                PbesExpression::true_constant(),
            ),
            equation_of(
                FixpointSymbol::Mu,
                "Zneg_0_",
                &parameters,
                PbesExpression::false_constant(),
            ),
            equation_of(
                FixpointSymbol::Mu,
                "Zneg_1_",
                &parameters,
                PbesExpression::false_constant(),
            ),
        ],
        PropositionalVariableInstantiation::new(
            "X",
            &[DataFunctionSymbol::with_sort("0", SortExpression::nat().copy()).into()],
        ),
    );

    let pre_srf = pbes_to_pre_srf(&pbes, true).unwrap();
    let targets: Vec<_> = equation(&pre_srf, "X")
        .summands()
        .iter()
        .map(|summand| summand.variable().name().value().to_string())
        .collect();
    assert_eq!(targets.len(), 3, "{targets:?}");
    for target in ["X", "Y", pre_srf.true_equation().variable().name().value()] {
        assert!(
            targets.iter().any(|name| name == target),
            "{target} missing in {targets:?}"
        );
    }
    // No counter-example variable is a summand target, they remain in the conditions.
    assert!(targets.iter().all(|name| !name.starts_with('Z')), "{targets:?}");
}

/// `X(b: Bool) = val(b) || exists b: Bool . Y(b)`: moving the guard `!val(b)` under the
/// existential would capture it, so the PBES is rejected as ill-typed instead.
#[test]
fn test_quantifier_shadowing_parameter_is_rejected() {
    let b_variable = DataVariable::with_sort("b", SortExpression::bool().copy());
    let parameters = std::slice::from_ref(&b_variable);
    let y_b: PbesExpression = PropositionalVariableInstantiation::new("Y", &[b()]).into();
    let pbes = Pbes::new(
        Mcrl2DataSpecification::default(),
        Vec::new(),
        vec![
            PbesEquation::new(
                FixpointSymbol::Nu,
                PropositionalVariable::new("X", parameters),
                PbesExpression::or(&b().into(), &PbesExpression::exists(parameters, &y_b)),
            ),
            PbesEquation::new(
                FixpointSymbol::Nu,
                PropositionalVariable::new("Y", parameters),
                b().into(),
            ),
        ],
        PropositionalVariableInstantiation::new("X", &[DataExpression::bool_literal(false)]),
    );

    for merge in [false, true] {
        for convert in [
            pbes_to_srf as fn(&Pbes, bool) -> Result<SrfPbes, MercError>,
            pbes_to_pre_srf,
        ] {
            let error = convert(&pbes, merge).err().expect("expected an ill-typed PBES");
            assert_eq!(
                error.downcast_ref::<PbesWellTypedError>(),
                Some(&PbesWellTypedError::QuantifierShadowsParameter {
                    location: merc_pbes::PbesLocation::Equation("X".to_string()),
                    variable: "b".to_string()
                }),
                "got {error:?}"
            );
        }
    }
}

/// `X = true && Y` is classified as disjunctive by its one-sided shape; without merging it must
/// not spawn an identical equation forever, and the result is already in SRF.
#[test]
fn test_one_sided_without_merging_terminates() {
    let y: PbesExpression = PropositionalVariableInstantiation::new("Y", &[]).into();
    let pbes = Pbes::new(
        Mcrl2DataSpecification::default(),
        Vec::new(),
        vec![
            PbesEquation::new(
                FixpointSymbol::Mu,
                PropositionalVariable::new("X", &[]),
                PbesExpression::and(&PbesExpression::true_constant(), &y),
            ),
            PbesEquation::new(
                FixpointSymbol::Mu,
                PropositionalVariable::new("Y", &[]),
                PbesExpression::false_constant(),
            ),
        ],
        PropositionalVariableInstantiation::new("X", &[]),
    );

    let srf = pbes_to_srf(&pbes, false).unwrap();
    // X, Y, X_false and X_true.
    assert_eq!(srf.equations().count(), 4);
    assert!(is_srf(&pbes).unwrap());
    assert_eq!(
        solve(&srf.to_pbes()),
        solve(&pbes).into_iter().chain(srf_escapes(&srf)).collect()
    );
}

/// The solution of the `X_false`/`X_true` equations of `srf`.
fn srf_escapes(srf: &SrfPbes) -> HashMap<(String, Vec<bool>), bool> {
    HashMap::from([
        (
            (srf.false_equation().variable().name().value().to_string(), Vec::new()),
            false,
        ),
        (
            (srf.true_equation().variable().name().value().to_string(), Vec::new()),
            true,
        ),
    ])
}

#[test]
fn test_is_srf() {
    // `X(b) || X(!b)` is in SRF, `X(b) || (X(true) && X(false))` needs a new equation.
    let negated = DataExpression::negation(&b());
    assert!(
        is_srf(&single(
            FixpointSymbol::Nu,
            PbesExpression::or(&x(&[b()]), &x(&[negated]))
        ))
        .unwrap()
    );
    let nested = PbesExpression::or(
        &x(&[b()]),
        &PbesExpression::and(
            &x(&[DataExpression::bool_literal(true)]),
            &x(&[DataExpression::bool_literal(false)]),
        ),
    );
    assert!(!is_srf(&single(FixpointSymbol::Nu, nested)).unwrap());
}

/// A small PBES solver for PBESs whose parameters and quantified variables are all of sort `Bool`
/// and whose data expressions are `Bool` connectives over variables and literals: it instantiates
/// every equation for every valuation of its parameters and solves the resulting BES by nested
/// fixpoint iteration. Returns the value of every equation for every valuation.
fn solve(pbes: &Pbes) -> HashMap<(String, Vec<bool>), bool> {
    // Consecutive equations with the same fixpoint symbol form one block.
    let mut blocks: Vec<Vec<&PbesEquation>> = Vec::new();
    for equation in &pbes.equations {
        match blocks.last_mut() {
            Some(block) if block[0].symbol == equation.symbol => block.push(equation),
            _ => blocks.push(vec![equation]),
        }
    }

    let mut values = HashMap::new();
    solve_from(&blocks, &mut values);
    values
}

fn valuations(count: usize) -> Vec<Vec<bool>> {
    (0..1usize << count)
        .map(|bits| (0..count).map(|i| bits & (1 << i) != 0).collect())
        .collect()
}

fn solve_from(blocks: &[Vec<&PbesEquation>], values: &mut HashMap<(String, Vec<bool>), bool>) {
    let Some((block, inner)) = blocks.split_first() else {
        return;
    };

    let initial = block[0].symbol == FixpointSymbol::Nu;
    let mut keys = Vec::new();
    for equation in block {
        let parameters: Vec<String> = equation
            .variable
            .parameters()
            .iter()
            .map(|parameter| parameter.name().to_string())
            .collect();
        for valuation in valuations(parameters.len()) {
            let key = (equation.variable.name().value().to_string(), valuation.clone());
            values.insert(key.clone(), initial);
            let environment: HashMap<String, bool> = parameters.iter().cloned().zip(valuation).collect();
            keys.push((key, &equation.formula, environment));
        }
    }

    loop {
        solve_from(inner, values);
        let mut changed = false;
        for (key, formula, environment) in &keys {
            let value = evaluate(&formula.copy(), &mut environment.clone(), values);
            if values.insert(key.clone(), value) != Some(value) {
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

fn evaluate(
    expr: &PbesExpressionRef<'_>,
    environment: &mut HashMap<String, bool>,
    values: &HashMap<(String, Vec<bool>), bool>,
) -> bool {
    match expr.kind() {
        PbesKind::True => true,
        PbesKind::False => false,
        PbesKind::Data(data) => evaluate_data(&data, environment),
        PbesKind::Instantiation(instantiation) => {
            let arguments = instantiation
                .arguments()
                .iter()
                .map(|argument| evaluate_data(&argument.copy(), environment))
                .collect();
            values[&(instantiation.name().value().to_string(), arguments)]
        }
        PbesKind::Not(body) => !evaluate(&body, environment, values),
        PbesKind::And(lhs, rhs) => evaluate(&lhs, environment, values) && evaluate(&rhs, environment, values),
        PbesKind::Or(lhs, rhs) => evaluate(&lhs, environment, values) || evaluate(&rhs, environment, values),
        PbesKind::Imp(lhs, rhs) => !evaluate(&lhs, environment, values) || evaluate(&rhs, environment, values),
        PbesKind::Forall(variables, body) | PbesKind::Exists(variables, body) => {
            let forall = matches!(expr.kind(), PbesKind::Forall(..));
            let names: Vec<String> = variables.iter().map(|variable| variable.name().to_string()).collect();
            let mut result = forall;
            for valuation in valuations(names.len()) {
                let mut inner = environment.clone();
                inner.extend(names.iter().cloned().zip(valuation));
                if evaluate(&body, &mut inner, values) != forall {
                    result = !forall;
                    break;
                }
            }
            result
        }
    }
}

fn evaluate_data(expr: &DataExpressionRef<'_>, environment: &HashMap<String, bool>) -> bool {
    if let DataExpressionKind::Variable(variable) = expr.kind() {
        return environment[variable.name()];
    }
    match expr.bool_kind() {
        BoolKind::Not(body) => !evaluate_data(&body, environment),
        BoolKind::And(lhs, rhs) => evaluate_data(&lhs, environment) && evaluate_data(&rhs, environment),
        BoolKind::Or(lhs, rhs) => evaluate_data(&lhs, environment) || evaluate_data(&rhs, environment),
        BoolKind::Implies(lhs, rhs) => !evaluate_data(&lhs, environment) || evaluate_data(&rhs, environment),
        BoolKind::Equal(lhs, rhs) => evaluate_data(&lhs, environment) == evaluate_data(&rhs, environment),
        BoolKind::NotEqual(lhs, rhs) => evaluate_data(&lhs, environment) != evaluate_data(&rhs, environment),
        BoolKind::Other if is_bool_literal(expr, true) => true,
        BoolKind::Other if is_bool_literal(expr, false) => false,
        BoolKind::Other => panic!("unexpected data expression {expr}"),
    }
}

/// A random `Bool` data expression over the variables in `scope`.
fn random_data(rng: &mut StdRng, scope: &[String]) -> DataExpression {
    let atom = |rng: &mut StdRng| -> DataExpression {
        if scope.is_empty() || rng.random_bool(0.3) {
            DataExpression::bool_literal(rng.random_bool(0.5))
        } else {
            let name = &scope[rng.random_range(0..scope.len())];
            DataVariable::with_sort(name.as_str(), SortExpression::bool().copy()).into()
        }
    };
    match rng.random_range(0..4) {
        0 => DataExpression::negation(&atom(rng)),
        1 => DataExpression::and(&atom(rng), &atom(rng)),
        _ => atom(rng),
    }
}

/// A random normalised formula (no `!` or `=>` at the PBES level) over the equations `X0`, ...,
/// `X{equations - 1}`, each with one `Bool` parameter. Quantified variables get fresh names, so
/// that nothing shadows.
fn random_formula(
    rng: &mut StdRng,
    depth: usize,
    equations: usize,
    scope: &mut Vec<String>,
    fresh: &mut usize,
) -> PbesExpression {
    if depth == 0 || rng.random_bool(0.25) {
        return match rng.random_range(0..6) {
            0 => PbesExpression::true_constant(),
            1 => PbesExpression::false_constant(),
            2 => random_data(rng, scope).into(),
            _ => {
                let name = format!("X{}", rng.random_range(0..equations));
                PropositionalVariableInstantiation::new(&name, &[random_data(rng, scope)]).into()
            }
        };
    }

    match rng.random_range(0..6) {
        0 | 1 => PbesExpression::and(
            &random_formula(rng, depth - 1, equations, scope, fresh),
            &random_formula(rng, depth - 1, equations, scope, fresh),
        ),
        2 | 3 => PbesExpression::or(
            &random_formula(rng, depth - 1, equations, scope, fresh),
            &random_formula(rng, depth - 1, equations, scope, fresh),
        ),
        quantifier => {
            *fresh += 1;
            let variable = DataVariable::with_sort(format!("c{fresh}").as_str(), SortExpression::bool().copy());
            scope.push(variable.name().to_string());
            let body = random_formula(rng, depth - 1, equations, scope, fresh);
            scope.pop();
            if quantifier == 4 {
                PbesExpression::forall(&[variable], &body)
            } else {
                PbesExpression::exists(&[variable], &body)
            }
        }
    }
}

fn random_pbes(rng: &mut StdRng) -> Pbes {
    let count = rng.random_range(1..=3);
    let mut fresh = 0;
    let equations = (0..count)
        .map(|index| {
            let symbol = if rng.random_bool(0.5) {
                FixpointSymbol::Mu
            } else {
                FixpointSymbol::Nu
            };
            let mut scope = vec!["b".to_string()];
            let formula = random_formula(rng, 4, count, &mut scope, &mut fresh);
            let parameter = DataVariable::with_sort("b", SortExpression::bool().copy());
            PbesEquation::new(
                symbol,
                PropositionalVariable::new(&format!("X{index}"), &[parameter]),
                formula,
            )
        })
        .collect();
    Pbes::new(
        Mcrl2DataSpecification::default(),
        Vec::new(),
        equations,
        PropositionalVariableInstantiation::new("X0", &[DataExpression::bool_literal(true)]),
    )
}

/// Converting to (pre-)SRF, with and without merging simple expressions, yields a PBES in SRF in
/// which every original equation has the same solution as before.
#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn test_random_srf_preserves_the_solution() {
    random_test(200, |rng| {
        let pbes = random_pbes(rng);
        let expected = solve(&pbes);

        for merge in [false, true] {
            for (convert, name) in [
                (
                    pbes_to_srf as fn(&Pbes, bool) -> Result<SrfPbes, MercError>,
                    "pbes_to_srf",
                ),
                (pbes_to_pre_srf, "pbes_to_pre_srf"),
            ] {
                let srf = convert(&pbes, merge).unwrap();
                let context = format!("{name}(merge = {merge}) of\n{pbes}\ngives\n{}", srf.to_pbes());

                for equation in srf.equations() {
                    for summand in equation.summands() {
                        assert!(
                            is_simple_expression(&summand.condition().copy(), false),
                            "condition {} is not simple in {context}",
                            summand.condition()
                        );
                    }
                }

                let actual = solve(&srf.to_pbes());
                for (key, value) in &expected {
                    assert_eq!(actual.get(key), Some(value), "{key:?} differs in {context}");
                }
                assert!(is_srf(&srf.to_pbes()).unwrap(), "not in SRF: {context}");
            }
        }
    });
}
