use merc_utilities::MercError;

use crate::StateFrm;

/// Applies the given function recursively to the state formula.
///
/// The substitution function takes a state formula and returns an optional new
/// formula. If it returns `Some(new_formula)`, the substitution is applied and
/// the new formula is returned. If it returns `None`, the substitution is not
/// applied and the function continues to traverse the formula tree.
pub fn apply_statefrm(
    formula: StateFrm,
    mut function: impl FnMut(&StateFrm) -> Result<Option<StateFrm>, MercError>,
) -> Result<StateFrm, MercError> {
    apply_statefrm_rec(formula, &mut function)
}

/// Visits the state formula and calls the given function on each subformula.
///
/// The substitution function takes a state formula and returns an optional new
/// formula. If it returns `Some(new_formula)`, the substitution is applied and
/// the new formula is returned. If it returns `None`, the substitution is not
/// applied and the function continues to traverse the formula tree.
pub fn visit_statefrm(formula: &StateFrm, mut visitor: impl FnMut(&StateFrm) -> Result<(), MercError>) -> Result<(), MercError> {
    visit_statefrm_rec(formula, &mut visitor)
}

/// See [`apply`].
fn apply_statefrm_rec(
    formula: StateFrm,
    apply: &mut impl FnMut(&StateFrm) -> Result<Option<StateFrm>, MercError>,
) -> Result<StateFrm, MercError> {
    if let Some(formula) = apply(&formula)? {
        // A substitution was made, return the new formula.
        return Ok(formula);
    }

    match formula {
        StateFrm::Binary { op, lhs, rhs } => {
            let new_lhs = apply_statefrm_rec(*lhs, apply)?;
            let new_rhs = apply_statefrm_rec(*rhs, apply)?;
            Ok(StateFrm::Binary {
                op,
                lhs: Box::new(new_lhs),
                rhs: Box::new(new_rhs),
            })
        }
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            let new_body = apply_statefrm_rec(*body, apply)?;
            Ok(StateFrm::FixedPoint {
                operator,
                variable,
                body: Box::new(new_body),
            })
        }
        StateFrm::Bound { bound, variables, body } => {
            let new_body = apply_statefrm_rec(*body, apply)?;
            Ok(StateFrm::Bound {
                bound,
                variables,
                body: Box::new(new_body),
            })
        }
        StateFrm::Modality {
            operator,
            formula,
            expr,
        } => {
            let expr = apply_statefrm_rec(*expr, apply)?;
            Ok(StateFrm::Modality {
                operator,
                formula,
                expr: Box::new(expr),
            })
        }
        StateFrm::Quantifier {
            quantifier,
            variables,
            body,
        } => {
            let new_body = apply_statefrm_rec(*body, apply)?;
            Ok(StateFrm::Quantifier {
                quantifier,
                variables,
                body: Box::new(new_body),
            })
        }
        StateFrm::DataValExprRightMult(expr, data_val) => {
            let new_expr = apply_statefrm_rec(*expr, apply)?;
            Ok(StateFrm::DataValExprRightMult(Box::new(new_expr), data_val))
        }
        StateFrm::DataValExprLeftMult(data_val, expr) => {
            let new_expr = apply_statefrm_rec(*expr, apply)?;
            Ok(StateFrm::DataValExprLeftMult(data_val, Box::new(new_expr)))
        }
        StateFrm::Unary { op, expr } => {
            let new_expr = apply_statefrm_rec(*expr, apply)?;
            Ok(StateFrm::Unary {
                op,
                expr: Box::new(new_expr),
            })
        }
        StateFrm::Id(_, _)
        | StateFrm::True
        | StateFrm::False
        | StateFrm::Delay(_)
        | StateFrm::Yaled(_)
        | StateFrm::DataValExpr(_) => Ok(formula),
    }
}

/// See [`visit`].
fn visit_statefrm_rec(
    formula: &StateFrm,
    function: &mut impl FnMut(&StateFrm) -> Result<(), MercError>,
) -> Result<(), MercError> {
    function(&formula)?;

    match formula {
        StateFrm::Binary { lhs, rhs, .. } => {
            visit_statefrm_rec(lhs, function)?;
            visit_statefrm_rec(rhs, function)?;
        }
        StateFrm::FixedPoint { body, .. } => {
            visit_statefrm_rec(body, function)?;
        }
        StateFrm::Bound { body, .. } => {
            visit_statefrm_rec(body, function)?;
        }
        StateFrm::Modality { expr, .. } => {
            visit_statefrm_rec(expr, function)?;
        }
        StateFrm::Quantifier { body, .. } => {
            visit_statefrm_rec(body, function)?;
        }
        StateFrm::DataValExprRightMult(expr, _data_val) => {
            visit_statefrm_rec(expr, function)?;
        }
        StateFrm::DataValExprLeftMult(_data_val, expr) => {
            visit_statefrm_rec(expr, function)?;
        }
        StateFrm::Unary { expr, .. } => {
            visit_statefrm_rec(expr, function)?;
        }
        StateFrm::Id(_, _)
        | StateFrm::True
        | StateFrm::False
        | StateFrm::Delay(_)
        | StateFrm::Yaled(_)
        | StateFrm::DataValExpr(_) => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::vec;

    use crate::UntypedStateFrmSpec;

    use super::*;

    #[test]
    fn test_visit_state_frm_variables() {
        let input = UntypedStateFrmSpec::parse("mu X. [a]X && mu X. X && Y").unwrap();

        let mut variables = vec![];
        apply_statefrm(input.formula, |frm| {
            if let StateFrm::Id(name, _) = frm {
                variables.push(name.clone());
            }

            Ok(None)
        })
        .unwrap();

        assert_eq!(variables, vec!["X", "X", "Y"]);
    }
}
