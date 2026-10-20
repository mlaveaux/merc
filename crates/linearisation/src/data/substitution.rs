use std::cell::RefCell;
use std::collections::HashSet;
use std::convert::Infallible;
use std::ops::ControlFlow;

use ahash::HashMap;

use merc_aterm::ATerm;
use merc_aterm::Term;
use merc_aterm::TermBuilder;
use merc_aterm::Yield;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_data::DataAbstraction;
use merc_data::DataAbstractionRef;
use merc_data::DataApplicationRef;
use merc_data::DataExpression;
use merc_data::DataExpressionVisitor;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::DataWhereClause;
use merc_data::DataWhereClauseRef;
use merc_data::DataWhrDecl;
use merc_data::is_data_application;
use merc_data::is_data_binder;
use merc_data::is_data_variable;
use merc_data::is_data_where_clause;
use merc_utilities::Step;
use merc_utilities::Visit;

use crate::data::NameGenerator;

/// The free variables of `expr`, in order of first occurrence.
pub(crate) fn free_variables(expr: &DataExpression) -> Vec<DataVariable> {
    let mut visitor = FreeVariables::default();
    visitor.visit(&expr.copy(), ());
    visitor.result
}

/// The variables of `scope` that occur free in `exprs`, in the order of `scope`.
pub(crate) fn occurring<'a, I>(scope: &[DataVariable], exprs: I) -> Vec<DataVariable>
where
    I: IntoIterator<Item = &'a DataExpression>,
{
    let free: HashSet<DataVariable> = exprs.into_iter().flat_map(free_variables).collect();
    scope
        .iter()
        .filter(|variable| free.contains(*variable))
        .cloned()
        .collect()
}

/// Collects free variables; the variables of a binder are subtracted from those of its body.
#[derive(Default)]
struct FreeVariables {
    result: Vec<DataVariable>,
    seen: HashSet<DataVariable>,
    /// Terms are maximally shared, so subterms that were visited before are pruned.
    visited: HashSet<usize>,
}

impl FreeVariables {
    fn add(&mut self, variable: DataVariable) {
        if self.seen.insert(variable.clone()) {
            self.result.push(variable);
        }
    }

    /// Adds the free variables of `body` that are not among `bound`.
    fn add_scoped(&mut self, body: &DataExpression, bound: &[DataVariable]) {
        for variable in free_variables(body) {
            if !bound.contains(&variable) {
                self.add(variable);
            }
        }
    }
}

impl DataExpressionVisitor for FreeVariables {
    type Context = ();
    type Break = Infallible;
    type Error = Infallible;

    fn visit_variable(
        &mut self,
        variable: &DataVariableRef<'_>,
        _: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.add(variable.protect());
        Ok(ControlFlow::Continue(Step::Prune))
    }

    fn visit_application(
        &mut self,
        application: &DataApplicationRef<'_>,
        _: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        Ok(ControlFlow::Continue(if self.visited.insert(application.index()) {
            Step::Into(())
        } else {
            Step::Prune
        }))
    }

    fn visit_binder(
        &mut self,
        binder: &DataAbstractionRef<'_>,
        _: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        let binder = binder.protect();
        self.add_scoped(&binder.body().protect(), &binder.variables().to_vec());
        Ok(ControlFlow::Continue(Step::Prune))
    }

    fn visit_where_clause(
        &mut self,
        clause: &DataWhereClauseRef<'_>,
        _: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        let clause = clause.protect();
        let declarations = clause.declarations().to_vec();
        for declaration in &declarations {
            for variable in free_variables(&declaration.expression().protect()) {
                self.add(variable);
            }
        }
        let bound: Vec<DataVariable> = declarations
            .iter()
            .map(|declaration| declaration.variable().protect())
            .collect();
        self.add_scoped(&clause.body().protect(), &bound);
        Ok(ControlFlow::Continue(Step::Prune))
    }
}

/// A capture-avoiding substitution of expressions for variables.
///
/// Invariant: `avoid` contains at least the names of the free variables of every expression in the
/// range, so applying the substitution under a binder renames every binder that could capture one
/// of them; [Substitution::avoid] adds names whose binders must be renamed for other reasons.
#[derive(Clone, Debug, Default)]
pub(crate) struct Substitution {
    map: HashMap<DataVariable, DataExpression>,
    avoid: HashSet<String>,
}

impl Substitution {
    pub(crate) fn new<I: IntoIterator<Item = (DataVariable, DataExpression)>>(pairs: I) -> Self {
        let mut result = Substitution::default();
        for (variable, expr) in pairs {
            result.insert(variable, expr);
        }
        result
    }

    /// Maps `variable` to `expr`, replacing an earlier mapping.
    pub(crate) fn insert(&mut self, variable: DataVariable, expr: DataExpression) {
        self.avoid
            .extend(free_variables(&expr).iter().map(|variable| variable.name().to_string()));
        self.map.insert(variable, expr);
    }

    /// Makes binders named `name` be renamed when the substitution is applied below them, as if a
    /// variable with that name occurred in the range.
    pub(crate) fn avoid(&mut self, name: &str) {
        self.avoid.insert(name.to_string());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Enters binders of `variables`: binders that could capture are renamed, and the returned
    /// substitution maps them to their new names and no longer replaces the bound variables.
    pub(crate) fn bind(
        &self,
        variables: &[DataVariable],
        names: &mut NameGenerator,
    ) -> (Vec<DataVariable>, Substitution) {
        let mut inner = self.clone();
        let renamed = variables
            .iter()
            .map(|variable| {
                if self.avoid.contains(variable.name()) {
                    let fresh = names.variable(variable);
                    inner.insert(variable.clone(), fresh.clone().into());
                    fresh
                } else {
                    inner.map.remove(variable);
                    variable.clone()
                }
            })
            .collect();
        (renamed, inner)
    }

    pub(crate) fn apply(&self, expr: &DataExpression, names: &mut NameGenerator) -> DataExpression {
        if self.map.is_empty() {
            return expr.clone();
        }

        // Bottom-up with an explicit stack; a binder or where clause is substituted by a recursive
        // call on its body, so the recursion depth is the nesting depth of binders.
        let names = RefCell::new(names);
        let mut builder = TermBuilder::<ATerm, merc_aterm::Symbol>::new();
        let term: ATerm = expr.clone().into();
        THREAD_TERM_POOL
            .with(|tp| {
                builder.evaluate(
                    tp,
                    term,
                    |_tp, args, t| {
                        if is_data_variable(&t) {
                            let variable = DataVariable::from(t.clone());
                            return Ok(Yield::Term(match self.map.get(&variable) {
                                Some(expr) => expr.clone().into(),
                                None => t,
                            }));
                        }
                        if is_data_binder(&t) {
                            let binder = DataAbstraction::from(t);
                            let (variables, inner) = self.bind(&binder.variables().to_vec(), &mut names.borrow_mut());
                            let body = inner.apply(&binder.body().protect(), &mut names.borrow_mut());
                            return Ok(Yield::Term(
                                DataAbstraction::new(binder.binder_type(), &variables, body).into(),
                            ));
                        }
                        if is_data_where_clause(&t) {
                            let clause = DataWhereClause::from(t);
                            let declarations = clause.declarations().to_vec();
                            let bound: Vec<DataVariable> = declarations
                                .iter()
                                .map(|declaration| declaration.variable().protect())
                                .collect();
                            let (bound, inner) = self.bind(&bound, &mut names.borrow_mut());
                            let declarations: Vec<DataWhrDecl> = declarations
                                .iter()
                                .zip(bound)
                                .map(|(declaration, variable)| {
                                    let value =
                                        self.apply(&declaration.expression().protect(), &mut names.borrow_mut());
                                    DataWhrDecl::new(variable, value)
                                })
                                .collect();
                            let body = inner.apply(&clause.body().protect(), &mut names.borrow_mut());
                            return Ok(Yield::Term(DataWhereClause::new(body, &declarations).into()));
                        }
                        if !is_data_application(&t) {
                            return Ok(Yield::Term(t));
                        }
                        for argument in t.arguments() {
                            args.push(argument.protect());
                        }
                        Ok(Yield::Construct(t.get_head_symbol().protect()))
                    },
                    |tp, symbol, args| Ok(tp.create_term_iter(&symbol, args)),
                )
            })
            .expect("substitution never fails")
            .into()
    }

    pub(crate) fn apply_all(&self, exprs: &[DataExpression], names: &mut NameGenerator) -> Vec<DataExpression> {
        exprs.iter().map(|expr| self.apply(expr, names)).collect()
    }
}
