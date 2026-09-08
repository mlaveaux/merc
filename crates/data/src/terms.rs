use crate::BasicSort;
use crate::DataApplication;
use crate::DataExpression;
use crate::DataFunctionSymbol;
use crate::SortArrow;
use crate::SortExpression;
use crate::is_data_application;

/// Function symbol names for mCRL2's `Bool` connectives, exactly as they occur
/// in the standard prelude.
const AND: &str = "&&";
const OR: &str = "||";
const NOT: &str = "!";
const EQ: &str = "==";
const NEQ: &str = "!=";
const IMPLIES: &str = "=>";

fn bool_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("Bool"))
}

/// Builds the function symbol named `name` with the function sort `arrow`.
fn function_symbol(name: &'static str, arrow: SortArrow) -> DataFunctionSymbol {
    let sort: SortExpression = arrow.into();
    DataFunctionSymbol::with_sort(name, sort.copy())
}

/// Returns the two arguments of `term` if it is a data application of the
/// function symbol named `symbol` to exactly two arguments.
fn binary_application(term: &DataExpression, symbol: &str) -> Option<(DataExpression, DataExpression)> {
    if !is_data_application(term) || term.data_arguments().len() != 2 {
        return None;
    }
    if term.data_function_symbol().name().value() != symbol {
        return None;
    }
    Some((term.data_arg(0).protect(), term.data_arg(1).protect()))
}

/// Returns the single argument of `term` if it is a data application of the
/// function symbol named `symbol` to exactly one argument.
fn unary_application(term: &DataExpression, symbol: &str) -> Option<DataExpression> {
    if !is_data_application(term) || term.data_arguments().len() != 1 {
        return None;
    }
    if term.data_function_symbol().name().value() != symbol {
        return None;
    }
    Some(term.data_arg(0).protect())
}

/// Returns the two sides of `term` if it is a conjunction (`lhs && rhs`).
pub fn is_and(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    binary_application(term, AND)
}

/// Returns the two sides of `term` if it is a disjunction (`lhs || rhs`).
pub fn is_or(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    binary_application(term, OR)
}

/// Returns the operand of `term` if it is a negation (`!operand`).
pub fn is_not(term: &DataExpression) -> Option<DataExpression> {
    unary_application(term, NOT)
}

/// Returns the two sides of `term` if it is an equality (`lhs == rhs`).
pub fn is_equal(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    binary_application(term, EQ)
}

/// Returns the two sides of `term` if it is a disequality (`lhs != rhs`).
pub fn is_not_equal(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    binary_application(term, NEQ)
}

/// Returns the antecedent and consequent of `term` if it is an implication
/// (`lhs => rhs`).
pub fn is_implies(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    binary_application(term, IMPLIES)
}

/// Builds `lhs && rhs`.
pub fn make_and(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    let symbol = function_symbol(AND, SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()));
    DataApplication::with_args(&symbol, &[lhs, rhs]).into()
}

/// Builds `lhs || rhs`.
pub fn make_or(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    let symbol = function_symbol(OR, SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()));
    DataApplication::with_args(&symbol, &[lhs, rhs]).into()
}

/// Builds `!operand`.
pub fn make_not(operand: DataExpression) -> DataExpression {
    let symbol = function_symbol(NOT, SortArrow::new(&[bool_sort()], bool_sort()));
    DataApplication::with_args(&symbol, &[operand]).into()
}

/// Builds `lhs => rhs`.
pub fn make_implies(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    let symbol = function_symbol(IMPLIES, SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()));
    DataApplication::with_args(&symbol, &[lhs, rhs]).into()
}

/// Builds `lhs == rhs`, where both sides have sort `domain`.
pub fn make_equal(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    let symbol = function_symbol(EQ, SortArrow::new(&[domain.clone(), domain], bool_sort()));
    DataApplication::with_args(&symbol, &[lhs, rhs]).into()
}

/// Builds `lhs != rhs`, where both sides have sort `domain`.
pub fn make_not_equal(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    let symbol = function_symbol(NEQ, SortArrow::new(&[domain.clone(), domain], bool_sort()));
    DataApplication::with_args(&symbol, &[lhs, rhs]).into()
}
