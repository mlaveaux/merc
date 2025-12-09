use crate::StateFrm;



/// Applies the given substitution function recursively to the state formula.
/// 
/// The substitution function takes a state formula and returns an optional new
/// formula. If it returns `Some(new_formula)`, the substitution is applied and
/// the new formula is returned. If it returns `None`, the substitution is not
/// applied and the function continues to traverse the formula tree.
pub fn substitute(formula: StateFrm, substitution: &impl Fn(&StateFrm) -> Option<StateFrm>) -> StateFrm {
    if let Some(formula) = substitution(&formula) {
        // A substitution was made, return the new formula.
        return formula;
    }

    match formula {
        StateFrm::Binary { op, lhs, rhs } => {
            let new_lhs = substitute(*lhs, substitution);
            let new_rhs = substitute(*rhs, substitution);
            StateFrm::Binary {
                op,
                lhs: Box::new(new_lhs),
                rhs: Box::new(new_rhs),
            }
        }
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            let new_body = substitute(*body, substitution);
            StateFrm::FixedPoint {
                operator,
                variable,
                body: Box::new(new_body),
            }
        }
        StateFrm::Bound { bound, variables, body } => {
            let new_body = substitute(*body, substitution);
            StateFrm::Bound {
                bound,
                variables,
                body: Box::new(new_body),
            }
        }
        StateFrm::Modality {
            operator,
            formula,
            expr,
        } => {
            let expr = substitute(*expr, substitution);
            StateFrm::Modality {
                operator,
                formula,
                expr: Box::new(expr),
            }
        }
        StateFrm::Quantifier {
            quantifier,
            variables,
            body,
        } => {
            let new_body = substitute(*body, substitution);
            StateFrm::Quantifier {
                quantifier,
                variables,
                body: Box::new(new_body),
            }
        }
        StateFrm::DataValExprRightMult(expr, data_val) => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::DataValExprRightMult(Box::new(new_expr), data_val)
        }
        StateFrm::DataValExprLeftMult(data_val, expr) => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::DataValExprLeftMult(data_val, Box::new(new_expr))
        }
        StateFrm::Unary { op, expr } => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::Unary {
                op,
                expr: Box::new(new_expr),
            }
        }
        StateFrm::Id(_, _)
        | StateFrm::True
        | StateFrm::False
        | StateFrm::Delay(_)
        | StateFrm::Yaled(_)
        | StateFrm::DataValExpr(_) => formula,
    }
}