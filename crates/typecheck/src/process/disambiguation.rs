use std::collections::HashSet;
use std::convert::Infallible;

use merc_syntax::ActionName;
use merc_syntax::DataExpr;
use merc_syntax::DataExprBinaryOp;
use merc_syntax::DataExprKind;
use merc_syntax::MultiActionLabel;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::respan;

/// Every name declared as an action or a process, gathered once up front.
struct Names(HashSet<String>);

impl Names {
    fn build(spec: &UntypedProcessSpecification) -> Self {
        let mut names = HashSet::new();
        names.extend(spec.action_declarations.iter().map(|decl| decl.identifier.node.clone()));
        names.extend(
            spec.process_declarations
                .iter()
                .map(|decl| decl.identifier.node.clone()),
        );
        Names(names)
    }

    fn contains(&self, name: &str) -> bool {
        self.0.contains(name)
    }
}

/// A semantic-aware disambiguation pass. mCRL2's concrete grammar shares tokens
/// between the process algebra and the data language (most notably `.`, `+`,
/// `||`), so a context-free parser can misparse a `Condition`'s `condition`
/// field swallowing what should have been the rest of the process expression
/// into it.
///
/// See the [Process
/// Specification](https://MERCorg.github.io/merc-website/developer/typechecking/process-specification/)
/// page for the exact shapes (with parse-tree diagrams) and the known
/// limitations.
pub fn disambiguate_process_specification(spec: &mut UntypedProcessSpecification) {
    let names = Names::build(spec);
    for decl in &mut spec.process_declarations {
        disambiguate_mut(&names, &mut decl.body);
    }
    if let Some(init) = &mut spec.init {
        disambiguate_mut(&names, init);
    }
}

/// Recursively rewrites `expr` in place.
fn disambiguate_mut(names: &Names, expr: &mut ProcessExpr) {
    match expr.apply_mut::<Infallible, _>(|node| Ok(fix_swallow(names, node))) {
        Ok(()) => {}
        Err(error) => match error {},
    }
}

/// For call sites that have an owned `ProcessExpr` rather than a `&mut` one
/// already in hand.
fn disambiguate(names: &Names, mut expr: ProcessExpr) -> ProcessExpr {
    disambiguate_mut(names, &mut expr);
    expr
}

/// The [`Traverse::apply_mut`] callback [`disambiguate_mut`] drives: `None` for every `ProcessExpr`
/// variant other than `Condition`, letting `Traverse`'s own generated recursion continue into
/// that node's children unchanged. For a `Condition`, always clones and fully re-derives it via
/// [`fix_condition`]. The replacement is not itself re-descended into by `Traverse` (see its doc
/// comment); [`fix_condition`] already resolves the whole thing recursively itself.
fn fix_swallow(names: &Names, node: &ProcessExpr) -> Option<ProcessExpr> {
    let ProcessExprKind::Condition { condition, then, else_ } = &node.node else {
        return None;
    };
    Some(fix_condition(names, condition.clone(), (**then).clone(), else_.clone()))
}

/// Fixes one `Condition` node. `then` is not yet disambiguated — this function recurses into it
/// itself, since the `+`/`||` swallow (see the module doc comment) needs to see it in its raw,
/// as-parsed shape to recognize it: a bare `cond -> then` (no `<>`) leaves `then` unrestricted, so
/// a further `+ cond2 -> ...` (or `|| cond2 -> ...`) can swallow straight into it (`take_swallow`'s
/// job). `else_`, when present, is disambiguated normally and re-attached to whichever clause
/// `condition`/`then` end up belonging to — it is *not* itself a place this swallow can start (an
/// explicit `<>` always closes the construct it belongs to), but it must never be dropped,
/// including when `then` did hide a swallow: a nested `if` inside `then` can have its own,
/// unrelated swallow regardless of whether the outer construct has an `else_` of its own.
///
/// A leading `.`-prefix peeled off `condition` (see [`peel_dot_prefix`]) sequences *before* the
/// whole (possibly `+`/`||`-split) construct, not inside its `then`: `a(1) . cond -> P` means
/// `a(1) . (cond -> P)` — the action happens unconditionally, then `cond -> P` follows — not `cond
/// -> (a(1) . P)`.
fn fix_condition(
    names: &Names,
    condition: DataExpr,
    then: ProcessExpr,
    else_: Option<Box<ProcessExpr>>,
) -> ProcessExpr {
    // Peel `condition` itself: any leading, fully-unconditional `+`/`||`-branches first (a whole
    // run with no `->` of its own at all, up to the *one* `->` this construct actually has — see
    // `peel_condition`), then a leading `.`-prefix off whatever's left (the plain `.` case).
    let (branches, prefix, condition) = peel_condition(names, condition);
    let else_ = else_.map(|boxed| Box::new(disambiguate(names, *boxed)));

    let result = match take_swallow(names, then) {
        Ok((op, this_then, rest)) => {
            // `then` hid a further `+`/`||`-separated clause (see `take_swallow`). `else_` belongs
            // to *this* clause, not `rest` — an explicit `<>` always closes the construct it was
            // written on, never a clause a swallow only revealed after the fact.
            let this_span = Span {
                start: condition.span.start,
                end: this_then.span.end,
            };
            let this_clause = ProcessExprKind::Condition {
                condition,
                then: Box::new(this_then),
                else_,
            }
            .spanned(this_span.clone());
            let choice_span = Span {
                start: this_span.start,
                end: rest.span.end,
            };
            ProcessExprKind::Binary {
                op,
                lhs: Box::new(this_clause),
                rhs: Box::new(rest),
            }
            .spanned(choice_span)
        }
        Err(then) => {
            let this_then = disambiguate(names, *then);
            let this_span = Span {
                start: condition.span.start,
                end: this_then.span.end,
            };
            let this_span = match &else_ {
                Some(else_) => Span {
                    end: else_.span.end,
                    ..this_span
                },
                None => this_span,
            };
            ProcessExprKind::Condition {
                condition,
                then: Box::new(this_then),
                else_,
            }
            .spanned(this_span)
        }
    };

    let result = prepend_seq(names, prefix, result);

    // Re-attach the leading unconditional branches peeled off above, each joined back with the
    // operator it was swallowed with (`+`/`Choice` or `||`/`Parallel`) — `r_assign_turnA.Turn(A) +
    // r_assign_turnB.Turn(B) + (t == A) -> ...` (`mutex.mcrl2`'s `Turn`) has no `->` at all before
    // its first two `+`s, so its *whole* run up to the process's one and only `->` swallows into
    // this single `Condition`'s `condition` in one shot, rather than cascading through nested
    // `Condition`s the way `take_swallow`'s two shapes do.
    branches.into_iter().rev().fold(result, |acc, (op, branch)| {
        let span = Span {
            start: branch.span.start,
            end: acc.span.end,
        };
        ProcessExprKind::Binary {
            op,
            lhs: Box::new(branch),
            rhs: Box::new(acc),
        }
        .spanned(span)
    })
}

/// Maps a data-expression binary operator to the process operator it's ambiguous with in the
/// shared-token grammar (see the module doc comment) — `None` for every other operator. The one
/// mapping shared by [`peel_condition`], [`take_swallow`], [`is_fully_process_content`], and
/// [`reinterpret_as_process`].
fn swallowed_op(op: &DataExprBinaryOp) -> Option<ProcExprBinaryOp> {
    match op {
        DataExprBinaryOp::Add => Some(ProcExprBinaryOp::Choice),
        DataExprBinaryOp::Disj => Some(ProcExprBinaryOp::Parallel),
        _ => None,
    }
}

/// Peels a leading run of fully-unconditional `+`/`||`-branches off `condition` (see the comment
/// on [`fix_condition`]'s call site), then — per [`peel_dot_prefix`] — a leading `.`-prefix off
/// whatever's left. Returns `(branches, dot_prefix, real_condition)`; `branches` is empty in the
/// overwhelmingly common case (nothing to peel at this level at all). Each branch carries the
/// process operator (`Choice` or `Parallel`) it was swallowed with, for [`fix_condition`] to
/// re-attach it with.
fn peel_condition(
    names: &Names,
    condition: DataExpr,
) -> (Vec<(ProcExprBinaryOp, ProcessExpr)>, Vec<DataExpr>, DataExpr) {
    let mut branches = Vec::new();
    let mut current = condition;
    loop {
        let swallow = match &current.node {
            DataExprKind::Binary { op, lhs, .. } => swallowed_op(op).filter(|_| is_fully_process_content(names, lhs)),
            _ => None,
        };
        let Some(proc_op) = swallow else {
            break;
        };
        let DataExprKind::Binary { lhs, rhs, .. } = current.into_node() else {
            unreachable!("just matched this shape above");
        };
        branches.push((proc_op, reinterpret_as_process(names, *lhs)));
        current = *rhs;
    }
    let (prefix, remaining) = peel_dot_prefix(names, current);
    (branches, prefix, remaining)
}

/// If `node` is `Condition { condition: Binary { op, lhs, rhs }, then: inner_then, else_:
/// inner_else }` with `op` a swallowed operator (see [`swallowed_op`]) and `lhs` pure declared
/// process content (see [`is_fully_process_content`]), fixes and extracts it as `Ok((process_op,
/// this_clause_content, rest))` — `rest` fully disambiguated and ready to use as-is. Otherwise hands
/// `node` straight back, boxed, as `Err` (boxed only to keep this `Result` from ballooning to
/// `ProcessExprKind`'s own size — clippy's `result_large_err`).
///
/// This is the one shape the swallow (see the module doc comment) can actually produce as `then`:
/// the second `->` in `cond -> lhs + rhs -> inner_then <...>` was swallowed *inside* the
/// shared-token `DataExpr` grammar, which only ever happens through `condition`. A bare
/// `Binary { op: Choice, .. }` sitting directly as `then` is deliberately **not** treated as a
/// swallow here: `cond -> a + b` and `cond -> (a + b)` produce byte-identical trees, so nothing
/// here can distinguish a user's own, already-correct `a + b` from one that needs splitting.
fn take_swallow(
    names: &Names,
    node: ProcessExpr,
) -> Result<(ProcExprBinaryOp, ProcessExpr, ProcessExpr), Box<ProcessExpr>> {
    let swallow = if let ProcessExprKind::Condition { condition, .. } = &node.node
        && let DataExprKind::Binary { op, lhs, .. } = &condition.node
    {
        swallowed_op(op).filter(|_| is_fully_process_content(names, lhs))
    } else {
        None
    };
    let Some(proc_op) = swallow else {
        return Err(Box::new(node));
    };

    let ProcessExprKind::Condition {
        condition,
        then: inner_then,
        else_: inner_else,
    } = node.into_node()
    else {
        unreachable!("just matched this shape above");
    };
    let DataExprKind::Binary { lhs, rhs, .. } = condition.into_node() else {
        unreachable!("just matched this shape above");
    };
    let this = reinterpret_as_process(names, *lhs);
    let rest = fix_condition(names, *rhs, *inner_then, inner_else);
    Ok((proc_op, this, rest))
}

/// Peels a leading run of declared-action/process steps off `condition`'s left-associative `.`
/// chain, returning `(steps, remaining_condition)` — an empty `steps` and `condition` unchanged
/// if no leading step is found.
fn peel_dot_prefix(names: &Names, condition: DataExpr) -> (Vec<DataExpr>, DataExpr) {
    let mut pieces = flatten_at_chain(condition).into_iter();
    let mut prefix = Vec::new();
    for piece in pieces.by_ref() {
        if is_declared_step(names, &piece) {
            prefix.push(piece);
        } else {
            let rest: Vec<DataExpr> = std::iter::once(piece).chain(pieces).collect();
            return (prefix, rebuild_at_chain(rest));
        }
    }
    // Every piece looked like a declared step: there's no real boolean condition left at all.
    // Pathological (the resulting specification won't type check either way) — leave it alone.
    (Vec::new(), rebuild_at_chain(prefix))
}

/// Flattens a left-associative `Binary { op: At, lhs, rhs }` chain (`a . b . c` parses as
/// `(a . b) . c`) back into its original left-to-right pieces `[a, b, c]`. An `expr` whose
/// top-level node isn't `At` at all flattens to the single piece `[expr]`.
fn flatten_at_chain(expr: DataExpr) -> Vec<DataExpr> {
    let mut parts = Vec::new();
    let mut current = expr;
    loop {
        let (node, span) = current.into_parts();
        match node {
            DataExprKind::Binary {
                op: DataExprBinaryOp::At,
                lhs,
                rhs,
            } => {
                parts.push(*rhs);
                current = *lhs;
            }
            other => {
                parts.push(other.spanned(span));
                break;
            }
        }
    }
    parts.reverse();
    parts
}

/// The inverse of [`flatten_at_chain`]: left-folds `pieces` back into one left-associative `At`
/// chain. Panics on an empty `pieces` — never called with one.
fn rebuild_at_chain(pieces: Vec<DataExpr>) -> DataExpr {
    let mut pieces = pieces.into_iter();
    let mut result = pieces
        .next()
        .expect("rebuild_at_chain is never called with an empty piece list");
    for piece in pieces {
        let span = Span {
            start: result.span.start,
            end: piece.span.end,
        };
        result = DataExprKind::Binary {
            op: DataExprBinaryOp::At,
            lhs: Box::new(result),
            rhs: Box::new(piece),
        }
        .spanned(span);
    }
    result
}

/// A single-action `hide`/`block`/`allow` applied to `(set, operand)` — the one process-operator
/// shape that can appear in the ambiguous position, since it uses ordinary function-call syntax
/// (see the module doc comment). `rename`/`comm` (`from -> to` pairs) and a multi-action `allow`
/// entry are not valid `DataExpr` syntax, so they can't reach here at all — a parse error instead.
#[derive(Clone, Copy)]
enum ProcessOperator {
    Hide,
    Block,
    Allow,
}

impl ProcessOperator {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "hide" => Some(Self::Hide),
            "block" => Some(Self::Block),
            "allow" => Some(Self::Allow),
            _ => None,
        }
    }
}

/// Recognizes a single-action `hide`/`block`/`allow` in the ambiguous position:
/// `Application { function: Id(name), arguments: [set, operand] }` where `name` is one of the
/// three operators, `set` is a clean enumeration of plain identifiers (see [`set_id_names`]), and
/// `operand` is (recursively) pure process content. `None` on anything else, including a
/// 2-argument call that merely happens to share one of these three names, or a `set` that isn't a
/// plain `{a, b, ...}` enumeration (a set comprehension, say) — those are left as an ordinary
/// `Condition`.
fn process_operator_application(names: &Names, expr: &DataExpr) -> Option<ProcessOperator> {
    let DataExprKind::Application { function, arguments } = &expr.node else {
        return None;
    };
    let DataExprKind::Id(name) = &function.node else {
        return None;
    };
    let operator = ProcessOperator::from_name(name)?;
    let [set, operand] = arguments.as_slice() else {
        return None;
    };
    set_id_names(set)?;
    is_fully_process_content(names, operand).then_some(operator)
}

/// `set`'s elements, each paired with the [Span] it was parsed from, if it is a
/// `DataExprKind::Set` enumeration of plain identifiers only — `None` on anything else (a set
/// comprehension, a non-`Id` element, or not a `Set` at all).
fn set_id_names(set: &DataExpr) -> Option<Vec<ActionName>> {
    let DataExprKind::Set(elements) = &set.node else {
        return None;
    };
    elements
        .iter()
        .map(|element| match &element.node {
            DataExprKind::Id(name) => Some(respan(element.span.clone(), name.clone())),
            _ => None,
        })
        .collect()
}

/// Whether `piece` is a bare identifier or application naming a declared action or process, or
/// `delta`/`tau` (the two nullary process constants — reserved words in the process algebra, but
/// not excluded from the data grammar's own `Id` token, so a swallowed `delta`/`tau` parses as an
/// ordinary [`DataExprKind::Id`] rather than [`merc_syntax::ProcessExprKind::Delta`]/`Tau`), or a
/// recognized single-action `hide`/`block`/`allow` application (see
/// [`process_operator_application`]) — the leaf shape [`reinterpret_as_process`] turns into a
/// process step.
fn is_declared_step(names: &Names, piece: &DataExpr) -> bool {
    let is_named_step = match &piece.node {
        DataExprKind::Id(name) => names.contains(name) || is_process_constant(name),
        DataExprKind::Application { function, .. } => {
            matches!(&function.node, DataExprKind::Id(name) if names.contains(name))
        }
        _ => false,
    };
    is_named_step || process_operator_application(names, piece).is_some()
}

/// Whether `name` is one of mCRL2's nullary process constants — see [`is_declared_step`].
fn is_process_constant(name: &str) -> bool {
    name == "delta" || name == "tau"
}

/// Whether every leaf of `expr`'s `.`/`+`/`||` chain is a declared action/process step (or a
/// recognized `hide`/`block`/`allow` leaf) — the precondition [`reinterpret_as_process`] relies
/// on.
fn is_fully_process_content(names: &Names, expr: &DataExpr) -> bool {
    match &expr.node {
        DataExprKind::Binary { op, lhs, rhs } if *op == DataExprBinaryOp::At || swallowed_op(op).is_some() => {
            is_fully_process_content(names, lhs) && is_fully_process_content(names, rhs)
        }
        _ => is_declared_step(names, expr),
    }
}

/// Converts `expr` — confirmed by [`is_fully_process_content`]/[`is_declared_step`] to be pure
/// process content — into the `ProcessExpr` it should have parsed as: `.` becomes `Sequence`,
/// `+`/`||` become `Choice`/`Parallel` (via [`swallowed_op`]), a recognized `hide`/`block`/`allow`
/// application becomes the matching `ProcessExprKind` variant, and every other `Id`/`Application`
/// leaf becomes an `Action` step. The grammar hands out `ProcessExprKind::Action` for both an
/// action instance and a positional process instantiation (see the crate README);
/// [`super::check`] resolves which table it belongs to.
fn reinterpret_as_process(names: &Names, expr: DataExpr) -> ProcessExpr {
    let span = expr.span.clone();
    match expr.into_node() {
        DataExprKind::Binary {
            op: DataExprBinaryOp::At,
            lhs,
            rhs,
        } => ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            lhs: Box::new(reinterpret_as_process(names, *lhs)),
            rhs: Box::new(reinterpret_as_process(names, *rhs)),
        }
        .spanned(span),
        DataExprKind::Binary { op, lhs, rhs } if swallowed_op(&op).is_some() => {
            let proc_op = swallowed_op(&op).expect("just checked Some above");
            ProcessExprKind::Binary {
                op: proc_op,
                lhs: Box::new(reinterpret_as_process(names, *lhs)),
                rhs: Box::new(reinterpret_as_process(names, *rhs)),
            }
            .spanned(span)
        }
        DataExprKind::Application { function, arguments } => {
            // The identifier's own span, not the whole `name(args)` application's — captured
            // before `function.node` is destructured below.
            let name_span = function.span.clone();
            let DataExprKind::Id(name) = function.into_node() else {
                unreachable!("is_fully_process_content only accepts an Id-headed application");
            };
            match (ProcessOperator::from_name(&name), <[DataExpr; 2]>::try_from(arguments)) {
                (Some(operator), Ok([set, operand])) => build_process_operator(names, operator, set, operand, span),
                (_, Ok(pair)) => ProcessExprKind::Action(respan(name_span, name), pair.into()).spanned(span),
                (_, Err(arguments)) => ProcessExprKind::Action(respan(name_span, name), arguments).spanned(span),
            }
        }
        DataExprKind::Id(name) if name == "delta" => ProcessExprKind::Delta.spanned(span),
        DataExprKind::Id(name) if name == "tau" => ProcessExprKind::Tau.spanned(span),
        // A bare `Id` leaf's own span *is* the whole expression's span.
        DataExprKind::Id(name) => ProcessExprKind::Action(respan(span.clone(), name), Vec::new()).spanned(span),
        _ => unreachable!(
            "is_fully_process_content only accepts At/Add/Disj chains, hide/block/allow, or Id/Application leaves"
        ),
    }
}

/// Builds the `Hide`/`Block`/`Allow` node for a recognized [`process_operator_application`] leaf.
/// `set` is re-walked via [`set_id_names`] rather than threaded through from the recognizer
/// (which only returns a bool) — `process_operator_application` already guarantees it is a clean
/// `Id` enumeration, so this never fails.
fn build_process_operator(
    names: &Names,
    operator: ProcessOperator,
    set: DataExpr,
    operand: DataExpr,
    span: Span,
) -> ProcessExpr {
    let action_names =
        set_id_names(&set).expect("process_operator_application already verified this is a clean Id set");
    let operand = Box::new(reinterpret_as_process(names, operand));
    match operator {
        ProcessOperator::Hide => ProcessExprKind::Hide {
            actions: action_names,
            operand,
        }
        .spanned(span),
        ProcessOperator::Block => ProcessExprKind::Block {
            actions: action_names,
            operand,
        }
        .spanned(span),
        ProcessOperator::Allow => ProcessExprKind::Allow {
            actions: action_names
                .into_iter()
                .map(|name| MultiActionLabel::new(vec![name]))
                .collect(),
            operand,
        }
        .spanned(span),
    }
}

/// Sequences `prefix` (in original left-to-right order) before `tail`; an empty `prefix` folds
/// away to just `tail`.
fn prepend_seq(names: &Names, prefix: Vec<DataExpr>, tail: ProcessExpr) -> ProcessExpr {
    prefix.into_iter().rev().fold(tail, |acc, piece| {
        let step = reinterpret_as_process(names, piece);
        let span = Span {
            start: step.span.start,
            end: acc.span.end,
        };
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            lhs: Box::new(step),
            rhs: Box::new(acc),
        }
        .spanned(span)
    })
}

#[cfg(test)]
mod tests {
    use merc_syntax::UntypedProcessSpecification;

    use super::*;

    fn disambiguated_body(text: &str) -> ProcessExpr {
        let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
        disambiguate_process_specification(&mut spec);
        spec.init.take().expect("the fixture always has an init")
    }

    fn disambiguated_proc_body(text: &str) -> ProcessExpr {
        let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
        disambiguate_process_specification(&mut spec);
        let decl = spec.process_declarations.pop().expect("the fixture always has a proc");
        decl.body
    }

    /// A single leading action step swallowed into the condition via `.` — the classic
    /// `act(args) . cond -> P <> Q` shape.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn dot_swallowed_action_prefix_is_recovered() {
        let body = disambiguated_body("act a: Nat; init a(1) . true -> delta;");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Sequence, got {body:?}");
        };
        assert!(matches!(&lhs.node, ProcessExprKind::Action(name, _) if name.node == "a"));
        assert!(matches!(&rhs.node, ProcessExprKind::Condition { .. }));
    }

    /// A `+`-separated chain of guarded actions, each swallowed into the previous clause's
    /// condition.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn choice_swallowed_guarded_actions_are_recovered() {
        let body = disambiguated_body("act a: Nat; b: Nat; init (true) -> a(1) + (false) -> b(2);");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        let ProcessExprKind::Condition { then: lhs_then, .. } = &lhs.node else {
            panic!("expected a Condition, got {lhs:?}");
        };
        assert!(matches!(&lhs_then.node, ProcessExprKind::Action(name, _) if name.node == "a"));
        let ProcessExprKind::Condition { then: rhs_then, .. } = &rhs.node else {
            panic!("expected a Condition, got {rhs:?}");
        };
        assert!(matches!(&rhs_then.node, ProcessExprKind::Action(name, _) if name.node == "b"));
    }

    /// The same shape with `||` (`Parallel`) in place of `+` (`Choice`): `P || (true) -> a` parses
    /// `P || (true)` as one `DataExprDisj` chain rather than a parallel-composition operand
    /// followed by the real condition.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn parallel_swallowed_condition_is_recovered() {
        let body = disambiguated_body("act a; proc P = delta; init P || (true) -> a;");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Parallel,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Parallel, got {body:?}");
        };
        assert!(matches!(&lhs.node, ProcessExprKind::Action(name, _) if name.node == "P"));
        let ProcessExprKind::Condition { condition, then, .. } = &rhs.node else {
            panic!("expected a Condition, got {rhs:?}");
        };
        assert!(matches!(&condition.node, DataExprKind::Bool(true)));
        assert!(matches!(&then.node, ProcessExprKind::Action(name, _) if name.node == "a"));
    }

    /// A three-way `+`-chain, each clause's action itself preceded by a `.`-sequence — combines
    /// both swallow shapes at once.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn three_way_choice_with_sequenced_actions_is_recovered() {
        let body = disambiguated_body(
            "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) . b(1) + (false) -> a(2) . b(2) + (true) -> c(3);",
        );
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        let ProcessExprKind::Condition { then: first_then, .. } = &lhs.node else {
            panic!("expected a Condition, got {lhs:?}");
        };
        assert!(matches!(
            &first_then.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));

        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs: second,
            rhs: third,
        } = &rhs.node
        else {
            panic!("expected a nested Choice, got {rhs:?}");
        };
        let ProcessExprKind::Condition { then: second_then, .. } = &second.node else {
            panic!("expected a Condition, got {second:?}");
        };
        assert!(matches!(
            &second_then.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));
        let ProcessExprKind::Condition { then: third_then, .. } = &third.node else {
            panic!("expected a Condition, got {third:?}");
        };
        assert!(matches!(&third_then.node, ProcessExprKind::Action(name, _) if name.node == "c"));
    }

    /// A `+`-separated chain in `else_` position — `cond -> then <> a + cond2 -> then2 <> else2`,
    /// mirroring `knights.mcrl2`'s `(...) -> jump . X(...) <> delta + (f==finalBoard) -> ready .
    /// delta <> delta`. A bare `Binary { op: Choice, .. }` sitting directly in `then`/`else_`
    /// position is indistinguishable from a genuine, deliberate `a + b` the user wrote (`cond ->
    /// (a + b)` and `cond -> a + b` parse identically), so it is deliberately left alone — see
    /// [`take_swallow`]'s doc comment.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn choice_directly_in_else_position_is_left_alone() {
        let body = disambiguated_proc_body(
            "act jump, ready; proc X(f: Bool) = (f) -> jump . X(f) <> delta + (!f) -> ready . delta <> delta; init X(true);",
        );
        let ProcessExprKind::Condition { then, else_, .. } = &body.node else {
            panic!("expected a Condition, got {body:?}");
        };
        assert!(matches!(
            &then.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));
        let Some(else_) = else_ else {
            panic!("expected an else_ branch, got None");
        };
        assert!(matches!(
            &else_.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Choice,
                ..
            }
        ));
    }

    /// Minimal form of the same shape: `then` hiding an unrelated (`Add`-shaped) swallow must never
    /// cost the current clause its own `<>`. `(ps==1) -> a(1) + (ps==2) -> a(2) <> b(9)` puts the
    /// swallow (`a(1) + (ps==2) -> ...`) one level down from the outermost condition, and the
    /// grammar attaches `<>`'s `b(9)` to the *innermost* reached `->` (`(ps==2)`, not `(ps==1)`).
    /// `else_` still must not be dropped by `take_swallow`'s `Ok` arm, wherever the grammar
    /// actually attaches it.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn else_branch_survives_a_swallow_one_level_down() {
        let body = disambiguated_body("act a: Nat; b: Nat; init (true) -> a(1) + (false) -> a(2) <> b(9);");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs: first,
            rhs: second,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        let ProcessExprKind::Condition { else_: first_else, .. } = &first.node else {
            panic!("expected a Condition, got {first:?}");
        };
        assert!(first_else.is_none(), "the swallow consumes the only `<>` in this input");
        let ProcessExprKind::Condition { else_: second_else, .. } = &second.node else {
            panic!("expected a Condition, got {second:?}");
        };
        let Some(second_else) = second_else else {
            panic!("expected `<> b(9)` to survive on the clause the grammar actually attaches it to");
        };
        assert!(matches!(&second_else.node, ProcessExprKind::Action(name, _) if name.node == "b"));
    }

    /// The minimal shape of the same class as [`choice_directly_in_else_position_is_left_alone`],
    /// with `then` (not `else_`) directly `a + b`: `cond -> a + b <> c` must keep `b` unconditional
    /// (not folded into `cond`'s own guard) *and* keep `c` as the `else_`.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn choice_directly_in_then_position_keeps_its_own_else() {
        let body = disambiguated_body("act a: Nat; b: Nat; c: Nat; init (true) -> a(1) + b(2) <> c(3);");
        let ProcessExprKind::Condition { condition, then, else_ } = &body.node else {
            panic!("expected a Condition, got {body:?}");
        };
        assert!(matches!(&condition.node, DataExprKind::Bool(true)));
        assert!(
            matches!(
                &then.node,
                ProcessExprKind::Binary {
                    op: ProcExprBinaryOp::Choice,
                    ..
                }
            ),
            "a + b must stay intact, got {then:?}"
        );
        let Some(else_) = else_ else {
            panic!("expected `<> c(3)` to survive, got None");
        };
        assert!(matches!(&else_.node, ProcessExprKind::Action(name, _) if name.node == "c"));
    }

    /// Leading, wholly *unconditional* `+`-branches (no `->` of their own at all) swallowed
    /// straight into the one and only `Condition`'s `condition` field — the shape produced when a
    /// process body has just one `->` in it, so the *whole* run up to that point is one legal
    /// `DataExpr`. Mirrors `mutex.mcrl2`'s `Turn`: `r_assign_turnA.Turn(A) + r_assign_turnB.Turn(B)
    /// + (t==A) -> s_read_turnA|label(true).Turn(A)`.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn unconditional_leading_branches_are_recovered() {
        let body = disambiguated_proc_body("act a, b, c; proc P = a.P + b.P + (true) -> c.P; init P;");
        // The two unconditional branches (`a.P`, `b.P`) peel off together, in one `Add`-chain
        // piece, as a nested `Choice` of their own; the real, guarded clause (`c.P`) is the
        // right-hand side.
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs: branches,
            rhs: guarded,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs: first,
            rhs: second,
        } = &branches.node
        else {
            panic!("expected a nested Choice, got {branches:?}");
        };
        assert!(matches!(
            &first.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));
        assert!(matches!(
            &second.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));
        let ProcessExprKind::Condition { condition, then, else_ } = &guarded.node else {
            panic!("expected a Condition, got {guarded:?}");
        };
        assert!(matches!(&condition.node, DataExprKind::Bool(true)));
        assert!(matches!(
            &then.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                ..
            }
        ));
        assert!(else_.is_none());
    }

    /// A genuine data expression that merely happens to use `.`/`+` — nothing declared as an
    /// action or process appears in it — must be left untouched.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn a_genuine_condition_using_at_and_plus_is_left_alone() {
        let body = disambiguated_body("sort L = List(Nat); init (([1,2,3] . 0) + 1 == 2) -> delta;");
        assert!(matches!(&body.node, ProcessExprKind::Condition { .. }));
    }

    /// Nothing to fix at all: a plain, already-well-formed condition is unchanged.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn an_ordinary_condition_is_left_alone() {
        let body = disambiguated_body("init true -> delta;");
        let ProcessExprKind::Condition { condition, then, else_ } = &body.node else {
            panic!("expected a Condition, got {body:?}");
        };
        assert!(matches!(&condition.node, DataExprKind::Bool(true)));
        assert!(matches!(&then.node, ProcessExprKind::Delta));
        assert!(else_.is_none());
    }

    /// `hide({a}, P) + (true) -> b` swallows the same way `+` alone does: `hide(...)` is
    /// recognized as a process step via [`process_operator_application`], so the whole run peels
    /// as an unconditional leading branch, and `hide`'s own `{a}` set survives as `Hide`'s
    /// `actions`.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn hide_swallowed_guarded_choice_is_recovered() {
        let body = disambiguated_body("act a, b; proc P = delta; init hide({a}, P) + (true) -> b;");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        let ProcessExprKind::Hide { actions, operand } = &lhs.node else {
            panic!("expected a Hide, got {lhs:?}");
        };
        assert_eq!(actions.iter().map(|name| name.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert!(matches!(&operand.node, ProcessExprKind::Action(name, _) if name.node == "P"));
        let ProcessExprKind::Condition { then, .. } = &rhs.node else {
            panic!("expected a Condition, got {rhs:?}");
        };
        assert!(matches!(&then.node, ProcessExprKind::Action(name, _) if name.node == "b"));
    }

    /// `comm` uses `from -> to` pairs inside its set argument, and `->` isn't a valid `DataExpr`
    /// operator, so it can't reach the swallow at all — a hard parse error, not a silent misparse.
    /// `rename` shares the same `from -> to` shape but does not match the ambiguous
    /// `DataExpr`-typed `condition` position, falling back to its own dedicated grammar rule, so
    /// it parses correctly here with nothing to fix.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn comm_fails_to_parse_and_rename_already_parses_correctly() {
        assert!(
            UntypedProcessSpecification::parse("act a, b; proc P = delta; init comm({a -> b}, P) + (true) -> b;")
                .is_err()
        );

        let body = disambiguated_body("act a, b; proc P = delta; init rename({a -> b}, P) + (true) -> b;");
        let ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            lhs,
            rhs,
        } = &body.node
        else {
            panic!("expected a Choice, got {body:?}");
        };
        assert!(matches!(&lhs.node, ProcessExprKind::Rename { .. }));
        assert!(matches!(&rhs.node, ProcessExprKind::Condition { .. }));
    }

    /// The fixture [`long_guarded_choice_chain_with_comments_is_recovered`] and
    /// [`disambiguation_is_idempotent_for_the_long_guarded_choice_chain`] share: a long `is_x(state) ->
    /// (...) + is_y(state) -> (...) + ...` chain of eight guarded clauses, each `then` a
    /// parenthesized `.`/`+` mix, with a `%`-comment sitting between two clauses.
    const LONG_GUARDED_CHOICE_CHAIN: &str = r#"act _JobWrapper_initialize, _JobWrapper_transferInputSandbox, _JobWrapper_resolveInputData, _JobWrapper_execute, _JobWrapper_processJobOutputs, _JobWrapper_finalize, AppPayload;
proc JobWrapper(cc:List(Nat),state:Nat) =

		is_startWrapper(state)->
		(
		_JobWrapper_initialize(cc,state).
			(
			_JobWrapper_initialize(cc,state).JobWrapper(cc,state) +
			_JobWrapper_initialize(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_initializeOK(state)->
		(
			_JobWrapper_transferInputSandbox(cc,state).
			(
			_JobWrapper_transferInputSandbox(cc,state).JobWrapper(cc,state) +
			_JobWrapper_transferInputSandbox(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_inputSandboxOK(state)->
		(
			_JobWrapper_resolveInputData(cc,state).
			(
			_JobWrapper_resolveInputData(cc,state).JobWrapper(cc,state) +
			_JobWrapper_resolveInputData(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_resolveInputDataOK(state)->
		(
			_JobWrapper_execute(cc,state).
			(
			AppPayload.JobWrapper(cc,state) +
			_JobWrapper_execute(cc,state).JobWrapper(cc,state)
			)

		)
		+
		% creates an execution thread
		% passes the execution thread to a Watchdog instance (TBW)

		is_executeOK(state)->
		(
			_JobWrapper_execute(cc,state).JobWrapper(cc,state) +
		 	_JobWrapper_execute(cc,state).JobWrapper(cc,state) +
		 	_JobWrapper_execute(cc,state).JobWrapper(cc,state)
		)
		+

		is_completedOK(state)->
		(
			_JobWrapper_processJobOutputs(cc,state).
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
			+
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
		)
		+
		is_outputSandboxOK(state)->
		(
			_JobWrapper_processJobOutputs(cc,state).
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
			+
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
		)
		+
		is_outputDataOK(state)->
		(
			_JobWrapper_finalize(cc,state) +
			_JobWrapper_finalize(cc,state) +
			_JobWrapper_finalize(cc,state)
		).JobWrapper(cc,state)
	;
init JobWrapper([],0);
"#;

    /// A long `is_x(state) -> (...) +
    /// is_y(state) -> (...) + ...` chain of eight guarded clauses, each `then` a parenthesized
    /// `.`/`+` mix, with a `%`-comment sitting between two clauses. Every guard must recover as its
    /// own `Condition`, in order, not swallow a neighboring clause.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn long_guarded_choice_chain_with_comments_is_recovered() {
        let body = disambiguated_proc_body(LONG_GUARDED_CHOICE_CHAIN);

        // Flattens the top-level `Choice` spine, returning each clause's guard name (the
        // function name of its `Condition`'s `condition`, which is always a plain application
        // here: `is_x(state)`).
        fn guard_names(expr: &ProcessExpr) -> Vec<&str> {
            match &expr.node {
                ProcessExprKind::Binary {
                    op: ProcExprBinaryOp::Choice,
                    lhs,
                    rhs,
                } => {
                    let mut names = guard_names(lhs);
                    names.extend(guard_names(rhs));
                    names
                }
                ProcessExprKind::Condition { condition, .. } => {
                    let DataExprKind::Application { function, .. } = &condition.node else {
                        panic!("expected an application guard, got {condition:?}");
                    };
                    let DataExprKind::Id(name) = &function.node else {
                        panic!("expected an Id-headed guard, got {function:?}");
                    };
                    vec![name.as_str()]
                }
                _ => panic!("expected a Choice or a Condition, got {expr:?}"),
            }
        }

        assert_eq!(
            guard_names(&body),
            [
                "is_startWrapper",
                "is_initializeOK",
                "is_inputSandboxOK",
                "is_resolveInputDataOK",
                "is_executeOK",
                "is_completedOK",
                "is_outputSandboxOK",
                "is_outputDataOK",
            ],
            "every guard must recover as its own Condition, in source order"
        );
    }

    /// Disambiguating an already-disambiguated specification must find nothing left to rewrite. Covers every shape the
    /// tests above exercise (a dot-prefix swallow, a choice/parallel swallow, a swallow nested one
    /// level down inside `then`, `else_` surviving every which way, leading unconditional
    /// branches, `hide`/`rename`, and a genuine data condition that must be left alone) in one
    /// pass; [`disambiguation_is_idempotent_for_the_long_guarded_choice_chain`] covers the long
    /// real-world chain separately, since it needs the whole `spec` (not one extracted body).
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn disambiguation_is_idempotent() {
        let fixtures = [
            "act a: Nat; init a(1) . true -> delta;",
            "act a: Nat; b: Nat; init (true) -> a(1) + (false) -> b(2);",
            "act a; proc P = delta; init P || (true) -> a;",
            "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) . b(1) + (false) -> a(2) . b(2) + (true) -> c(3);",
            "act jump, ready; proc X(f: Bool) = (f) -> jump . X(f) <> delta + (!f) -> ready . delta <> delta; init X(true);",
            "act a: Nat; b: Nat; init (true) -> a(1) + (false) -> a(2) <> b(9);",
            "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) + b(2) <> c(3);",
            "act a, b, c; proc P = a.P + b.P + (true) -> c.P; init P;",
            "sort L = List(Nat); init (([1,2,3] . 0) + 1 == 2) -> delta;",
            "init true -> delta;",
            "act a, b; proc P = delta; init hide({a}, P) + (true) -> b;",
            "act a, b; proc P = delta; init rename({a -> b}, P) + (true) -> b;",
        ];

        for text in fixtures {
            let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
            disambiguate_process_specification(&mut spec);
            let once = spec.clone();

            disambiguate_process_specification(&mut spec);
            assert_eq!(
                spec, once,
                "disambiguating an already-disambiguated specification must be a no-op for {text:?}"
            );
        }
    }

    /// As [`disambiguation_is_idempotent`], for [`LONG_GUARDED_CHOICE_CHAIN`] — the fixture
    /// [`long_guarded_choice_chain_with_comments_is_recovered`] also uses — on its own, since it
    /// needs the real `proc`/`init` split rather than one extracted body.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn disambiguation_is_idempotent_for_the_long_guarded_choice_chain() {
        let mut spec = UntypedProcessSpecification::parse(LONG_GUARDED_CHOICE_CHAIN).expect("the fixture should parse");
        disambiguate_process_specification(&mut spec);
        let once = spec.clone();

        disambiguate_process_specification(&mut spec);
        assert_eq!(
            spec, once,
            "disambiguating an already-disambiguated specification must be a no-op"
        );
    }
}
