use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use merc_aterm::ATerm;
use merc_aterm::Symbol;
use merc_aterm::Term;
use merc_aterm::TermBuilder;
use merc_aterm::Yield;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_process::Action;
use merc_process::ProcessExpression;
use merc_process::ProcessExpressionRef;
use merc_process::ProcessKind;

use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::data::free_variables;

/// What the term builder of [substitute] is given: a process expression to substitute in, or a
/// term that is already substituted.
#[derive(Debug)]
enum Input {
    Process(ATerm),
    Done(ATerm),
}

/// Applies `sigma` to the data in `expr`, renaming the variables of sums that would capture.
///
/// Bottom-up with an explicit stack; a sum is substituted by a recursive call on its operand, so
/// the recursion depth is the nesting depth of sums.
pub(crate) fn substitute(
    expr: &ProcessExpression,
    sigma: &Substitution,
    names: &mut NameGenerator,
) -> ProcessExpression {
    if sigma.is_empty() {
        return expr.clone();
    }

    let names = RefCell::new(names);
    let data = |expr: merc_data::DataExpressionRef<'_>| -> ATerm {
        sigma.apply(&expr.protect(), &mut names.borrow_mut()).into()
    };
    let mut builder = TermBuilder::<Input, Symbol>::new();
    THREAD_TERM_POOL
        .with(|tp| {
            builder.evaluate(
                tp,
                Input::Process(expr.clone().into()),
                |_tp, args, input| {
                    let term = match input {
                        Input::Done(term) => return Ok(Yield::Term(term)),
                        Input::Process(term) => term,
                    };
                    let expr = ProcessExpressionRef::from(term.copy());
                    let operand = |expr: ProcessExpressionRef<'_>| Input::Process(expr.protect().into());
                    match expr.kind() {
                        ProcessKind::Action(action) => {
                            let action = action.protect();
                            let arguments = sigma.apply_all(&action.arguments().to_vec(), &mut names.borrow_mut());
                            return Ok(Yield::Term(Action::new(&action.label().protect(), &arguments).into()));
                        }
                        ProcessKind::Process(identifier, arguments) => {
                            let arguments = sigma.apply_all(&arguments.to_vec(), &mut names.borrow_mut());
                            return Ok(Yield::Term(
                                ProcessExpression::process(&identifier.protect(), &arguments).into(),
                            ));
                        }
                        ProcessKind::Delta | ProcessKind::Tau => return Ok(Yield::Term(term)),
                        ProcessKind::Sum(variables, operand) => {
                            let (variables, inner) = sigma.bind(&variables.to_vec(), &mut names.borrow_mut());
                            let operand = substitute(&operand.protect(), &inner, &mut names.borrow_mut());
                            return Ok(Yield::Term(ProcessExpression::sum(&variables, &operand).into()));
                        }
                        ProcessKind::Stochastic(variables, distribution, operand) => {
                            let (variables, inner) = sigma.bind(&variables.to_vec(), &mut names.borrow_mut());
                            let distribution = inner.apply(&distribution.protect(), &mut names.borrow_mut());
                            let operand = substitute(&operand.protect(), &inner, &mut names.borrow_mut());
                            return Ok(Yield::Term(
                                ProcessExpression::stochastic(&variables, &distribution, &operand).into(),
                            ));
                        }
                        ProcessKind::Block(list, rhs) | ProcessKind::Hide(list, rhs) => {
                            args.push(Input::Done(list.into()));
                            args.push(operand(rhs));
                        }
                        ProcessKind::Rename(list, rhs) => {
                            args.push(Input::Done(list.into()));
                            args.push(operand(rhs));
                        }
                        ProcessKind::Comm(list, rhs) => {
                            args.push(Input::Done(list.into()));
                            args.push(operand(rhs));
                        }
                        ProcessKind::Allow(list, rhs) => {
                            args.push(Input::Done(list.into()));
                            args.push(operand(rhs));
                        }
                        ProcessKind::AtTime(lhs, time) => {
                            args.push(operand(lhs));
                            args.push(Input::Done(data(time)));
                        }
                        ProcessKind::IfThen(condition, then) => {
                            args.push(Input::Done(data(condition)));
                            args.push(operand(then));
                        }
                        ProcessKind::IfThenElse(condition, then, else_) => {
                            args.push(Input::Done(data(condition)));
                            args.push(operand(then));
                            args.push(operand(else_));
                        }
                        ProcessKind::Sync(lhs, rhs)
                        | ProcessKind::Seq(lhs, rhs)
                        | ProcessKind::BInit(lhs, rhs)
                        | ProcessKind::Merge(lhs, rhs)
                        | ProcessKind::LeftMerge(lhs, rhs)
                        | ProcessKind::Choice(lhs, rhs) => {
                            args.push(operand(lhs));
                            args.push(operand(rhs));
                        }
                    }
                    Ok(Yield::Construct(term.get_head_symbol().protect()))
                },
                |tp, symbol, args| Ok(tp.create_term_iter(&symbol, args)),
            )
        })
        .expect("substitution never fails")
        .into()
}

/// The free data variables of `expr`, in no particular order.
pub(crate) fn free_process_variables(expr: &ProcessExpression) -> HashSet<DataVariable> {
    let mut result = HashSet::new();
    for_each_data(
        expr,
        |data, bound| {
            result.extend(
                free_variables(&data)
                    .into_iter()
                    .filter(|variable| !bound.contains(variable)),
            )
        },
        |_| {},
    );
    result
}

/// The names of the variables that occur in `expr`, free or bound by a sum.
pub(crate) fn variable_names(expr: &ProcessExpression) -> HashSet<String> {
    let mut result = HashSet::new();
    let mut add =
        |variables: &[DataVariable]| result.extend(variables.iter().map(|variable| variable.name().to_string()));
    let mut free = Vec::new();
    for_each_data(expr, |data, _| free.extend(free_variables(&data)), &mut add);
    add(&free);
    result
}

/// Calls `data` on every data expression in `expr` with the variables bound around it, and
/// `binder` on the variables of every sum and stochastic operator.
///
/// Uses an explicit stack, so the depth of `expr` is not limited by the call stack.
fn for_each_data<D, B>(expr: &ProcessExpression, mut data: D, mut binder: B)
where
    D: FnMut(DataExpression, &[DataVariable]),
    B: FnMut(&[DataVariable]),
{
    let none: Rc<[DataVariable]> = Rc::from(Vec::new());
    let mut stack: Vec<(ProcessExpressionRef<'_>, Rc<[DataVariable]>)> = vec![(expr.copy(), none)];

    while let Some((expr, bound)) = stack.pop() {
        match expr.kind() {
            ProcessKind::Action(action) => {
                for argument in action.protect().arguments().iter() {
                    data(argument, &bound);
                }
            }
            ProcessKind::Process(_, arguments) => {
                for argument in arguments.iter() {
                    data(argument, &bound);
                }
            }
            ProcessKind::Delta | ProcessKind::Tau => {}
            ProcessKind::Sum(variables, operand) => {
                let variables = variables.to_vec();
                binder(&variables);
                let inner: Rc<[DataVariable]> = bound.iter().cloned().chain(variables).collect();
                stack.push((operand, inner));
            }
            ProcessKind::Stochastic(variables, distribution, operand) => {
                let variables = variables.to_vec();
                binder(&variables);
                let inner: Rc<[DataVariable]> = bound.iter().cloned().chain(variables).collect();
                data(distribution.protect(), &inner);
                stack.push((operand, inner));
            }
            ProcessKind::Block(_, operand)
            | ProcessKind::Hide(_, operand)
            | ProcessKind::Rename(_, operand)
            | ProcessKind::Comm(_, operand)
            | ProcessKind::Allow(_, operand) => stack.push((operand, bound)),
            ProcessKind::AtTime(operand, time) => {
                data(time.protect(), &bound);
                stack.push((operand, bound));
            }
            ProcessKind::IfThen(condition, then) => {
                data(condition.protect(), &bound);
                stack.push((then, bound));
            }
            ProcessKind::IfThenElse(condition, then, else_) => {
                data(condition.protect(), &bound);
                stack.push((then, bound.clone()));
                stack.push((else_, bound));
            }
            ProcessKind::Sync(lhs, rhs)
            | ProcessKind::Seq(lhs, rhs)
            | ProcessKind::BInit(lhs, rhs)
            | ProcessKind::Merge(lhs, rhs)
            | ProcessKind::LeftMerge(lhs, rhs)
            | ProcessKind::Choice(lhs, rhs) => {
                stack.push((lhs, bound.clone()));
                stack.push((rhs, bound));
            }
        }
    }
}

/// The variables of `scope` that occur free in `expr`, in the order of `scope`.
pub(crate) fn occurring_in(expr: &ProcessExpression, scope: &[DataVariable]) -> Vec<DataVariable> {
    let free = free_process_variables(expr);
    scope
        .iter()
        .filter(|variable| free.contains(*variable))
        .cloned()
        .collect()
}
