use std::collections::HashSet;
use std::fmt;

use merc_syntax::FixedPointOperator;
use merc_syntax::StateFrm;
use merc_syntax::StateVarDecl;
use merc_syntax::apply_statefrm;
use merc_syntax::visit_statefrm;

/// A fixpoint equation system representing a ranked set of fixpoint equations.
///
/// Each equation is of the shape `{mu, nu} X(args...) = rhs`. Where rhs
/// contains no further fixpoint equations.
pub struct ModalEquationSystem {
    equations: Vec<Equation>,
}

/// A single fixpoint equation of the shape `{mu, nu} X(args...) = rhs`.
#[derive(Clone)]
pub struct Equation {
    operator: FixedPointOperator,
    variable: StateVarDecl,
    rhs: StateFrm,
}

impl Equation {
    /// Returns the operator of the equation.
    pub fn operator(&self) -> FixedPointOperator {
        self.operator
    }

    /// Returns the variable declaration of the equation.
    pub fn variable(&self) -> &StateVarDecl {
        &self.variable
    }

    /// Returns the body of the equation.
    pub fn body(&self) -> &StateFrm {
        &self.rhs
    }
}

impl Into<StateFrm> for Equation {
    fn into(self) -> StateFrm {
        StateFrm::FixedPoint {
            operator: self.operator,
            variable: self.variable,
            body: Box::new(self.rhs),
        }
    }
}

impl ModalEquationSystem {
    /// Converts a plain state formula into a fixpoint equation system.
    pub fn new(formula: &StateFrm) -> Self {
        let mut equations = Vec::new();

        visit_statefrm(formula, |formula| match formula {
            // E(nu X. f) = (nu X = RHS(f)) + E(f)
            // E(mu X. f) = (mu X = RHS(f)) + E(f)
            // E(g) = epsilon, if g is not a fixpoint formula
            StateFrm::FixedPoint {
                operator,
                variable,
                body,
            } => {
                equations.push(Equation {
                    operator: operator.clone(),
                    variable: variable.clone(),
                    rhs: rhs(body),
                });

                Ok(())
            }
            _ => Ok(()),
        })
        .expect("No error expected during fixpoint equation system construction");

        // Check that there are no duplicate variable names
        if cfg!(debug_assertions) {
            let identifiers: HashSet<&String> = HashSet::from_iter(equations.iter().map(|eq| &eq.variable.identifier));
            debug_assert_eq!(
                identifiers.len(),
                equations.len(),
                "Duplicate variable names found in fixpoint equation system"
            );
        }

        debug_assert!(
            equations.len() > 0,
            "At least one fixpoint equation expected in the equation system"
        );

        ModalEquationSystem { equations }
    }

    /// Returns the ith equation in the system.
    pub fn equation(&self, i: usize) -> &Equation {
        &self.equations[i]
    }

    /// Returns the alternation depth of the ith equation
    pub fn alternation_depth(&self, i: usize) -> usize {
        let equation = &self.equations[i];
        self.alternation_depth_rec(i, equation.operator, &equation.variable.identifier)
    }

    /// Finds an equation by its variable identifier.
    pub fn find_equation_by_identifier(&self, id: &str) -> Option<(usize, &Equation)> {
        self.equations
            .iter()
            .enumerate()
            .find(|(_, eq)| eq.variable.identifier == id)
    }

    /// Returns the alternation depth of the given state formula.
    ///
    /// # Details
    ///
    /// Let `E` be the set of equations in the system and `X` be the variable coresponding to the
    /// equation sigma X = f.
    ///
    ///  AD(X) = CAD(sigma, X, E), which is inductively defined as:
    ///  - CAD(sigma, X, (sigma' Y)E') = 0, if sigma = sigma' and X = Y
    ///  - CAD(sigma, X, (sigma' Y)E') = CAD(sigma, X, E'), if sigma == sigma' and X != Y
    ///  - CAD(sigma, X, (sigma' Y)E') = 1 + CAD(sigma', Y, E'), if sigma != sigma'
    fn alternation_depth_rec(&self, i: usize, sigma: FixedPointOperator, variable: &String) -> usize {
        let equation = &self.equations[i];
        if sigma == equation.operator {
            if equation.variable.identifier == *variable {
                0
            } else {
                self.alternation_depth_rec(i + 1, sigma, variable)
            }
        } else {
            1 + self.alternation_depth_rec(i + 1, equation.operator, &equation.variable.identifier)
        }
    }
}

/// Applies `RHS` to the given formula.
///
/// RHS(true) = true
/// RHS(false) = false
/// RHS(<a>f) = <a>RHS(f)
/// RHS([a]f) = [a]RHS(f)
/// RHS(f1 && f2) = RHS(f1) && RHS(f2)
/// RHS(f1 || f2) = RHS(f1) || RHS(f2)
/// RHS(X) = X
/// RHS(mu X. f) = X(args)
/// RHS(nu X. f) = X(args)
fn rhs(formula: &StateFrm) -> StateFrm {
    apply_statefrm(formula.clone(), |formula| match formula {
        // RHS(mu X. phi) = X(args)
        StateFrm::FixedPoint { variable, .. } => Ok(Some(StateFrm::Id(
            variable.identifier.clone(),
            variable.arguments.iter().map(|arg| arg.expr.clone()).collect(),
        ))),
        _ => Ok(None),
    })
    .expect("No error expected during RHS extraction")
}

impl fmt::Display for ModalEquationSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for equation in &self.equations {
            writeln!(f, "{} {} = {}", equation.operator, equation.variable, equation.rhs)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::UntypedStateFrmSpec;

    use super::*;

    #[test]
    fn test_fixpoint_equation_system_construction() {
        let formula = UntypedStateFrmSpec::parse("mu X. [a]X && nu Y. <b>true")
            .unwrap()
            .formula;
        let fes = ModalEquationSystem::new(&formula);

        println!("{}", fes);

        assert_eq!(fes.equations.len(), 2);
        assert_eq!(fes.alternation_depth(0), 1);
        assert_eq!(fes.alternation_depth(1), 0);
    }

    // #[test]
    // fn test_fixpoint_equation_system_duplicates() {
    //     let formula = UntypedStateFrmSpec::parse("mu X. [a]X && nu Y. <b>true && nu Y . <c>X")
    //         .unwrap()
    //         .formula;
    //     let fes = ModalEquationSystem::new(&formula);

    //     println!("{}", fes);

    //     assert_eq!(fes.equations.len(), 2);
    // }
}
