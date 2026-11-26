use std::convert::Infallible;
use std::marker::PhantomData;
use std::ops::ControlFlow;

use merc_aterm::Symb;
use merc_aterm::Term;
use merc_utilities::Step;
use merc_utilities::Visit;

use crate::DataAbstractionRef;
use crate::DataApplicationRef;
use crate::DataExpressionKind;
use crate::DataExpressionRef;
use crate::DataFunctionSymbolRef;
use crate::DataVariableRef;
use crate::DataWhereClauseRef;
use crate::MachineNumberRef;

/// A top-down traversal over data expressions that threads a context down the term.
///
/// There is one method per node kind, so a visitor only overrides the kinds it cares about and
/// the remaining ones keep descending. Every method returns a [Visit], so a traversal that has
/// found what it was looking for stops without walking the rest of the term, and subterms that
/// cannot contribute are skipped with [Step::Prune]. The context is passed from a node to its
/// children, which lets a visitor track where it is without maintaining a stack of its own.
///
/// Terms are maximally shared, so the same subterm is typically reachable along many paths. A
/// visitor whose work per node is not trivial should remember the nodes it has seen and prune the
/// repeats, which keeps the traversal linear in the size of the term graph rather than of the tree
/// it unfolds to. Every node is a subterm of the visited expression and is passed with its
/// lifetime `'a`, so it can be kept as such a [DataExpressionRef] for as long as the expression is
/// borrowed.
///
/// The traversal only inspects terms; [Step::Replace] is uninhabited here because a rewritten
/// term has to be rebuilt bottom-up, which is what `TermBuilder` is for.
///
/// Binders and where clauses are traversed too. The variables they bind are not visited as
/// variables, only their occurrences in the body are, so a visitor that needs to tell bound
/// occurrences from free ones handles [DataExpressionVisitor::visit_binder] and
/// [DataExpressionVisitor::visit_where_clause] itself.
pub trait DataExpressionVisitor<'a> {
    /// Threaded from a node to its children.
    type Context: Copy;

    /// Produced when the traversal stops early.
    type Break;

    type Error;

    /// The returned [Step] is ignored, since a variable has no children.
    fn visit_variable(
        &mut self,
        _variable: &DataVariableRef<'a>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The returned [Step] is ignored, since a function symbol has no children.
    fn visit_function_symbol(
        &mut self,
        _function_symbol: &DataFunctionSymbolRef<'a>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The returned [Step] is ignored, since a machine number has no children.
    fn visit_machine_number(
        &mut self,
        _number: &MachineNumberRef<'a>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The head function symbol is not visited separately; it is only reachable through the
    /// application that carries it.
    fn visit_application(
        &mut self,
        _application: &DataApplicationRef<'a>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// A binder's only subexpression is its body.
    fn visit_binder(
        &mut self,
        _binder: &DataAbstractionRef<'a>,
        context: Self::Context,
    ) -> Visit<Infallible, Self::Context, Self::Break, Self::Error> {
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    /// The subexpressions of a where clause are the values of its declarations, followed by its
    /// body.
    fn visit_where_clause(
        &mut self,
        _clause: &DataWhereClauseRef<'a>,
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
        expr: &DataExpressionRef<'a>,
        context: Self::Context,
    ) -> Result<Option<Self::Break>, Self::Error> {
        let mut stack: Vec<(DataExpressionRef<'a>, Self::Context)> = vec![(expr.copy(), context)];

        while let Some((expr, context)) = stack.pop() {
            let outcome = match expr.kind() {
                DataExpressionKind::Variable(variable) => self.visit_variable(&variable, context)?,
                DataExpressionKind::FunctionSymbol(symbol) => self.visit_function_symbol(&symbol, context)?,
                DataExpressionKind::MachineNumber(number) => self.visit_machine_number(&number, context)?,
                DataExpressionKind::Application(application) => {
                    let context = match self.visit_application(&application, context)? {
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
                }
                DataExpressionKind::Abstraction(binder) => {
                    match self.visit_binder(&binder, context)? {
                        ControlFlow::Break(result) => return Ok(Some(result)),
                        ControlFlow::Continue(Step::Prune) => {}
                        ControlFlow::Continue(Step::Replace(replacement)) => match replacement {},
                        ControlFlow::Continue(Step::Into(context)) => stack.push((expr.arg(2).into(), context)),
                    }
                    continue;
                }
                DataExpressionKind::WhereClause(clause) => {
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
                }
            };

            if let ControlFlow::Break(result) = outcome {
                return Ok(Some(result));
            }
        }

        Ok(None)
    }

    /// See [DataExpressionVisitor::try_visit], for visitors that cannot fail.
    fn visit(&mut self, expr: &DataExpressionRef<'a>, context: Self::Context) -> Option<Self::Break>
    where
        Self: DataExpressionVisitor<'a, Error = Infallible>,
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

impl<'a, C, T, E, F> DataExpressionVisitor<'a> for ClosureVisitor<C, T, E, F>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'a>, C) -> Visit<Infallible, C, T, E>,
{
    type Context = C;
    type Break = T;
    type Error = E;

    fn visit_variable(&mut self, variable: &DataVariableRef<'a>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(variable)), context)
    }

    fn visit_function_symbol(
        &mut self,
        function_symbol: &DataFunctionSymbolRef<'a>,
        context: C,
    ) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(function_symbol)), context)
    }

    fn visit_machine_number(&mut self, number: &MachineNumberRef<'a>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(number)), context)
    }

    fn visit_application(&mut self, application: &DataApplicationRef<'a>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(application)), context)
    }

    fn visit_binder(&mut self, binder: &DataAbstractionRef<'a>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(binder)), context)
    }

    fn visit_where_clause(&mut self, clause: &DataWhereClauseRef<'a>, context: C) -> Visit<Infallible, C, T, E> {
        (self.function)(&DataExpressionRef::from(Term::copy(clause)), context)
    }
}

/// Visits `expr` and its subexpressions top-down, calling `visitor` on every node.
///
/// See [DataExpressionVisitor] for the meaning of the context and the return value; implement
/// that trait directly when the node kinds need to be treated differently.
pub fn try_visit_data_expr<'a, C, T, E, F>(expr: &DataExpressionRef<'a>, context: C, visitor: F) -> Result<Option<T>, E>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'a>, C) -> Visit<Infallible, C, T, E>,
{
    DataExpressionVisitor::<'a>::try_visit(&mut ClosureVisitor::<C, T, E, F>::new(visitor), expr, context)
}

/// See [try_visit_data_expr], for visitors that cannot fail.
pub fn visit_data_expr<'a, C, T, F>(expr: &DataExpressionRef<'a>, context: C, mut visitor: F) -> Option<T>
where
    C: Copy,
    F: FnMut(&DataExpressionRef<'a>, C) -> ControlFlow<T, Step<Infallible, C>>,
{
    match try_visit_data_expr::<C, T, Infallible, _>(expr, context, |expr, context| Ok(visitor(expr, context))) {
        Ok(result) => result,
        Err(error) => match error {},
    }
}
