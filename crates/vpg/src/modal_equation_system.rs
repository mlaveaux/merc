use std::fmt;

use merc_syntax::FixedPointOperator;
use merc_syntax::StateFrm;
use merc_syntax::StateVarDecl;
use merc_syntax::apply;
use merc_syntax::visit;

/// A fixpoint equation system representing a ranked set of fixpoint equations.
pub struct ModalEquationSystem {
    equations: Vec<Equation>,
}

/// A single fixpoint equation of the shape `{mu, nu} X(args...) = rhs`.
struct Equation {
    operator: FixedPointOperator,
    variable: StateVarDecl,
    rhs: StateFrm,
}

impl ModalEquationSystem {
    /// Converts a plain state formula into a fixpoint equation system.
    pub fn new(formula: &StateFrm) -> Self {
        let mut equations = Vec::new();

        visit(formula, |formula| match formula {
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

        ModalEquationSystem { equations }
    }

    /// Returns the alternation depth of the fixpoint equation system.
    pub fn alternation_depth(&self) -> usize {
        if self.equations.is_empty() {
            return 0;
        }
        
        let first = &self.equations[0];
        self.alternation_depth_rec(&first.rhs, first.operator, &first.variable)
    }

    /// Returns the alternation depth of the given state formula.
    ///
    /// `current_op` is the operator of the current equation.
    /// `current_var` is the variable declaration of the current equation.
    ///
    /// # Details
    ///
    /// This implements the following function:
    ///   - AD(X) = AD of equation for X (if found), with alternation if operator changes
    ///   - AD(μ X. Ψ) = AD(Ψ) + 1 if there is a change in operator.
    ///   - AD(ν X. Ψ) = AD(Ψ) + 1 if there is a change in operator.
    ///   - AD(Ψ_1 op Ψ_2) = max(AD(Ψ_1), AD(Ψ_2))
    ///   - AD([a] Ψ) = AD(Ψ)
    ///   - AD(<a> Ψ) = AD(Ψ)
    fn alternation_depth_rec(&self, formula: &StateFrm, current_op: FixedPointOperator, current_var: &StateVarDecl) -> usize {
        match formula {
            StateFrm::Id(id, _) => {
                // Check if this is a recursive reference to the current variable
                if id == &current_var.identifier {
                    return 1;
                }
                
                // Find the equation corresponding to this variable and continue recursion
                if let Some(equation) = self.equations.iter().find(|eq| &eq.variable.identifier == id) {
                    let depth = self.alternation_depth_rec(&equation.rhs, equation.operator, &equation.variable);
                    if depth > 0 {
                        (if equation.operator != current_op { 1 } else { 0 }) + depth
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            StateFrm::FixedPoint {
                operator,
                variable,
                body,
            } => {
                if variable.identifier == current_var.identifier {
                    // Do not count inner fixed-point variables with the same name
                    return 0;
                }

                let depth = self.alternation_depth_rec(body, *operator, variable);
                if depth > 0 {
                    (if *operator != current_op { 1 } else { 0 }) + depth
                } else {
                    0
                }
            }
            StateFrm::Binary { lhs, rhs, .. } => {
                self.alternation_depth_rec(lhs, current_op, current_var)
                    .max(self.alternation_depth_rec(rhs, current_op, current_var))
            }
            StateFrm::Modality { expr, .. } => self.alternation_depth_rec(expr, current_op, current_var),
            _ => 0,
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
    apply(formula.clone(), |formula| match formula {
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
    }
}
