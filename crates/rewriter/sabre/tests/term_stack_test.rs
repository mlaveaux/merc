//! Tests for instantiating a right-hand side from a matched left-hand side, see
//! [merc_sabre::utilities::TermStack].

use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;
use rand::Rng;
use rand::RngExt;

use merc_data::BinderType;
use merc_data::DataAbstraction;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::DataWhereClause;
use merc_data::DataWhrDecl;
use merc_sabre::Rule;
use merc_sabre::test_utility::create_rewrite_rule;
use merc_sabre::utilities::DataPosition;
use merc_sabre::utilities::DataPositionIndexed;
use merc_sabre::utilities::TermStack;
use merc_sabre::utilities::apply_substitution;
use merc_sabre::utilities::create_var_map;
use merc_utilities::random_test;

/// Parses `input` treating every name in `variables` as a variable.
fn term(input: &str, variables: &[&str]) -> DataExpression {
    let variables: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

#[test]
fn test_term_stack_instantiates_the_right_hand_side() {
    let stack = TermStack::new(&create_rewrite_rule("fact(s(N))", "times(s(N), fact(N))", &["N"]).unwrap());

    assert_eq!(
        stack.evaluate(&term("fact(s(a))", &[])),
        term("times(s(a), fact(a))", &[])
    );
    assert_eq!(
        stack.evaluate(&term("fact(s(g(b)))", &[])),
        term("times(s(g(b)), fact(g(b)))", &[])
    );
}

#[test]
fn test_term_stack_of_a_variable_right_hand_side() {
    let stack = TermStack::new(&create_rewrite_rule("f(x)", "x", &["x"]).unwrap());

    assert_eq!(stack.evaluate(&term("f(g(a))", &[])), term("g(a)", &[]));
}

#[test]
fn test_term_stack_from_an_explicit_variable_map() {
    let rhs = term("f(f(a,a),x)", &["x"]);

    let mut map = HashMap::new();
    map.insert(DataVariable::new("x"), DataPosition::new(&[1]));
    let stack = TermStack::from_term(&rhs.copy(), &map);

    assert_eq!(stack.evaluate(&term("g(b)", &[])), term("f(f(a,a),b)", &[]));
}

/// A binder in the right-hand side keeps its own bound variable: substituting the matched `y`
/// for `x` in `exists y. f(x, y)` would capture it, so the value is bound by a where clause.
#[test]
fn test_term_stack_does_not_capture_a_value_in_a_binder() {
    let variables = ["x", "y"];
    let x = DataVariable::from(term("x", &variables));
    let y = DataVariable::from(term("y", &variables));
    let binder: DataExpression = DataAbstraction::new(
        BinderType::Exists,
        std::slice::from_ref(&y),
        term("f(x, y)", &variables),
    )
    .into();

    let h = DataFunctionSymbol::new("h");
    let rhs: DataExpression = DataApplication::with_args(&h, std::slice::from_ref(&binder)).into();
    let stack = TermStack::new(&Rule::new(term("g(x)", &variables), rhs));

    let where_clause: DataExpression = DataWhereClause::new(binder, &[DataWhrDecl::new(x, y.into())]).into();
    let expected: DataExpression = DataApplication::with_args(&h, &[where_clause]).into();
    assert_eq!(stack.evaluate(&term("g(y)", &variables)), expected);
}

#[test]
fn test_create_var_map_points_at_each_variable() {
    let variables = ["x", "y"];
    let t = term("f(x, g(y, x))", &variables);

    let map = create_var_map(&t);

    let mut keys: Vec<DataVariable> = map.keys().cloned().collect();
    keys.sort();
    let mut expected = vec![DataVariable::new("x"), DataVariable::new("y")];
    expected.sort();
    assert_eq!(keys, expected);

    for (variable, position) in &map {
        assert_eq!(
            t.get_data_position(position),
            DataExpression::from(variable.clone()).copy()
        );
    }
}

/// Returns a random term of at most `depth` levels over `f/2`, `g/1` and the given leaves.
fn random_term(rng: &mut impl Rng, depth: usize, leaves: &[&str]) -> String {
    if depth == 0 || rng.random_bool(0.3) {
        return leaves[rng.random_range(0..leaves.len())].to_string();
    }

    if rng.random_bool(0.5) {
        format!("g({})", random_term(rng, depth - 1, leaves))
    } else {
        format!(
            "f({}, {})",
            random_term(rng, depth - 1, leaves),
            random_term(rng, depth - 1, leaves)
        )
    }
}

/// For a rule `l = r` and a substitution `sigma`, evaluating the stack of `r` on the matched
/// term `sigma(l)` gives `sigma(r)`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_term_stack_evaluates_to_the_substituted_right_hand_side() {
    const VARIABLES: [&str; 3] = ["x", "y", "z"];

    random_test(200, |rng| {
        // A left-hand side with at least one variable below a function symbol.
        let lhs_text = format!(
            "h({}, {})",
            VARIABLES[rng.random_range(0..VARIABLES.len())],
            random_term(rng, 3, &["x", "y", "z", "a"])
        );
        let lhs = term(&lhs_text, &VARIABLES);

        let lhs_variables: Vec<&str> = VARIABLES
            .iter()
            .copied()
            .filter(|variable| create_var_map(&lhs).contains_key(&DataVariable::new(*variable)))
            .collect();
        let mut rhs_leaves = lhs_variables.clone();
        rhs_leaves.extend(["a", "b"]);
        let rhs = term(&random_term(rng, 4, &rhs_leaves), &VARIABLES);

        let mut sigma = HashMap::new();
        for variable in &lhs_variables {
            sigma.insert(
                DataVariable::new(*variable),
                term(&random_term(rng, 3, &["a", "b"]), &[]),
            );
        }

        let stack = TermStack::new(&Rule::new(lhs.clone(), rhs.clone()));
        assert_eq!(
            stack.evaluate(&apply_substitution(&lhs, &sigma)),
            apply_substitution(&rhs, &sigma),
            "rule {lhs} = {rhs}"
        );
    });
}
