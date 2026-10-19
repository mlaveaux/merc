use std::convert::Infallible;
use std::marker::PhantomData;
use std::ops::ControlFlow;

use merc_aterm::Symb;
use merc_aterm::Term;
use merc_utilities::Step;
use merc_utilities::Visit;

use crate::DataAbstractionRef;
use crate::DataApplicationRef;
use crate::DataExpressionRef;
use crate::DataFunctionSymbolRef;
use crate::DataVariableRef;
use crate::DataWhereClauseRef;
use crate::MachineNumberRef;
use crate::is_data_application;
use crate::is_data_binder;
use crate::is_data_function_symbol;
use crate::is_data_machine_number;
use crate::is_data_variable;
use crate::is_data_where_clause;

/// A top-down traversal over data expressions that threads a context down the term.
///
/// There is one method per node kind, so a visitor only overrides the kinds it cares about and
/// the remaining ones keep descending. Every method returns a [Visit], so a traversal that has
/// found what it was looking for stops without walking the rest of the term, and subterms that
/// cannot contribute are skipped with [Step::Prune]. The context is passed from a node to its
/// children, which lets a visitor track where it is without maintaining a stack of its own.
///
/// Terms are maximally shared, so the same subterm is typically reachable along many paths. A
/// visitor whose work per node is not trivial should remember the indices of the nodes it has
/// seen and prune the repeats, which keeps the traversal linear in the size of the term graph
/// rather than of the tree it unfolds to.
///
/// The traversal only inspects terms; [Step::Replace] is uninhabited here because a rewritten
/// term has to be rebuilt bottom-up, which is what `TermBuilder` is for.
///
/// Binders and where clauses are traversed too. The variables they bind are not visited as
/// variables, only their occurrences in the body are, so a visitor that needs to tell bound
/// occurrences from free ones handles [DataExpressionVisitor::visit_binder] and
/// [DataExpressionVisitor::visit_where_clause] itself.
pub trait DataExpressionVisitor {
    /// Threaded from a node to its children.
    type Context: Copy;

    /// Produced when the traversal stops early.
    type Break;

    type Error;

    /// The returned [Step] is ignored, since a variable has no children.
    fn visit_variable(
        &mut self,
        _variable: &DataVariableRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The returned [Step] is ignored, since a function symbol has no children.
    fn visit_function_symbol(
        &mut self,
        _function_symbol: &DataFunctionSymbolRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The returned [Step] is ignored, since a machine number has no children.
    fn visit_machine_number(
        &mut self,
        _number: &MachineNumberRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The head function symbol is not visited separately; it is only reachable through the
    /// application that carries it.
    fn visit_application(
        &mut self,
        _application: &DataApplicationRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// A binder's only subexpression is its body.
    fn visit_binder(
        &mut self,
        _binder: &DataAbstractionRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The subexpressions of a where clause are the values of its declarations, followed by its
    /// body.
    fn visit_where_clause(
        &mut self,
        _clause: &DataWhereClauseRef<'_>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// Visits `expr` and, unless a visit method breaks or prunes, its subexpressions.
    ///
    /// This is the only place that recurses. It keeps its own worklist rather than using the call
    /// stack, so a term that is deeper than the stack can hold is still traversed.
    fn try_visit(
        &mut self,
        expr: &DataExpressionRef<'_>,
        context: Self::Context,
    ) -> Result<Option<Self::Break>, Self::Error> {
        let mut stack: Vec<(DataExpressionRef<'_>, Self::Context)> = vec![(expr.copy(), context)];

        while let Some((expr, context)) = stack.pop() {
            // Recognising an application is the most expensive test, so it is tried last.
            let outcome = if is_data_variable(&expr) {
                self.visit_variable(&DataVariableRef::from(Term::copy(&expr)), context)?
            } else if is_data_function_symbol(&expr) {
                self.visit_function_symbol(&DataFunctionSymbolRef::from(Term::copy(&expr)), context)?
            } else if is_data_machine_number(&expr) {
                self.visit_machine_number(&MachineNumberRef::from(Term::copy(&expr)), context)?
            } else if is_data_application(&expr) {
                let context = match self.visit_application(&DataApplicationRef::from(Term::copy(&expr)), context)? {
                    ControlFlow::Break(result) => return Ok(Some(result)),
                    ControlFlow::Continue(Step::Prune) => continue,
                    ControlFlow::Continue(Step::Replace(replacement)) => match replacement {},
                    ControlFlow::Continue(Step::Into(context)) => context,
                };

                // The arguments are pushed in reverse so that they are visited left to right.
                for index in (0..expr.data_arguments().len()).rev() {
                    stack.push((expr.data_arg(index), context));
                }

                continue;
            } else if is_data_binder(&expr) {
                let binder = DataAbstractionRef::from(Term::copy(&expr));
                match self.visit_binder(&binder, context)? {
                    ControlFlow::Break(result) => return Ok(Some(result)),
                    ControlFlow::Continue(Step::Prune) => {}
                    ControlFlow::Continue(Step::Replace(replacement)) => match replacement {},
                    ControlFlow::Continue(Step::Into(context)) => stack.push((expr.arg(2).into(), context)),
                }
                continue;
            } else if is_data_where_clause(&expr) {
                let clause = DataWhereClauseRef::from(Term::copy(&expr));
                match self.visit_where_clause(&clause, context)? {
                    ControlFlow::Break(result) => return Ok(Some(result)),
                    ControlFlow::Continue(Step::Prune) => {}
                    ControlFlow::Continue(Step::Replace(replacement)) => match replacement {},
                    ControlFlow::Continue(Step::Into(context)) => {
                        // Pushed in reverse, so the declarations are visited first and the body last.
                        stack.push((expr.arg(0).into(), context));
                        // The declarations form a cons list of `DataVarIdInit(variable, value)`.
                        let mut declarations = Vec::new();
                        let mut list = expr.arg(1);
                        while list.get_head_symbol().arity() == 2 {
                            declarations.push(list.arg(0).arg(1));
                            list = list.arg(1);
                        }
                        for value in declarations.into_iter().rev() {
                            stack.push((value.into(), context));
                        }
                    }
                }
                continue;
            } else {
                panic!("not a data expression: {expr}");
            };

            if let ControlFlow::Break(result) = outcome {
                return Ok(Some(result));
            }
        }

        Ok(None)
    }

    /// See [DataExpressionVisitor::try_visit], for visitors that cannot fail.
    fn visit(&mut self, expr: &DataExpressionRef<'_>, context: Self::Context) -> Option<Self::Break>
    where
        Self: DataExpressionVisitor<Error = Infallible>,
    {
        match self.try_visit(expr, context) {
            Ok(result) => result,
            Err(error) => match error {},
        }
    }
}

/// Adapts a closure into a [DataExpressionVisitor] that treats every node kind the same.
pub struct ClosureVisitor<C, T, E, F> {
    function: F,
    _marker: PhantomData<fn(C) -> (T, E)>,
}

impl<C, T, E, F> ClosureVisitor<C, T, E, F> {
    pub fn new(function: F) -> Self {
        ClosureVisitor {
            function,
            _marker: PhantomData,
        }
    }
}

impl<C, T, E, F> DataExpressionVisitor for ClosureVisitor<C, T, E, F>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'_>, C) -> Visit<Infallible, C, T, E>,
{
    type Context = C;
    type Break = T;
    type Error = E;

    fn visit_variable(&mut self, variable: &DataVariableRef<'_>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(variable)), context)
    }

    fn visit_function_symbol(
        &mut self,
        function_symbol: &DataFunctionSymbolRef<'_>,
        context: C,
    ) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(function_symbol)), context)
    }

    fn visit_machine_number(&mut self, number: &MachineNumberRef<'_>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(number)), context)
    }

    fn visit_application(&mut self, application: &DataApplicationRef<'_>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(application)), context)
    }

    fn visit_binder(&mut self, binder: &DataAbstractionRef<'_>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(binder)), context)
    }

    fn visit_where_clause(&mut self, clause: &DataWhereClauseRef<'_>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(clause)), context)
    }
}

/// Visits `expr` and its subexpressions top-down, calling `visitor` on every node.
///
/// See [DataExpressionVisitor] for the meaning of the context and the return value; implement
/// that trait directly when the node kinds need to be treated differently.
pub fn try_visit_data_expr<C, T, E, F>(expr: &DataExpressionRef<'_>, context: C, visitor: F) -> Result<Option<T>, E>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'_>, C) -> Visit<Infallible, C, T, E>,
{
    ClosureVisitor::<C, T, E, F>::new(visitor).try_visit(expr, context)
}

/// See [try_visit_data_expr], for visitors that cannot fail.
pub fn visit_data_expr<C, T, F>(expr: &DataExpressionRef<'_>, context: C, mut visitor: F) -> Option<T>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'_>, C) -> ControlFlow<T, Step<Infallible, C>>,
{
    match try_visit_data_expr::<C, T, Infallible, _>(expr, context, |expr, context| Ok(visitor(expr, context))) {
        Ok(result) => result,
        Err(error) => match error {},
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::ops::ControlFlow;

    use ahash::AHashSet;
    use merc_aterm::Term;
    use merc_utilities::Step;
    use merc_utilities::Visit;

    use crate::BasicSort;
    use crate::BinderType;
    use crate::DataAbstraction;
    use crate::DataApplicationRef;
    use crate::DataExpression;
    use crate::DataExpressionRef;
    use crate::DataFunctionSymbolRef;
    use crate::DataVariable;
    use crate::DataVariableRef;
    use crate::DataWhereClause;
    use crate::DataWhrDecl;
    use crate::SortExpression;
    use crate::is_data_variable;
    use crate::visitor::DataExpressionVisitor;
    use crate::visitor::visit_data_expr;

    /// Collects the name of every function symbol in the order in which it is visited.
    #[derive(Default)]
    struct SymbolNames {
        names: Vec<String>,
    }

    impl DataExpressionVisitor for SymbolNames {
        type Context = ();
        type Break = Infallible;
        type Error = Infallible;

        fn visit_function_symbol(
            &mut self,
            function_symbol: &DataFunctionSymbolRef<'_>,
            context: (),
        ) -> Visit<Infallible, (), Infallible, Infallible> {
            self.names.push(function_symbol.name().value().to_string());
            Ok(ControlFlow::Continue(Step::Into(context)))
        }

        fn visit_application(
            &mut self,
            application: &DataApplicationRef<'_>,
            context: (),
        ) -> Visit<Infallible, (), Infallible, Infallible> {
            self.names
                .push(application.data_function_symbol().name().value().to_string());
            Ok(ControlFlow::Continue(Step::Into(context)))
        }
    }

    #[test]
    fn test_visit_order_is_top_down() {
        let term = DataExpression::from_string("f(g(a), b)").unwrap();

        let mut visitor = SymbolNames::default();
        assert!(visitor.visit(&term.copy(), ()).is_none());
        assert_eq!(visitor.names, ["f", "g", "a", "b"]);
    }

    #[test]
    fn test_visit_breaks_early() {
        let variables = AHashSet::from_iter(["x".to_string()]);
        let term = DataExpression::from_string_untyped("f(g(x), h(x))", &variables).unwrap();

        // Counting the applications visited before the break shows that `h` is never reached.
        let mut visited = 0;
        let found = visit_data_expr(&term.copy(), (), |expr, context| {
            visited += 1;
            if crate::is_data_variable(expr) {
                ControlFlow::Break(DataVariableRef::from(Term::copy(expr)).protect())
            } else {
                ControlFlow::Continue(Step::Into(context))
            }
        });

        assert!(found.is_some_and(|variable| variable.name() == "x"));
        assert_eq!(visited, 3, "expected f, g and x to be visited");
    }

    #[test]
    fn test_visit_prunes_subterms() {
        let term = DataExpression::from_string("f(g(a), b)").unwrap();

        let mut visited = Vec::new();
        let result: Option<Infallible> = visit_data_expr(&term.copy(), (), |expr, _context| {
            visited.push(format!("{expr}"));

            // Everything below `g` is skipped, so `a` is never visited.
            if format!("{expr}").starts_with("g") {
                ControlFlow::Continue(Step::Prune)
            } else {
                ControlFlow::Continue(Step::Into(()))
            }
        });

        assert!(result.is_none());
        assert_eq!(visited, ["f(g(a), b)", "g(a)", "b"]);
    }

    #[test]
    fn test_visit_threads_context() {
        let term = DataExpression::from_string("f(g(a), b)").unwrap();

        // The context is the depth of the node, which is one more than its parent's.
        let mut depths = Vec::new();
        let result: Option<Infallible> = visit_data_expr(&term.copy(), 0, |expr: &DataExpressionRef<'_>, depth| {
            depths.push((format!("{expr}"), depth));
            ControlFlow::Continue(Step::Into(depth + 1))
        });

        assert!(result.is_none());
        assert_eq!(
            depths,
            [
                ("f(g(a), b)".to_string(), 0),
                ("g(a)".to_string(), 1),
                ("a".to_string(), 2),
                ("b".to_string(), 1)
            ]
        );
    }

    /// Binders and where clauses are traversed: their bodies and declared values are visited, the
    /// binding occurrences of their variables are not.
    #[test]
    fn test_visit_binder_and_where_clause() {
        let x = DataVariable::with_sort("x", SortExpression::from(BasicSort::new("Bool")).copy());
        let y = DataVariable::with_sort("y", SortExpression::from(BasicSort::new("Bool")).copy());
        let lambda: DataExpression =
            DataAbstraction::new(BinderType::Lambda, std::slice::from_ref(&x), y.clone().into()).into();
        let clause: DataExpression =
            DataWhereClause::new(lambda, &[DataWhrDecl::new(y.clone(), x.clone().into())]).into();

        let mut variables = Vec::new();
        let _: Option<()> = visit_data_expr(&clause.copy(), (), |expr, context| {
            if is_data_variable(expr) {
                variables.push(DataVariableRef::from(expr.copy()).name().to_string());
            }
            ControlFlow::Continue(Step::Into(context))
        });
        assert_eq!(variables, ["x", "y"]);
    }
}
