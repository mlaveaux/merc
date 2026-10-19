use std::ops::ControlFlow;

use ahash::AHashSet;
use merc_aterm::Term;
use merc_utilities::Step;

use crate::DataExpressionRef;
use crate::is_data_variable;
use crate::visit_data_expr;

/// Returns true iff `term` contains no data variables, i.e. it is a ground term.
///
/// A closed term normalises to the same result under every substitution, which is what makes
/// it sound to cache its normal form across calls that pass different substitutions.
///
/// Occurrences of variables bound by a binder or a where clause also count, so such a term may
/// be reported as not closed even though it is; that is conservative for the caching above.
pub fn is_closed<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    // Terms are maximally shared, so the same subterm is typically reachable along many paths.
    // Remembering the ones already seen keeps this linear in the size of the term graph rather
    // than the size of the tree it unfolds to. Keying on the term address is only valid because
    // no terms are created here, so no garbage collection can run during the traversal.
    let mut visited = AHashSet::new();

    let variable: Option<()> = visit_data_expr(&DataExpressionRef::from(term.copy()), (), |expr, context| {
        if !visited.insert(expr.index()) {
            ControlFlow::Continue(Step::Prune)
        } else if is_data_variable(expr) {
            ControlFlow::Break(())
        } else {
            // A function symbol and a machine number have no children, and the head function
            // symbol of an application is closed by definition, so neither needs to be recognised
            // separately here.
            ControlFlow::Continue(Step::Into(context))
        }
    });

    variable.is_none()
}

#[cfg(test)]
mod tests {
    use ahash::AHashSet;

    use crate::DataExpression;
    use crate::is_closed;

    #[test]
    fn test_is_closed_ground_term() {
        let term = DataExpression::from_string("s(s(a), b)").unwrap();
        assert!(is_closed(&term));
    }

    #[test]
    fn test_is_closed_with_variable() {
        let variables = AHashSet::from_iter(["x".to_string()]);
        let term = DataExpression::from_string_untyped("s(s(x), b)", &variables).unwrap();
        assert!(!is_closed(&term));
    }

    #[test]
    fn test_is_closed_bare_variable() {
        let variables = AHashSet::from_iter(["x".to_string()]);
        let term = DataExpression::from_string_untyped("x", &variables).unwrap();
        assert!(!is_closed(&term));
    }
}
