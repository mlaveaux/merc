use std::collections::HashSet;
use std::fmt;

use log::debug;

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

        // Apply E to extract all equations from the formula
        apply_e(&mut equations, formula);

        // Check that there are no duplicate variable names
        let identifiers: HashSet<&String> = HashSet::from_iter(equations.iter().map(|eq| &eq.variable.identifier));
        assert_eq!(
            identifiers.len(),
            equations.len(),
            "Duplicate variable names found in fixpoint equation system"
        );

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

    /// The alternation depth is a complexity measure of the given formula.
    ///
    /// # Details
    ///
    /// The alternation depth of mu X . psi is defined as the maximum chain X <= X_1 <= ... <= X_n,
    /// where X <= Y iff X appears freely in the corresponding equation sigma Y . phi. And furthermore,
    /// X_0, X_2, ... are bound by mu and X_1, X_3, ... are bound by nu. Similarly, for nu X . psi. Note
    /// that the alternation depth of a formula with a rhs is always 1, since the chain cannot be extended.
    pub fn alternation_depth(&self, i: usize) -> usize {
        let equation = &self.equations[i];
        self.alternation_depth_rec(i, equation.body(), equation.operator(), &equation.variable().identifier)
    }

    /// Finds an equation by its variable identifier.
    pub fn find_equation_by_identifier(&self, id: &str) -> Option<(usize, &Equation)> {
        self.equations
            .iter()
            .enumerate()
            .find(|(_, eq)| eq.variable.identifier == id)
    }

    /// Recursive helper function to compute the alternation depth of equation `i`.
    fn alternation_depth_rec(
        &self,
        i: usize,
        formula: &StateFrm,
        op: FixedPointOperator,
        identifier: &String,
    ) -> usize {
        let equation = &self.equations[i];

        match formula {
            StateFrm::Id(id, _) => {
                if id == identifier {
                    1
                } else {
                    let (j, inner_equation) = self
                        .find_equation_by_identifier(id)
                        .expect("Equation not found for identifier");
                    if j > i {
                        let depth =
                            self.alternation_depth_rec(j, &inner_equation.rhs, inner_equation.operator, identifier);
                        depth
                            + (if inner_equation.operator != equation.operator {
                                1 // Alternation occurs.
                            } else {
                                0
                            })
                    } else {
                        // Only consider nested equations
                        0
                    }
                }
            }
            StateFrm::Binary { lhs, rhs, .. } => self
                .alternation_depth_rec(i, lhs, op, identifier)
                .max(self.alternation_depth_rec(i, rhs, op, identifier)),
            StateFrm::Modality { expr, .. } => self.alternation_depth_rec(i, expr, op, identifier),
            StateFrm::True | StateFrm::False => 0,
            _ => {
                unimplemented!("Cannot determine alternation depth of formula {}", formula)
            }
        }
    }
}

// E(nu X. f) = (nu X = RHS(f)) + E(f)
// E(mu X. f) = (mu X = RHS(f)) + E(f)
// E(g) = ... (traverse all the subformulas of g and apply E to them)
fn apply_e(equations: &mut Vec<Equation>, formula: &StateFrm) {
    debug!("Applying E to formula: {}", formula);

    visit_statefrm(formula, |formula| match formula {
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            debug!("Adding equation for variable {}", variable.identifier);
            // Add the equation with the renamed variable (the span is the same as the original variable).
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
        for (i, equation) in self.equations.iter().enumerate() {
            writeln!(f, "{i}: {} {} = {}", equation.operator, equation.variable, equation.rhs)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_macros::merc_test;
    use merc_syntax::UntypedStateFrmSpec;

    use super::*;

    #[merc_test]
    fn test_fixpoint_equation_system_construction() {
        let formula = UntypedStateFrmSpec::parse("mu X. [a]X && nu Y. <b>true")
            .unwrap()
            .formula;
        let fes = ModalEquationSystem::new(&formula);

        println!("{}", fes);

        assert_eq!(fes.equations.len(), 2);
        assert_eq!(fes.alternation_depth(0), 0);
        assert_eq!(fes.alternation_depth(1), 0);
    }

    #[merc_test]
    fn test_fixpoint_equation_system_example() {
        let formula = UntypedStateFrmSpec::parse(include_str!("../../../examples/vpg/running_example.mcf"))
            .unwrap()
            .formula;
        let fes = ModalEquationSystem::new(&formula);

        println!("{}", fes);

        assert_eq!(fes.equations.len(), 2);
        assert_eq!(fes.alternation_depth(0), 2);
        assert_eq!(fes.alternation_depth(1), 0);
    }

    #[merc_test]
    fn test_fixpoint_equation_system_duplicates() {
        let formula = UntypedStateFrmSpec::parse("mu X. [a]X && (nu Y. <b>true) && (nu Y . <c>X)")
            .unwrap()
            .formula;
        let fes = ModalEquationSystem::new(&formula);

        println!("{}", fes);

        assert_eq!(fes.equations.len(), 3);
    }
}
