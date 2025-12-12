use std::fmt;

use merc_syntax::FixedPointOperator;
use merc_syntax::StateFrm;
use merc_syntax::StateVarDecl;
use merc_syntax::apply;
use merc_syntax::visit;

/// A fixpoint equation system representing a ranked set of fixpoint equations.
pub struct FixpointEquationSystem {
    equations: Vec<Equation>,
}

/// A single fixpoint equation of the shape `{mu, nu} X(args...) = rhs`.
struct Equation {
    operator: FixedPointOperator,
    variable: StateVarDecl,
    rhs: StateFrm,
}


impl FixpointEquationSystem {
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

        FixpointEquationSystem { equations }
    }
}

// RHS(true) = true
// RHS(false) = false
// RHS(<a>f) = <a>RHS(f)
// RHS([a]f) = [a]RHS(f)
// RHS(f1 && f2) = RHS(f1) && RHS(f2)
// RHS(f1 || f2) = RHS(f1) || RHS(f2)
// RHS(X) = X
// RHS(mu X. f) = X(args)
// RHS(nu X. f) = X(args)
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

impl fmt::Display for FixpointEquationSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for equation in &self.equations {
            writeln!(
                f,
                "{} {} = {}",
                equation.operator, equation.variable, equation.rhs
            )?;
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
        let fes = FixpointEquationSystem::new(&formula);

        println!("{}", fes);
    }
}
