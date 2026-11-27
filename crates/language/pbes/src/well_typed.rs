use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt;
use std::ops::ControlFlow;

use merc_aterm::ATerm;
use merc_aterm::ATermRef;
use merc_aterm::Symb;
use merc_aterm::Term;
use merc_data::BinderType;
use merc_data::DataAbstractionRef;
use merc_data::DataApplicationRef;
use merc_data::DataExpressionRef;
use merc_data::DataExpressionVisitor;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::DataWhereClauseRef;
use merc_data::SortArrow;
use merc_data::SortExpression;
use merc_data::is_data_application;
use merc_data::is_data_binder;
use merc_data::is_data_function_symbol;
use merc_data::is_data_variable;
use merc_data::is_data_where_clause;
use merc_data::is_function_sort;
use merc_utilities::Step;
use merc_utilities::Visit;

use crate::Pbes;
use crate::PbesExpressionRef;
use crate::PbesKind;
use crate::PropositionalVariable;
use crate::PropositionalVariableInstantiationRef;
use crate::try_visit_pbes_expr;

/// Where in a PBES a [PbesWellTypedError] was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbesLocation {
    /// The equation of the propositional variable with this name.
    Equation(String),
    /// The initial state.
    Init,
}

impl fmt::Display for PbesLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PbesLocation::Equation(name) => write!(f, "the equation of '{name}'"),
            PbesLocation::Init => write!(f, "the initial state"),
        }
    }
}

/// A violation of one of the well-typedness conditions of a [Pbes].
///
/// `#[non_exhaustive]`, since the conditions that need the data specification, such as sorts being
/// declared, may be added later.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PbesWellTypedError {
    #[error("the propositional variable '{name}' is declared more than once")]
    DuplicateEquation { name: String },
    #[error("the parameter name '{parameter}' occurs more than once in {location}")]
    DuplicateParameter { location: PbesLocation, parameter: String },
    #[error("the global variable name '{name}' is declared more than once")]
    DuplicateGlobalVariable { name: String },
    #[error("the quantifier variable '{variable}' in {location} has the same name as a parameter")]
    QuantifierShadowsParameter { location: PbesLocation, variable: String },
    #[error(
        "the quantifier variable '{variable}' in {location} occurs within the scope of a quantifier variable with the same name"
    )]
    QuantifierShadowsQuantifier { location: PbesLocation, variable: String },
    #[error("the quantifier variable '{variable}' in {location} is also a global variable")]
    QuantifierShadowsGlobal { location: PbesLocation, variable: String },
    #[error("the free variable '{variable}' in {location} is not declared as a global variable")]
    UndeclaredFreeVariable { location: PbesLocation, variable: String },
    #[error("{location} refers to the undeclared propositional variable '{name}'")]
    UndeclaredPropositionalVariable { location: PbesLocation, name: String },
    #[error("the instantiation '{instantiation}' in {location} does not match the declaration '{declaration}'")]
    InstantiationMismatch {
        location: PbesLocation,
        instantiation: String,
        declaration: String,
    },
}

impl Pbes {
    /// Checks the well-typedness conditions.
    ///
    /// - the names of the equations, of the parameters of each equation, and of the global
    ///   variables are unique;
    /// - a quantifier variable has a different name than the parameters of its equation and than
    ///   the variables of the quantifiers it occurs in, and is not a global variable;
    /// - every free variable is a global variable;
    /// - every instantiation, including the initial state, refers to a declared propositional
    ///   variable, with as many arguments as it has parameters and of the same sorts.
    ///
    /// Sorts are compared as terms, so they must be normalised; mCRL2 compares them modulo aliases.
    /// Unlike mCRL2, an instantiation of an undeclared propositional variable is rejected too.
    pub fn check_well_typed(&self) -> Result<(), PbesWellTypedError> {
        let mut equations = HashMap::new();
        for equation in &self.equations {
            let name = equation.variable.name().value().to_string();
            if equations.insert(name.clone(), &equation.variable).is_some() {
                return Err(PbesWellTypedError::DuplicateEquation { name });
            }
        }

        if let Some(name) = first_duplicate_name(&self.global_variables) {
            return Err(PbesWellTypedError::DuplicateGlobalVariable { name });
        }

        let checker = Checker {
            equations: &equations,
            global_variables: &self.global_variables,
        };
        for equation in &self.equations {
            let location = PbesLocation::Equation(equation.variable.name().value().to_string());
            let parameters = equation.variable.parameters().to_vec();
            if let Some(parameter) = first_duplicate_name(&parameters) {
                return Err(PbesWellTypedError::DuplicateParameter { location, parameter });
            }

            checker.check_expression(&location, parameters, &equation.formula.copy())?;
        }

        checker.check_expression(
            &PbesLocation::Init,
            Vec::new(),
            &PbesExpressionRef::from(Term::copy(&self.initial_state)),
        )
    }
}

/// The variables bound at some point of a traversal: the parameters of the equation at the root,
/// then one scope per enclosing binder.
struct Scope {
    parent: Option<usize>,
    variables: Vec<DataVariable>,
    /// Whether the variables are bound by a PBES quantifier, rather than by the equation or by a
    /// binder or where clause inside a data expression.
    quantifier: bool,
}

/// The scopes of one traversal, which refer to their parent by index.
struct Scopes(Vec<Scope>);

impl Scopes {
    fn push(&mut self, parent: usize, variables: Vec<DataVariable>, quantifier: bool) -> usize {
        self.0.push(Scope {
            parent: Some(parent),
            variables,
            quantifier,
        });
        self.0.len() - 1
    }

    /// The scopes visible from `scope`, innermost first.
    fn chain(&self, scope: usize) -> impl Iterator<Item = &Scope> {
        std::iter::successors(Some(&self.0[scope]), |scope| scope.parent.map(|parent| &self.0[parent]))
    }

    fn binds(&self, scope: usize, variable: &DataVariableRef<'_>) -> bool {
        self.chain(scope)
            .any(|scope| scope.variables.iter().any(|bound| bound.copy() == *variable))
    }
}

struct Checker<'p> {
    equations: &'p HashMap<String, &'p PropositionalVariable>,
    global_variables: &'p [DataVariable],
}

impl Checker<'_> {
    /// Checks `expr`, the formula of an equation with the given `parameters` or the initial state.
    fn check_expression<'a>(
        &self,
        location: &PbesLocation,
        parameters: Vec<DataVariable>,
        expr: &PbesExpressionRef<'a>,
    ) -> Result<(), PbesWellTypedError> {
        let mut data = FreeVariableChecker {
            global_variables: self.global_variables,
            location,
            scopes: Scopes(vec![Scope {
                parent: None,
                variables: parameters,
                quantifier: false,
            }]),
            seen: HashSet::new(),
        };

        // The result of checking a subexpression depends on the variables in scope, so a shared
        // subexpression is skipped only when it is reached again within the same scope. The data
        // expressions keep their own record, since a data expression is also a PBES expression.
        let mut seen: HashSet<(PbesExpressionRef<'a>, usize)> = HashSet::new();
        let result: Option<Infallible> = try_visit_pbes_expr(expr, 0, |expr, scope| {
            if !seen.insert((expr.copy(), scope)) {
                return Ok(ControlFlow::Continue(Step::Prune));
            }

            match expr.kind() {
                PbesKind::Data(expr) => {
                    data.try_visit(&expr, scope)?;
                }
                PbesKind::Instantiation(instantiation) => {
                    self.check_instantiation(location, &instantiation)?;
                    for argument in list_elements(Term::copy(&instantiation).arg(1)) {
                        data.try_visit(&DataExpressionRef::from(argument), scope)?;
                    }
                }
                PbesKind::Forall(variables, _) | PbesKind::Exists(variables, _) => {
                    let variables = variables.to_vec();
                    self.check_quantifier(location, &data.scopes, scope, &variables)?;
                    return Ok(ControlFlow::Continue(Step::Into(
                        data.scopes.push(scope, variables, true),
                    )));
                }
                _ => {}
            }

            Ok(ControlFlow::Continue(Step::Into(scope)))
        })?;

        match result {
            None => Ok(()),
            Some(never) => match never {},
        }
    }

    /// Checks that `instantiation` refers to a declared propositional variable, with as many
    /// arguments as it has parameters and of the same sorts.
    fn check_instantiation(
        &self,
        location: &PbesLocation,
        instantiation: &PropositionalVariableInstantiationRef<'_>,
    ) -> Result<(), PbesWellTypedError> {
        let name = instantiation.name().value().to_string();
        let Some(declaration) = self.equations.get(&name) else {
            return Err(PbesWellTypedError::UndeclaredPropositionalVariable {
                location: location.clone(),
                name,
            });
        };

        let arguments = list_elements(Term::copy(instantiation).arg(1));
        let parameters = declaration.parameters().to_vec();
        let matches = arguments.len() == parameters.len()
            && arguments.iter().zip(parameters.iter()).all(|(argument, parameter)| {
                data_expression_sort(&DataExpressionRef::from(argument.copy()))
                    .is_none_or(|sort| sort.copy() == parameter.sort())
            });
        if !matches {
            return Err(PbesWellTypedError::InstantiationMismatch {
                location: location.clone(),
                instantiation: instantiation.to_string(),
                declaration: declaration.to_string(),
            });
        }

        Ok(())
    }

    /// Checks the variables of a quantifier occurring in `scope` against the parameters, the
    /// enclosing quantifiers and the global variables.
    fn check_quantifier(
        &self,
        location: &PbesLocation,
        scopes: &Scopes,
        scope: usize,
        variables: &[DataVariable],
    ) -> Result<(), PbesWellTypedError> {
        for (index, variable) in variables.iter().enumerate() {
            let error = |make: fn(PbesLocation, String) -> PbesWellTypedError| {
                Err(make(location.clone(), variable.name().to_string()))
            };
            let same_name = |other: &DataVariable| other.name() == variable.name();

            let mut chain = scopes.chain(scope);
            if chain.any(|scope| scope.parent.is_none() && scope.variables.iter().any(same_name)) {
                return error(|location, variable| PbesWellTypedError::QuantifierShadowsParameter {
                    location,
                    variable,
                });
            }
            let shadows_quantifier = scopes
                .chain(scope)
                .any(|scope| scope.quantifier && scope.variables.iter().any(same_name));
            if shadows_quantifier || variables[..index].iter().any(same_name) {
                return error(|location, variable| PbesWellTypedError::QuantifierShadowsQuantifier {
                    location,
                    variable,
                });
            }
            if self.global_variables.contains(variable) {
                return error(|location, variable| PbesWellTypedError::QuantifierShadowsGlobal { location, variable });
            }
        }

        Ok(())
    }
}

/// Checks that every free variable of the data expressions it visits is a global variable. The
/// context is the scope in which a data expression occurs.
struct FreeVariableChecker<'c, 'a> {
    global_variables: &'c [DataVariable],
    location: &'c PbesLocation,
    scopes: Scopes,
    /// The (term, scope) pairs that have been visited, since the result only depends on those.
    seen: HashSet<(ATermRef<'a>, usize)>,
}

impl<'a> DataExpressionVisitor<'a> for FreeVariableChecker<'_, 'a> {
    type Context = usize;
    type Break = Infallible;
    type Error = PbesWellTypedError;

    fn visit_variable(
        &mut self,
        variable: &DataVariableRef<'a>,
        scope: usize,
    ) -> Visit<Infallible, usize, Infallible, PbesWellTypedError> {
        if self.seen.insert((Term::copy(variable), scope))
            && !self.scopes.binds(scope, variable)
            && !self.global_variables.iter().any(|global| global.copy() == *variable)
        {
            return Err(PbesWellTypedError::UndeclaredFreeVariable {
                location: self.location.clone(),
                variable: variable.to_string(),
            });
        }

        Ok(ControlFlow::Continue(Step::Into(scope)))
    }

    fn visit_application(
        &mut self,
        application: &DataApplicationRef<'a>,
        scope: usize,
    ) -> Visit<Infallible, usize, Infallible, PbesWellTypedError> {
        Ok(ControlFlow::Continue(self.step(Term::copy(application), scope)))
    }

    fn visit_binder(
        &mut self,
        binder: &DataAbstractionRef<'a>,
        scope: usize,
    ) -> Visit<Infallible, usize, Infallible, PbesWellTypedError> {
        let step = match self.step(Term::copy(binder), scope) {
            Step::Into(scope) => Step::Into(self.scopes.push(scope, binder.variables().to_vec(), false)),
            step => step,
        };
        Ok(ControlFlow::Continue(step))
    }

    fn visit_where_clause(
        &mut self,
        clause: &DataWhereClauseRef<'a>,
        scope: usize,
    ) -> Visit<Infallible, usize, Infallible, PbesWellTypedError> {
        if let Step::Into(scope) = self.step(Term::copy(clause), scope) {
            // The traversal would visit the values of the declarations and the body in the same
            // scope, but only the body is in the scope of the declared variables, so both are
            // visited here instead.
            let declarations = list_elements(Term::copy(clause).arg(1));
            for declaration in &declarations {
                self.try_visit(&DataExpressionRef::from(declaration.arg(1)), scope)?;
            }

            let variables = declarations
                .iter()
                .map(|declaration| DataVariable::from(declaration.arg(0).protect()))
                .collect();
            let inner = self.scopes.push(scope, variables, false);
            self.try_visit(&DataExpressionRef::from(Term::copy(clause).arg(0)), inner)?;
        }

        Ok(ControlFlow::Continue(Step::Prune))
    }
}

impl<'a> FreeVariableChecker<'_, 'a> {
    /// Descends into `term` in `scope`, unless it has been visited there before.
    fn step(&mut self, term: ATermRef<'a>, scope: usize) -> Step<Infallible, usize> {
        if self.seen.insert((term, scope)) {
            Step::Into(scope)
        } else {
            Step::Prune
        }
    }
}

/// The elements of the list term `list`, borrowed from it.
fn list_elements(mut list: ATermRef<'_>) -> Vec<ATermRef<'_>> {
    let mut elements = Vec::new();
    while list.get_head_symbol().arity() == 2 {
        elements.push(list.arg(0));
        list = list.arg(1);
    }
    elements
}

/// The first name that occurs more than once among `variables`.
fn first_duplicate_name(variables: &[DataVariable]) -> Option<String> {
    let mut names = HashSet::new();
    variables
        .iter()
        .find(|variable| !names.insert(variable.name()))
        .map(|variable| variable.name().to_string())
}

/// The sort of `expr`, or `None` if it cannot be determined from the term alone, as for machine
/// numbers and set or bag comprehensions.
fn data_expression_sort(expr: &DataExpressionRef<'_>) -> Option<SortExpression> {
    if is_data_variable(expr) || is_data_function_symbol(expr) {
        Some(expr.data_sort())
    } else if is_data_application(expr) {
        let head = data_expression_sort(&DataExpressionRef::from(Term::copy(expr).arg(0)))?;
        is_function_sort(&head).then(|| SortArrow::from(ATerm::from(head)).codomain().protect())
    } else if is_data_binder(expr) {
        let binder = DataAbstractionRef::from(Term::copy(expr));
        match binder.binder_type() {
            BinderType::Forall | BinderType::Exists => Some(SortExpression::bool()),
            BinderType::Lambda => {
                let domain: Vec<SortExpression> = binder
                    .variables()
                    .iter()
                    .map(|variable| variable.sort().protect())
                    .collect();
                let codomain = data_expression_sort(&binder.body())?;
                Some(SortArrow::new(&domain, &codomain).into())
            }
            BinderType::SetComp | BinderType::BagComp => None,
        }
    } else if is_data_where_clause(expr) {
        data_expression_sort(&DataExpressionRef::from(Term::copy(expr).arg(0)))
    } else {
        None
    }
}
