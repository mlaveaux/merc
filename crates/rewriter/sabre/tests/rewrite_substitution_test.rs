//! Tests for applying a substitution to a data expression, see
//! [merc_sabre::utilities::apply_substitution].

use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;

use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_sabre::utilities::EmptySubstitution;
use merc_sabre::utilities::SliceSubstitution;
use merc_sabre::utilities::apply_substitution;

/// Parses `input` treating every name in `variables` as a variable.
fn term(input: &str, variables: &[&str]) -> DataExpression {
    let variables: AHashSet<String> = variables.iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

/// Builds a substitution mapping each name to the corresponding parsed closed term.
fn substitution(entries: &[(&str, &str)]) -> HashMap<DataVariable, DataExpression> {
    let mut sigma = HashMap::new();
    for (name, value) in entries {
        sigma.insert(DataVariable::new(*name), term(value, &[]));
    }
    sigma
}

#[test]
fn test_apply_substitution_replaces_variables() {
    let sigma = substitution(&[("x", "a")]);

    assert_eq!(
        apply_substitution(&term("f(x, g(x), b)", &["x"]), &sigma),
        term("f(a, g(a), b)", &[])
    );
}

#[test]
fn test_apply_substitution_bare_variable() {
    let sigma = substitution(&[("x", "s(a)")]);

    assert_eq!(apply_substitution(&term("x", &["x"]), &sigma), term("s(a)", &[]));
}

#[test]
fn test_apply_substitution_leaves_variables_outside_the_domain() {
    let sigma = substitution(&[("x", "a")]);
    let input = term("f(x, y)", &["x", "y"]);

    assert_eq!(apply_substitution(&input, &sigma), term("f(a, y)", &["y"]));
}

#[test]
fn test_apply_substitution_preserves_closed_terms() {
    let sigma = substitution(&[("x", "a")]);
    let input = term("f(b, g(c))", &[]);

    assert_eq!(apply_substitution(&input, &sigma), input);
}

#[test]
fn test_apply_substitution_empty_is_identity() {
    let input = term("f(x, g(y))", &["x", "y"]);

    assert_eq!(apply_substitution(&input, &EmptySubstitution), input);
}

#[test]
fn test_slice_substitution_replaces_variables() {
    let variables = [DataVariable::new("x"), DataVariable::new("y")];
    let values = [term("a", &[]), term("b", &[])];
    let sigma = SliceSubstitution::new(&variables, &values);

    assert_eq!(
        apply_substitution(&term("f(x, y)", &["x", "y"]), &sigma),
        term("f(a, b)", &[])
    );
}
