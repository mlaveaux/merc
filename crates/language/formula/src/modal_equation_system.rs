use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt;
use std::ops::ControlFlow;

use log::debug;
use merc_syntax::FixedPointOperator;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmKind;
use merc_syntax::StateVarDecl;
use merc_syntax::StateVarId;
use merc_syntax::StateVarIdAllocator;
use merc_syntax::Traverse;
use merc_syntax::fresh_state_var_generator;
use merc_syntax::respan;
use merc_utilities::FreshNameGenerator;
use merc_utilities::Span;

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

impl From<Equation> for StateFrm {
    fn from(val: Equation) -> Self {
        StateFrmKind::FixedPoint {
            operator: val.operator,
            variable: val.variable,
            body: Box::new(val.rhs),
        }
        .into()
    }
}

impl ModalEquationSystem {
    /// Converts a plain state formula into a fixpoint equation system.
    ///
    /// `formula` must already be resolved (see `resolve_modal_variables`).
    pub fn new(formula: &StateFrm, state_var_ids: &mut StateVarIdAllocator) -> Self {
        let mut equations = Vec::new();
        let mut identifier_generator = fresh_state_var_generator(formula);

        // Ensure that the formula has an outermost fixpoint operator.
        let formula = add_placeholder_operator(formula.clone(), &mut identifier_generator, state_var_ids);

        // Apply E to extract all equations from the formula
        apply_e(&mut equations, &formula);

        // Check that there are no duplicate variable ids — resolution already guarantees this, so
        // a violation here means `formula` wasn't actually resolved before being passed in.
        debug_assert!(
            {
                let ids: HashSet<StateVarId> = equations
                    .iter()
                    .map(|eq| eq.variable.id.expect("ModalEquationSystem requires a resolved formula"))
                    .collect();
                ids.len() == equations.len()
            },
            "Duplicate fixpoint-variable ids found in fixpoint equation system"
        );

        debug_assert!(
            !equations.is_empty(),
            "At least one fixpoint equation expected in the equation system"
        );

        ModalEquationSystem { equations }
    }

    /// Returns the ith equation in the system.
    pub fn equation(&self, i: usize) -> &Equation {
        &self.equations[i]
    }

    /// Returns an iterator over the equations in the system, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Equation> {
        self.equations.iter()
    }

    /// Returns the number of equations in the system.
    pub fn len(&self) -> usize {
        self.equations.len()
    }

    /// Returns true if the system contains no equations.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.equations.is_empty()
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
        let target_id = equation
            .variable()
            .id
            .expect("ModalEquationSystem requires a resolved formula");
        self.alternation_depth_rec(i, equation.body(), target_id)
    }

    /// Finds an equation by its variable's [`StateVarId`].
    ///
    /// # Details
    ///
    /// This is a linear scan over the equations, and is also called from within
    /// [`Self::alternation_depth`]'s recursion. Equation systems correspond to a
    /// single (modal) formula and are therefore small, so an index map is not
    /// worth its maintenance cost; revisit if very large formulas become common.
    pub fn find_equation_by_id(&self, id: StateVarId) -> Option<(usize, &Equation)> {
        self.equations
            .iter()
            .enumerate()
            .find(|(_, eq)| eq.variable.id == Some(id))
    }

    /// Recursive helper function to compute the alternation depth of equation `i`.
    ///
    /// # Details
    ///
    /// The depth of a formula is the largest depth of the variables occurring in it, so the
    /// traversal only has to look at the [StateFrmKind::Resolved] leaves. A variable bound by a
    /// later equation continues the chain in that equation's body, which is a different formula
    /// and therefore a nested traversal.
    fn alternation_depth_rec(&self, i: usize, formula: &StateFrm, target_id: StateVarId) -> usize {
        let equation = &self.equations[i];
        let mut depth = 0;

        formula.visit::<(), _>(|formula| {
            match &formula.node {
                StateFrmKind::Resolved(_, _, id) => {
                    depth = depth.max(if *id == target_id {
                        1
                    } else {
                        let (j, inner_equation) = self
                            .find_equation_by_id(*id)
                            .expect("Equation not found for identifier");

                        if j > i {
                            self.alternation_depth_rec(j, &inner_equation.rhs, target_id)
                                + usize::from(inner_equation.operator != equation.operator)
                        } else {
                            // Only consider nested equations
                            0
                        }
                    });
                }
                StateFrmKind::Binary { .. }
                | StateFrmKind::Modality { .. }
                | StateFrmKind::True
                | StateFrmKind::False => {}
                _ => {
                    unimplemented!("Cannot determine alternation depth of formula {}", formula)
                }
            }

            ControlFlow::Continue(())
        });

        depth
    }
}

/// If the given formula has no outermost fixpoint operator, adds a placeholder
/// fixpoint operator around it.
fn add_placeholder_operator(
    formula: StateFrm,
    identifier_generator: &mut FreshNameGenerator,
    state_var_ids: &mut StateVarIdAllocator,
) -> StateFrm {
    if matches!(formula.node, StateFrmKind::FixedPoint { .. }) {
        // The outer operator is already a fixpoint
        formula
    } else {
        // Introduce a placeholder.
        let mut variable = StateVarDecl::new(respan(Span::default(), identifier_generator.generate("X")), Vec::new());
        variable.id = Some(state_var_ids.alloc());
        StateFrmKind::FixedPoint {
            operator: FixedPointOperator::Least,
            variable,
            body: Box::new(formula),
        }
        .into()
    }
}

/// Applies `E` to the given formula, adding equations to the given vector.
///
/// E(nu X. f) = (nu X = RHS(f)) + E(f)
/// E(mu X. f) = (mu X = RHS(f)) + E(f)
/// E(g) = ... (traverse all the subformulas of g and apply E to them)
fn apply_e(equations: &mut Vec<Equation>, formula: &StateFrm) {
    debug!("Applying E to formula: {}", formula);

    formula.visit::<(), _>(|formula| {
        if let StateFrmKind::FixedPoint {
            operator,
            variable,
            body,
        } = &formula.node
        {
            debug!("Adding equation for variable {}", variable.identifier.node);
            // Add the equation with the renamed variable (the span is the same as the original variable).
            equations.push(Equation {
                operator: *operator,
                variable: variable.clone(),
                rhs: rhs(body),
            });
        }

        ControlFlow::Continue(())
    });
}

/// Applies `RHS` to the given formula.
///
/// ```text
/// RHS(true) = true
/// RHS(false) = false
/// RHS(<a>f) = <a>RHS(f)
/// RHS([a]f) = [a]RHS(f)
/// RHS(f1 && f2) = RHS(f1) && RHS(f2)
/// RHS(f1 || f2) = RHS(f1) || RHS(f2)
/// RHS(X) = X
/// RHS(mu X. f) = X(args)
/// RHS(nu X. f) = X(args)
/// ```
fn rhs(formula: &StateFrm) -> StateFrm {
    let result = formula.clone().apply::<Infallible, _>(|formula| match &formula.node {
        // RHS(mu X. phi) = X(args)
        StateFrmKind::FixedPoint { variable, .. } => Ok(Some(
            StateFrmKind::Resolved(
                variable.identifier.clone(),
                variable.arguments.iter().map(|arg| arg.expr.clone()).collect(),
                variable.id.expect("ModalEquationSystem requires a resolved formula"),
            )
            .into(),
        )),
        _ => Ok(None),
    });

    match result {
        Ok(formula) => formula,
        Err(error) => match error {},
    }
}

impl fmt::Display for ModalEquationSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, equation) in self.equations.iter().enumerate() {
            write!(f, "{i}: {} {} = {}", equation.operator, equation.variable, equation.rhs)?;
            if i + 1 < self.equations.len() {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}
