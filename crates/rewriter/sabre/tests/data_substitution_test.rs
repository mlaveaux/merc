//! Tests for replacing the subterm at a position, see [merc_sabre::utilities::data_substitute].

use merc_aterm::Protected;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_data::DataExpression;
use merc_sabre::utilities::DataPosition;
use merc_sabre::utilities::data_substitute;
use merc_sabre::utilities::data_substitute_with;

/// Replaces the subterm of `input` at `position` by `replacement`.
fn substitute(input: &str, position: &[usize], replacement: &str) -> DataExpression {
    let input = DataExpression::from_string(input).unwrap();
    let replacement = DataExpression::from_string(replacement).unwrap();
    THREAD_TERM_POOL.with(|tp| data_substitute(tp, &input.copy(), replacement, &DataPosition::new(position)))
}

#[test]
fn test_data_substitute_nested_position() {
    assert_eq!(
        substitute("s(s(a))", &[1, 1], "0"),
        DataExpression::from_string("s(s(0))").unwrap()
    );
}

#[test]
fn test_data_substitute_keeps_the_other_arguments() {
    assert_eq!(
        substitute("f(g(a), b, c)", &[2], "h(d)"),
        DataExpression::from_string("f(g(a), h(d), c)").unwrap()
    );
    assert_eq!(
        substitute("f(g(a), b, c)", &[1, 1], "x"),
        DataExpression::from_string("f(g(x), b, c)").unwrap()
    );
}

#[test]
fn test_data_substitute_at_the_root() {
    assert_eq!(
        substitute("f(g(a), b)", &[], "c"),
        DataExpression::from_string("c").unwrap()
    );
}

/// A builder reused across calls must not leak the arguments of an earlier substitution.
#[test]
fn test_data_substitute_with_a_reused_builder() {
    let mut builder = Protected::new(vec![]);
    let input = DataExpression::from_string("f(g(a, b), c)").unwrap();

    THREAD_TERM_POOL.with(|tp| {
        let first = data_substitute_with(
            &mut builder,
            tp,
            &input.copy(),
            DataExpression::from_string("d").unwrap(),
            &DataPosition::new(&[1, 2]),
        );
        assert_eq!(first, DataExpression::from_string("f(g(a, d), c)").unwrap());

        let second = data_substitute_with(
            &mut builder,
            tp,
            &input.copy(),
            DataExpression::from_string("e").unwrap(),
            &DataPosition::new(&[2]),
        );
        assert_eq!(second, DataExpression::from_string("f(g(a, b), e)").unwrap());
    });
}
