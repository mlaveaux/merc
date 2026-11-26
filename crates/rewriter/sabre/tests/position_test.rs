//! Tests for positions in terms and data expressions, see [merc_sabre::utilities::ExplicitPosition]
//! and [merc_sabre::utilities::DataPosition].

use merc_aterm::ATerm;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_sabre::utilities::DataPosition;
use merc_sabre::utilities::DataPositionIndexed;
use merc_sabre::utilities::DataPositionIterator;
use merc_sabre::utilities::ExplicitPosition;
use merc_sabre::utilities::PositionIndexed;
use merc_sabre::utilities::PositionIterator;

#[test]
fn test_get_position() {
    let t = ATerm::from_string("f(g(a),b)").unwrap();

    assert_eq!(t.get_position(&ExplicitPosition::new(&[])), t.copy());
    assert_eq!(
        t.get_position(&ExplicitPosition::new(&[1, 1])),
        ATerm::from_string("a").unwrap().copy()
    );
    assert_eq!(
        t.get_position(&ExplicitPosition::new(&[2])),
        ATerm::from_string("b").unwrap().copy()
    );
}

#[test]
fn test_position_iterator_visits_every_subterm_once() {
    let t = ATerm::from_string("f(g(a),b)").unwrap();

    let mut actual: Vec<(Vec<usize>, ATerm)> = PositionIterator::new(t.copy())
        .map(|(term, position)| (position.indices().to_vec(), term.protect()))
        .collect();
    actual.sort_by(|left, right| left.0.cmp(&right.0));

    let expected: Vec<(Vec<usize>, ATerm)> = [
        (vec![], "f(g(a),b)"),
        (vec![1], "g(a)"),
        (vec![1, 1], "a"),
        (vec![2], "b"),
    ]
    .into_iter()
    .map(|(position, term)| (position, ATerm::from_string(term).unwrap()))
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn test_get_data_position() {
    let t = DataExpression::from_string("f(g(a),b)").unwrap();

    assert_eq!(t.get_data_position(&DataPosition::new(&[])), t.copy());
    assert_eq!(
        t.get_data_position(&DataPosition::new(&[1, 1])),
        DataExpression::from_string("a").unwrap().copy()
    );
    assert_eq!(
        t.get_data_position(&DataPosition::new(&[2])),
        DataExpression::from_string("b").unwrap().copy()
    );
}

#[test]
fn test_data_position_iterator_visits_every_subterm_once() {
    let t = DataExpression::from_string("f(g(a),b)").unwrap();

    let mut actual: Vec<(Vec<usize>, DataExpression)> = DataPositionIterator::new(t.copy())
        .map(|(term, position)| (position.indices().to_vec(), term.protect()))
        .collect();
    actual.sort_by(|left, right| left.0.cmp(&right.0));

    let expected: Vec<(Vec<usize>, DataExpression)> = [
        (vec![], "f(g(a),b)"),
        (vec![1], "g(a)"),
        (vec![1, 1], "a"),
        (vec![2], "b"),
    ]
    .into_iter()
    .map(|(position, term)| (position, DataExpression::from_string(term).unwrap()))
    .collect();
    assert_eq!(actual, expected);
}
