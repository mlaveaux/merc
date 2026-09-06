//! Milestone 1 of the linearisation port (see
//! `docs/linearisation-implementation-plan.md`): linearising a single,
//! self-recursive, untimed, non-stochastic, `||`-free process into a
//! [`LinearProcessSpecification`].
//!
//! # Supported grammar
//!
//! Starting from `init`, every reachable [`ProcessExprKind`] must be one of:
//! `Action`, `Delta`, `Tau`, `Sum`, `Condition`, `Binary{Choice}`,
//! `Binary{Sequence}`, `Binary{CommMerge}` (only directly between actions, to
//! write a literal multi-action such as `a|b`), `Rename`, `Hide`, `Block`,
//! `Allow` (filtering by action-name multiset, exactly as it would inside a
//! `||` — see the `Allow` arm below and the [`alphabet`] module, which can
//! often eliminate the wrapper outright), or `Id`/`Action`-as-process-instantiation
//! naming **the same** process throughout (self-recursion; mutually
//! recursive processes need a shared parameter vector this milestone does
//! not build yet — see the crate README).
//!
//! `Binary{Parallel}`, `Binary{LeftMerge}`, `Binary{Until}`, `Comm`, `At`,
//! and `Dist` are all rejected with
//! [`LinearisationError::UnsupportedConstruct`].
//!
//! A process call must be in *tail position*: nothing may be sequenced after
//! it (`P(x) . a` is rejected as [`LinearisationError::NotInTailPosition`]),
//! matching mCRL2's "regular" restriction.
//!
//! # Why `merc_syntax` expressions, not `merc_data`
//!
//! `merc_typecheck::ProcessSpecification` only exposes its process bodies as
//! `merc_syntax` syntax trees plus a span-keyed `TypingInfo` side table (see
//! its README) — there is no public API yet to lower a process body's data
//! expressions into `merc_data`'s aterm representation the way
//! `DataSpecification::lower_data_specification` does for equations. Adding
//! one is future work (tracked in the implementation plan); until then, this
//! translator only ever rearranges the expressions it's given, so staying at
//! the `merc_syntax` level costs nothing and avoids a dependency this crate
//! can't satisfy yet.

use std::collections::HashMap;
use std::collections::HashSet;

use merc_syntax::Assignment;
use merc_syntax::DataExpr;
use merc_syntax::DataExprBinaryOp;
use merc_syntax::DataExprKind;
use merc_syntax::DataExprUnaryOp;
use merc_syntax::IdDecl;
use merc_syntax::ProcDecl;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::Span;
use merc_syntax::respan;
use merc_typecheck::ProcessSpecification;

use crate::ActionInstance;
use crate::ActionSummand;
use crate::DeadlockSummand;
use crate::LinearProcessSpecification;
use crate::LinearisationError;
use crate::alphabet;
use crate::alphabet::MultiActionName;

/// Linearises `spec`'s `init` process into a [`LinearProcessSpecification`].
///
/// See the module documentation for exactly what's supported so far.
pub fn linearise(spec: &ProcessSpecification) -> Result<LinearProcessSpecification, LinearisationError> {
    let init = spec.init().ok_or(LinearisationError::MissingInit)?;

    let processes: HashMap<&str, &ProcDecl> = spec
        .process_declarations()
        .iter()
        .map(|decl| (decl.identifier.node.as_str(), decl))
        .collect();

    let (process_parameters, initial_assignment, own_name, body): (
        Vec<IdDecl>,
        Vec<DataExpr>,
        Option<&str>,
        &ProcessExpr,
    ) = match resolve_process_call(init, &processes)? {
        Some((decl, args)) => (
            decl.params.clone(),
            args,
            Some(decl.identifier.node.as_str()),
            &decl.body,
        ),
        None => (Vec::new(), Vec::new(), None, init),
    };

    linearise_process(process_parameters, initial_assignment, own_name, body, &processes)
}

/// Linearises a single process body into a [`LinearProcessSpecification`],
/// given its (already resolved) parameters, initial values, own name (for
/// recognising tail-recursive self-calls — see [`Context::own_name`]), and
/// the table of every declared process reachable from it.
///
/// Shared by [`linearise`] (called once, on `init`) and
/// `compositional::linearise_compositional` (called once per operand of the
/// top-level `||`) — the two differ only in *which* body and parameter
/// vector this runs on, not in the algorithm itself.
///
/// `body` is run through [`alphabet::simplify`] first, dropping any
/// `hide`/`block`/`allow`/`rename`/`comm` wrapper the [`alphabet`] module can
/// prove has no effect — see its module documentation. This is a pure
/// size/performance optimization: every operator `translate` implements
/// already has correct standalone semantics, so simplifying first never
/// changes the result, only how much work producing it takes.
pub(crate) fn linearise_process(
    process_parameters: Vec<IdDecl>,
    initial_assignment: Vec<DataExpr>,
    own_name: Option<&str>,
    body: &ProcessExpr,
    processes: &HashMap<&str, &ProcDecl>,
) -> Result<LinearProcessSpecification, LinearisationError> {
    let body = alphabet::simplify(body, own_name, processes);
    let ctx = Context { own_name, processes };
    let raw_summands = translate(&body, &ctx)?;

    let mut action_summands = Vec::new();
    let mut deadlock_summands = Vec::new();
    for summand in raw_summands {
        // A summand that never performs an action deadlocks outright; every
        // other tail (an action followed by `delta`/successful
        // termination/a recursive call) is an `ActionSummand`, reusing the
        // current parameter values when there is nothing further to assign
        // (sound because a terminated/deadlocked tail can never be
        // distinguished from one that revisits the same parameter values —
        // both simply never move again).
        let next_state = match &summand.tail {
            Tail::Recurse(args) => args.clone(),
            Tail::Deadlock | Tail::Terminate => process_parameters
                .iter()
                .map(|param| {
                    respan(
                        param.identifier.span.clone(),
                        DataExprKind::Resolved(param.identifier.node.clone(), param.identifier.span.clone()),
                    )
                })
                .collect(),
        };

        if summand.actions.is_empty() && matches!(summand.tail, Tail::Deadlock) {
            deadlock_summands.push(DeadlockSummand {
                summation_variables: summand.sum_vars,
                condition: summand.condition,
            });
        } else {
            action_summands.push(ActionSummand {
                summation_variables: summand.sum_vars,
                condition: summand.condition,
                multi_action: summand.actions,
                next_state,
            });
        }
    }

    Ok(LinearProcessSpecification {
        process_parameters,
        action_summands,
        deadlock_summands,
        initial_assignment,
    })
}

pub(crate) struct Context<'a> {
    /// The name of the single process being linearised, if `init` names one
    /// (absent when `init` is a bare, parameter-less expression). A process
    /// call resolving to any *other* name is rejected — see the module
    /// documentation.
    own_name: Option<&'a str>,
    processes: &'a HashMap<&'a str, &'a ProcDecl>,
}

/// What a summand does once its collected actions run out.
///
/// The three cases below look interchangeable at first (`delta`, "falls off
/// the end", and "calls `P` again" all just mean "no more actions"), but
/// [`translate_sequence`]-style composition needs to tell them apart:
/// `delta . Q` is `delta` (`Q` never starts), while `(a . nil) . Q` is
/// `a . Q` (`Q` starts right after `a`). Once a summand is finished (turned
/// into an [`ActionSummand`]/[`DeadlockSummand`] in [`linearise`]),
/// `Deadlock` and `Terminate` collapse into the same representation.
enum Tail {
    /// `delta`: this summand never performs another action.
    Deadlock,
    /// Fell off the end of a sequence with nothing further to do
    /// (`tau`/`a` alone, or the `then`/`else_` of a condition with nothing
    /// after it). Sequencing something after this splices it in; sequencing
    /// something after [`Tail::Deadlock`] does not.
    Terminate,
    /// A tail call to [`Context::own_name`] with these (already fully
    /// evaluated in the calling scope) argument expressions.
    Recurse(Vec<DataExpr>),
}

/// A summand under construction: the bound variables and guard accumulated
/// so far, the multi-action collected so far, and how it currently ends.
struct RawSummand {
    sum_vars: Vec<IdDecl>,
    condition: DataExpr,
    actions: Vec<ActionInstance>,
    tail: Tail,
}

impl RawSummand {
    fn deadlock() -> Self {
        RawSummand {
            sum_vars: Vec::new(),
            condition: true_expr(),
            actions: Vec::new(),
            tail: Tail::Deadlock,
        }
    }

    fn terminate(actions: Vec<ActionInstance>) -> Self {
        RawSummand {
            sum_vars: Vec::new(),
            condition: true_expr(),
            actions,
            tail: Tail::Terminate,
        }
    }

    fn recurse(args: Vec<DataExpr>) -> Self {
        RawSummand {
            sum_vars: Vec::new(),
            condition: true_expr(),
            actions: Vec::new(),
            tail: Tail::Recurse(args),
        }
    }
}

fn true_expr() -> DataExpr {
    respan(Span::default(), DataExprKind::Bool(true))
}

/// Builds `lhs && rhs`, dropping either side when it's the literal `true` —
/// summands accumulate a condition per nesting level, so without this most
/// conditions would otherwise be buried in a chain of `(true && ...)`s.
fn and_expr(lhs: DataExpr, rhs: DataExpr) -> DataExpr {
    if lhs.node == DataExprKind::Bool(true) {
        return rhs;
    }
    if rhs.node == DataExprKind::Bool(true) {
        return lhs;
    }
    respan(
        Span::default(),
        DataExprKind::Binary {
            op: DataExprBinaryOp::Conj,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
    )
}

fn not_expr(expr: DataExpr) -> DataExpr {
    respan(
        Span::default(),
        DataExprKind::Unary {
            op: DataExprUnaryOp::Negation,
            expr: Box::new(expr),
        },
    )
}

/// If `expr` is `Action`/`Id`-shaped *and* names a declared process, resolves
/// it to that process's declaration and its positional argument list;
/// `Ok(None)` otherwise (a genuine action, or any other expression shape).
pub(crate) fn resolve_process_call<'a>(
    expr: &ProcessExpr,
    processes: &HashMap<&str, &'a ProcDecl>,
) -> Result<Option<(&'a ProcDecl, Vec<DataExpr>)>, LinearisationError> {
    match &expr.node {
        ProcessExprKind::Action(name, args) => match processes.get(name.node.as_str()) {
            Some(&decl) => Ok(Some((decl, args.clone()))),
            None => Ok(None),
        },
        ProcessExprKind::Id(name, assignments) => {
            let &decl = processes
                .get(name.node.as_str())
                .expect("`Id` always names a declared process once type checking has succeeded");
            Ok(Some((decl, resolve_assignments(decl, assignments))))
        }
        _ => Ok(None),
    }
}

/// Resolves `P(x = e, ...)`-style assignments against `decl`'s parameters,
/// producing the positional argument list in `decl.params` order. A parameter
/// with no assignment in `assignments` keeps its current value — this is
/// mCRL2's documented semantics for the assignment notation, not a
/// shorthand this milestone invented (`P()` is a legal, and common, way to
/// write "every parameter unchanged"; see e.g. `examples/mCRL2/language/
/// numbers.mcrl2`), so it is not an error.
fn resolve_assignments(decl: &ProcDecl, assignments: &[Assignment]) -> Vec<DataExpr> {
    let by_name: HashMap<&str, &DataExpr> = assignments.iter().map(|a| (a.identifier.as_str(), &a.expr)).collect();

    decl.params
        .iter()
        .map(|param| match by_name.get(param.identifier.as_str()) {
            Some(expr) => (*expr).clone(),
            None => respan(
                param.identifier.span.clone(),
                DataExprKind::Resolved(param.identifier.node.clone(), param.identifier.span.clone()),
            ),
        })
        .collect()
}

fn translate_call(
    decl: &ProcDecl,
    args: Vec<DataExpr>,
    span: Span,
    ctx: &Context,
) -> Result<Vec<RawSummand>, LinearisationError> {
    if Some(decl.identifier.node.as_str()) == ctx.own_name {
        Ok(vec![RawSummand::recurse(args)])
    } else {
        Err(LinearisationError::MutualRecursionUnsupported {
            name: decl.identifier.node.clone(),
            span,
        })
    }
}

/// Collects the flat multi-action a `Binary{CommMerge}` (`|`) tree denotes —
/// only legal when both sides are themselves actions, `tau`, or nested `|`s
/// (i.e. `expr` is used purely to *write* a multi-action, such as `a|b`, not
/// to synchronise arbitrary behaviour, which needs full parallel-composition
/// support).
fn collect_multi_action(expr: &ProcessExpr, ctx: &Context) -> Result<Vec<ActionInstance>, LinearisationError> {
    match &expr.node {
        ProcessExprKind::Tau => Ok(Vec::new()),
        ProcessExprKind::Action(name, args) if !ctx.processes.contains_key(name.node.as_str()) => {
            Ok(vec![ActionInstance::new(name.clone(), args.clone())])
        }
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::CommMerge,
            lhs,
            rhs,
        } => {
            let mut actions = collect_multi_action(lhs, ctx)?;
            actions.extend(collect_multi_action(rhs, ctx)?);
            Ok(actions)
        }
        _ => Err(LinearisationError::UnsupportedSynchronisation {
            span: expr.span.clone(),
        }),
    }
}

fn translate(expr: &ProcessExpr, ctx: &Context) -> Result<Vec<RawSummand>, LinearisationError> {
    match &expr.node {
        ProcessExprKind::Delta => Ok(vec![RawSummand::deadlock()]),
        ProcessExprKind::Tau => Ok(vec![RawSummand::terminate(Vec::new())]),

        ProcessExprKind::Action(name, args) => match ctx.processes.get(name.node.as_str()) {
            Some(&decl) => translate_call(decl, args.clone(), expr.span.clone(), ctx),
            None => Ok(vec![RawSummand::terminate(vec![ActionInstance::new(
                name.clone(),
                args.clone(),
            )])]),
        },
        ProcessExprKind::Id(name, assignments) => {
            let &decl = ctx
                .processes
                .get(name.node.as_str())
                .expect("`Id` always names a declared process once type checking has succeeded");
            let args = resolve_assignments(decl, assignments);
            translate_call(decl, args, expr.span.clone(), ctx)
        }

        ProcessExprKind::Sum { variables, operand } => {
            let mut summands = translate(operand, ctx)?;
            for summand in &mut summands {
                let mut sum_vars = variables.clone();
                sum_vars.append(&mut summand.sum_vars);
                summand.sum_vars = sum_vars;
            }
            Ok(summands)
        }

        ProcessExprKind::Condition { condition, then, else_ } => {
            let mut result = translate(then, ctx)?;
            for summand in &mut result {
                summand.condition = and_expr(
                    condition.clone(),
                    std::mem::replace(&mut summand.condition, true_expr()),
                );
            }

            let else_summands = match else_ {
                Some(operand) => translate(operand, ctx)?,
                None => vec![RawSummand::deadlock()],
            };
            let negated = not_expr(condition.clone());
            for mut summand in else_summands {
                summand.condition = and_expr(negated.clone(), summand.condition);
                result.push(summand);
            }
            Ok(result)
        }

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs,
            rhs,
        } => {
            let mut result = translate(lhs, ctx)?;
            result.extend(translate(rhs, ctx)?);
            Ok(result)
        }

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            lhs,
            rhs,
        } => translate_sequence(lhs, rhs, ctx),

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::CommMerge,
            ..
        } => Ok(vec![RawSummand::terminate(collect_multi_action(expr, ctx)?)]),

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Parallel,
            ..
        } => Err(unsupported("|| (parallel composition)", expr.span.clone())),
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::LeftMerge,
            ..
        } => Err(unsupported("||_ (left merge)", expr.span.clone())),
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Until,
            ..
        } => Err(unsupported("<< (until)", expr.span.clone())),

        ProcessExprKind::Rename { renames, operand } => {
            let mut summands = translate(operand, ctx)?;
            let table: HashMap<&str, &str> = renames
                .iter()
                .map(|r| (r.from.node.as_str(), r.to.node.as_str()))
                .collect();
            for summand in &mut summands {
                for action in &mut summand.actions {
                    if let Some(&to) = table.get(action.name.node.as_str()) {
                        action.name = respan(action.name.span.clone(), to.to_string());
                    }
                }
            }
            Ok(summands)
        }

        ProcessExprKind::Hide {
            actions: hidden,
            operand,
        } => {
            let mut summands = translate(operand, ctx)?;
            let hidden: HashSet<&str> = hidden.iter().map(|name| name.node.as_str()).collect();
            for summand in &mut summands {
                summand
                    .actions
                    .retain(|action| !hidden.contains(action.name.node.as_str()));
            }
            Ok(summands)
        }

        ProcessExprKind::Block {
            actions: blocked,
            operand,
        } => {
            let summands = translate(operand, ctx)?;
            let blocked: HashSet<&str> = blocked.iter().map(|name| name.node.as_str()).collect();
            Ok(summands
                .into_iter()
                .filter(|summand| {
                    !summand
                        .actions
                        .iter()
                        .any(|action| blocked.contains(action.name.node.as_str()))
                })
                .collect())
        }

        ProcessExprKind::Allow { actions, operand } => {
            let summands = translate(operand, ctx)?;
            let allowed: HashSet<MultiActionName> = actions.iter().map(MultiActionName::from_label).collect();
            Ok(summands
                .into_iter()
                .filter(|summand| {
                    // `tau` always passes an `allow`: it only restricts which
                    // *visible* multi-actions may occur (mirrored in
                    // `alphabet::alphabet_of`'s `Allow` case, and matching
                    // why `crates/explore::combine_lts` rejects `tau` from
                    // ever appearing in an allow set at all).
                    summand.actions.is_empty() || allowed.contains(&MultiActionName::from_actions(&summand.actions))
                })
                .collect())
        }
        ProcessExprKind::Comm { .. } => Err(unsupported("comm", expr.span.clone())),
        ProcessExprKind::At { .. } => Err(unsupported("@ (time)", expr.span.clone())),
        ProcessExprKind::Dist { .. } => Err(unsupported("dist", expr.span.clone())),
    }
}

fn unsupported(construct: &'static str, span: Span) -> LinearisationError {
    LinearisationError::UnsupportedConstruct { construct, span }
}

/// `lhs . rhs`: every `lhs` summand that deadlocks stays as-is (`delta` is a
/// zero for sequencing); every `lhs` summand that terminates gets `rhs`
/// spliced onto it; a `lhs` summand that recurses is a process call not in
/// tail position, which Milestone 1 does not support (see the module
/// documentation).
fn translate_sequence(
    lhs: &ProcessExpr,
    rhs: &ProcessExpr,
    ctx: &Context,
) -> Result<Vec<RawSummand>, LinearisationError> {
    let lhs_summands = translate(lhs, ctx)?;
    let mut result = Vec::with_capacity(lhs_summands.len());

    for summand in lhs_summands {
        match summand.tail {
            Tail::Deadlock => result.push(summand),
            Tail::Recurse(_) => {
                return Err(LinearisationError::NotInTailPosition {
                    name: ctx.own_name.unwrap_or("<init>").to_string(),
                    span: lhs.span.clone(),
                });
            }
            Tail::Terminate => {
                for rhs_summand in translate(rhs, ctx)? {
                    let mut sum_vars = summand.sum_vars.clone();
                    sum_vars.extend(rhs_summand.sum_vars);
                    let mut actions = summand.actions.clone();
                    actions.extend(rhs_summand.actions);
                    result.push(RawSummand {
                        sum_vars,
                        condition: and_expr(summand.condition.clone(), rhs_summand.condition),
                        actions,
                        tail: rhs_summand.tail,
                    });
                }
            }
        }
    }

    Ok(result)
}
