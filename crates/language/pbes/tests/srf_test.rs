//! The conversion to SRF is tested by what it must preserve and guarantee, not by the shape of the
//! summands it produces: [test_random_srf_preserves_the_solution] solves random PBESs with `Bool`
//! parameters with a small solver below and checks that the solution does not change, the result
//! is in SRF, and the corner cases that used to loop or capture variables are handled.

use std::collections::HashMap;

use merc_data::BoolKind;
use merc_data::DataExpression;
use merc_data::DataExpressionKind;
use merc_data::DataExpressionRef;
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
use merc_pbes::SrfPbes;
use merc_pbes::is_simple_expression;
use merc_pbes::is_srf;
use merc_pbes::pbes_to_pre_srf;
use merc_pbes::pbes_to_srf;
use merc_utilities::MercError;
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

/// Converts `pbes` to (pre-)SRF, with and without merging simple expressions, and checks that the
/// result is in SRF, has simple conditions, and gives every original equation the same solution.
fn assert_conversions_preserve_the_solution(pbes: &Pbes) {
    let expected = solve(pbes);

    for merge in [false, true] {
        for (convert, name) in [
            (
                pbes_to_srf as fn(&Pbes, bool) -> Result<SrfPbes, MercError>,
                "pbes_to_srf",
            ),
            (pbes_to_pre_srf, "pbes_to_pre_srf"),
        ] {
            let srf = convert(pbes, merge).unwrap();
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
}

/// `sigma X(b: Bool) = exists/forall c: Bool. X(c)` recurses through a quantifier, which the
/// conversion defers to a spawned equation: the solution must stay `true` for `nu` and `false`
/// for `mu`, for both quantifiers.
#[test]
fn test_recursion_through_a_quantifier_keeps_the_fixpoint() {
    let c = DataVariable::with_sort("c", SortExpression::bool().copy());
    let body = x(&[c.clone().into()]);

    for symbol in [FixpointSymbol::Nu, FixpointSymbol::Mu] {
        for quantified in [
            PbesExpression::exists(std::slice::from_ref(&c), &body),
            PbesExpression::forall(std::slice::from_ref(&c), &body),
        ] {
            let pbes = single(symbol, quantified);
            let expected = symbol == FixpointSymbol::Nu;
            for valuation in [false, true] {
                assert_eq!(solve(&pbes)[&("X".to_string(), vec![valuation])], expected, "{pbes}");
            }

            assert_conversions_preserve_the_solution(&pbes);
        }
    }
}

/// Random PBESs keep their solution under the conversions to (pre-)SRF.
#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn test_random_srf_preserves_the_solution() {
    random_test(200, |rng| assert_conversions_preserve_the_solution(&random_pbes(rng)));
}
