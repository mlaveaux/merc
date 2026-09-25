use merc_syntax::ActionName;
use merc_syntax::Assignment;
use merc_syntax::CommExpr;
use merc_syntax::DataExpr;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::Rename;
use merc_syntax::Span;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::VarId;

use crate::DataSpecification;
use crate::DisplaySortContext;
use crate::ResolvedName;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking::Scope;
use crate::checking::check_expression_against;
use crate::checking::collect_binder_sorts;
use crate::checking::resolve_single_candidate;
use crate::declared_span;
use crate::typing_info;

use super::ProcessError;
use super::process_specification::DeclarationTables;
use super::process_specification::resolve_declared_sort;

/// Checks a process specification against the declared sorts, returning the
/// merged typing information.
pub(super) fn check_process_specification(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    spec: &UntypedProcessSpecification,
) -> Result<TypingInfo, ProcessError> {
    let mut typing = TypingInfo::default();
    let mut sort_references = Vec::new();

    for decl in &spec.action_declarations {
        for sort in &decl.args {
            typing_info::collect_sort_name_references(sort, &mut sort_references);
        }
    }

    for decl in &spec.process_declarations {
        for param in &decl.params {
            typing_info::collect_sort_name_references(&param.sort, &mut sort_references);
        }
    }

    for decl in &spec.global_variables {
        typing_info::collect_sort_name_references(&decl.sort, &mut sort_references);
    }

    let globals: Vec<(VarId, ResolvedSortId, Span)> = spec
        .global_variables
        .iter()
        .zip(&tables.global_sorts)
        .map(|(decl, &sort)| {
            (
                decl.var_id.expect("resolve_process_variables ran before checking"),
                sort,
                decl.identifier.span.clone(),
            )
        })
        .collect();

    for (decl, &sort) in spec.global_variables.iter().zip(&tables.global_sorts) {
        typing_info::push_binder_declaration(
            data,
            &mut typing,
            decl.identifier.span.clone(),
            decl.identifier.node.clone(),
            sort,
        );
    }

    for (proc_decl, params) in spec.process_declarations.iter().zip(&tables.process_params) {
        let mut scope = globals.clone();
        // A process's own parameters are in scope throughout its body.
        scope.extend(proc_decl.params.iter().zip(params).map(|(decl, &(_, sort))| {
            (
                decl.var_id.expect("resolve_process_variables ran before checking"),
                sort,
                decl.identifier.span.clone(),
            )
        }));

        for (decl, &(_, sort)) in proc_decl.params.iter().zip(params) {
            typing_info::push_binder_declaration(
                data,
                &mut typing,
                decl.identifier.span.clone(),
                decl.identifier.node.clone(),
                sort,
            );
        }

        collect_scope(data, &proc_decl.body, &mut scope, &mut sort_references, &mut typing)?;
        check_process_expr(data, tables, &scope, &proc_decl.body, &mut typing)?;
    }

    if let Some(init) = &spec.init {
        let mut scope = globals.clone();
        collect_scope(data, init, &mut scope, &mut sort_references, &mut typing)?;

        check_process_expr(data, tables, &scope, init, &mut typing)?;
    }

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Collects the scope for a process expression, resolving the declared sorts of
/// all `sum`/`dist` binders.
fn collect_scope(
    data: &mut DataSpecification,
    expr: &ProcessExpr,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    match &expr.node {
        ProcessExprKind::Delta | ProcessExprKind::Tau | ProcessExprKind::Action(..) | ProcessExprKind::Id(..) => Ok(()),
        ProcessExprKind::Sum { variables, operand } => {
            collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
            collect_scope(data, operand, scope, sort_references, typing)
        }
        ProcessExprKind::Dist { variables, operand, .. } => {
            collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
            collect_scope(data, operand, scope, sort_references, typing)
        }
        ProcessExprKind::Binary { lhs, rhs, .. } => {
            collect_scope(data, lhs, scope, sort_references, typing)?;
            collect_scope(data, rhs, scope, sort_references, typing)
        }
        ProcessExprKind::Condition { then, else_, .. } => {
            collect_scope(data, then, scope, sort_references, typing)?;
            match else_ {
                Some(else_) => collect_scope(data, else_, scope, sort_references, typing),
                None => Ok(()),
            }
        }
        ProcessExprKind::At { expr, .. } => collect_scope(data, expr, scope, sort_references, typing),
        ProcessExprKind::Hide { operand, .. }
        | ProcessExprKind::Block { operand, .. }
        | ProcessExprKind::Allow { operand, .. }
        | ProcessExprKind::Comm { operand, .. }
        | ProcessExprKind::Rename { operand, .. } => collect_scope(data, operand, scope, sort_references, typing),
    }
}

fn check_process_expr(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    expr: &ProcessExpr,
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    match &expr.node {
        ProcessExprKind::Delta | ProcessExprKind::Tau => Ok(()),

        ProcessExprKind::Action(name, args) => {
            check_action_or_process(data, tables, scope, name, args, &expr.span, typing)
        }
        ProcessExprKind::Id(name, assignments) => {
            check_instantiation(data, tables, scope, name, assignments, &expr.span, typing)
        }

        ProcessExprKind::Sum { operand, .. } => check_process_expr(data, tables, scope, operand, typing),
        ProcessExprKind::Dist {
            expr: weight, operand, ..
        } => {
            // Checked against `Real`: `dist`'s weight is the distribution's density over its own
            // bound variables, already part of `scope` (collected up front by `collect_scope`).
            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ProcessError>(data, scope, weight, real_sort, typing)?;
            check_process_expr(data, tables, scope, operand, typing)
        }

        ProcessExprKind::Binary { lhs, rhs, .. } => {
            check_process_expr(data, tables, scope, lhs, typing)?;
            check_process_expr(data, tables, scope, rhs, typing)
        }

        ProcessExprKind::Condition { condition, then, else_ } => {
            let bool_sort = data.context().sorts.bool_sort();
            check_expression_against::<ProcessError>(data, scope, condition, bool_sort, typing)?;
            check_process_expr(data, tables, scope, then, typing)?;
            if let Some(else_) = else_ {
                check_process_expr(data, tables, scope, else_, typing)?;
            }
            Ok(())
        }

        ProcessExprKind::At {
            expr: inner,
            operand: time,
        } => {
            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ProcessError>(data, scope, time, real_sort, typing)?;
            check_process_expr(data, tables, scope, inner, typing)
        }

        ProcessExprKind::Hide { actions, operand } => {
            check_action_names(tables, actions, typing)?;
            check_process_expr(data, tables, scope, operand, typing)
        }
        ProcessExprKind::Block { actions, operand } => {
            check_action_names(tables, actions, typing)?;
            check_process_expr(data, tables, scope, operand, typing)
        }
        ProcessExprKind::Allow { actions, operand } => {
            for label in actions {
                check_action_names(tables, &label.actions, typing)?;
            }
            check_process_expr(data, tables, scope, operand, typing)
        }
        ProcessExprKind::Comm { comm, operand } => {
            for c in comm {
                check_action_names(tables, &c.from.actions, typing)?;
                check_action_names(tables, std::slice::from_ref(&c.to), typing)?;
                check_comm_sorts(data, tables, c)?;
            }
            check_process_expr(data, tables, scope, operand, typing)
        }
        ProcessExprKind::Rename { renames, operand } => {
            for r in renames {
                check_action_names(tables, std::slice::from_ref(&r.from), typing)?;
                check_action_names(tables, std::slice::from_ref(&r.to), typing)?;
                check_rename_sorts(data, tables, r)?;
            }
            check_process_expr(data, tables, scope, operand, typing)
        }
    }
}

/// Which declaration table an [`check_action_or_process`] candidate came from, kept alongside its
/// resolved argument domain so the winning candidate's own declaration span can be recovered once
/// exactly one succeeds — `tables.actions.action_domains`/`process_params` alone don't say which
/// table (or index) a flattened candidate list entry belongs to.
enum Candidate {
    Action(usize),
    Process(usize),
}

/// Resolves `name(args)` (an action instance or a positional process instantiation against both declaration tables, trying every
/// candidate of the right arity and requiring exactly one to succeed.
///
/// Each candidate is checked against its own scratch `TypingInfo`, merged into `typing` only once
/// the single successful candidate is known — a failed or ultimately-ambiguous candidate's typing
/// must never reach `typing`, since it would otherwise misreport a sort for the wrong overload at
/// the same span. On success, also pushes a [`ResolvedName::Action`]/[`ResolvedName::Process`] at
/// `name`'s own span (not `span`, the whole `name(args)` node) — the winning candidate identifies
/// exactly which declaration `name` names, the same way `typing_info::resolved_name` already picks
/// a `Constructor`/`Mapping` declaration by its resolved overload.
fn check_action_or_process(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    name: &ActionName,
    args: &[DataExpr],
    span: &Span,
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    let candidates: Vec<(Candidate, Vec<ResolvedSortId>)> = tables
        .actions
        .actions_by_name
        .get(name.as_str())
        .into_iter()
        .flatten()
        .map(|&index| (Candidate::Action(index), tables.actions.action_domains[index].clone()))
        .chain(
            tables
                .processes_by_name
                .get(name.as_str())
                .into_iter()
                .flatten()
                .map(|&index| {
                    (
                        Candidate::Process(index),
                        tables.process_params[index].iter().map(|&(_, sort)| sort).collect(),
                    )
                }),
        )
        .filter(|(_, domain)| domain.len() == args.len())
        .collect();

    if candidates.is_empty() {
        return Err(ProcessError::UndeclaredActionOrProcess {
            name: name.node.clone(),
            arity: args.len(),
            span: span.clone(),
            candidates: tables
                .actions
                .actions_by_name
                .keys()
                .chain(tables.processes_by_name.keys())
                .cloned()
                .collect(),
        });
    }

    let ((candidate, _), mut matched_typing) = resolve_single_candidate(
        &candidates,
        |(_, expected), candidate_typing| check_arguments(data, scope, args, expected, candidate_typing),
        |cause| ProcessError::NoMatchingOverload {
            name: name.node.clone(),
            span: span.clone(),
            cause: Box::new(cause),
        },
        |count| ProcessError::AmbiguousActionOrProcess {
            name: name.node.clone(),
            count,
            span: span.clone(),
        },
    )?;

    let resolved = match candidate {
        Candidate::Action(index) => ResolvedName::Action {
            name: name.node.clone(),
            declaration: declared_span(&tables.actions.action_decl_spans[*index]),
        },
        Candidate::Process(index) => ResolvedName::Process {
            name: name.node.clone(),
            declaration: declared_span(&tables.process_decl_spans[*index]),
        },
    };
    matched_typing.push(name.span.clone(), resolved);
    typing.merge(matched_typing);
    Ok(())
}

fn check_arguments(
    data: &mut DataSpecification,
    scope: &Scope,
    args: &[DataExpr],
    expected: &[ResolvedSortId],
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    for (arg, &sort) in args.iter().zip(expected) {
        check_expression_against::<ProcessError>(data, scope, arg, sort, typing)?;
    }
    Ok(())
}

/// Resolves `P(x = e, ...)` (the assignment form of process instantiation) against every
/// same-named process declaration whose parameters cover every assigned identifier, requiring
/// exactly one to succeed. Unlike [`check_action_or_process`], this is not arity-filtered: an
/// assignment list may legally leave some parameters unassigned.
///
/// Merges only the single successful candidate's `TypingInfo` into `typing`, plus a
/// [`ResolvedName::Process`] at `name`'s own span — see [`check_action_or_process`]'s doc comment.
/// Zero successes is reported the same way too: wrapped in [`ProcessError::NoMatchingOverload`]
/// even for a single candidate, rather than surfacing that candidate's own error bare.
fn check_instantiation(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    name: &ActionName,
    assignments: &[Assignment],
    span: &Span,
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    let indices: &[usize] = tables.processes_by_name.get(name.as_str()).map_or(&[], Vec::as_slice);
    if indices.is_empty() {
        return Err(ProcessError::UndeclaredActionOrProcess {
            name: name.node.clone(),
            arity: assignments.len(),
            span: span.clone(),
            candidates: tables
                .actions
                .actions_by_name
                .keys()
                .chain(tables.processes_by_name.keys())
                .cloned()
                .collect(),
        });
    }

    let mut successes = 0usize;
    let mut first_error = None;
    let mut matched: Option<(usize, TypingInfo)> = None;
    for &index in indices {
        let mut candidate_typing = TypingInfo::default();
        match check_one_instantiation(
            data,
            scope,
            &tables.process_params[index],
            assignments,
            &name.node,
            &mut candidate_typing,
        ) {
            Ok(()) => {
                successes += 1;
                matched = Some((index, candidate_typing));
            }
            Err(error) => drop(first_error.get_or_insert(error)),
        }
    }

    match successes {
        0 => Err(ProcessError::NoMatchingOverload {
            name: name.node.clone(),
            span: span.clone(),
            cause: Box::new(
                first_error.expect("at least one candidate, so at least one recorded error when none succeed"),
            ),
        }),
        1 => {
            let (index, mut matched_typing) = matched.expect("successes == 1 implies a matched candidate");
            matched_typing.push(
                name.span.clone(),
                ResolvedName::Process {
                    name: name.node.clone(),
                    declaration: declared_span(&tables.process_decl_spans[index]),
                },
            );
            typing.merge(matched_typing);
            Ok(())
        }
        count => Err(ProcessError::AmbiguousActionOrProcess {
            name: name.node.clone(),
            count,
            span: span.clone(),
        }),
    }
}

fn check_one_instantiation(
    data: &mut DataSpecification,
    scope: &Scope,
    params: &[(String, ResolvedSortId)],
    assignments: &[Assignment],
    process: &str,
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    let mut assigned: Vec<&str> = Vec::with_capacity(assignments.len());
    for assignment in assignments {
        let Some(&(_, sort)) = params.iter().find(|(param, _)| *param == assignment.identifier) else {
            return Err(ProcessError::UnknownProcessParameter {
                process: process.to_string(),
                name: assignment.identifier.clone(),
                span: assignment.span.clone(),
            });
        };
        if assigned.contains(&assignment.identifier.as_str()) {
            return Err(ProcessError::DuplicateAssignment {
                process: process.to_string(),
                name: assignment.identifier.clone(),
                span: assignment.span.clone(),
            });
        }
        assigned.push(&assignment.identifier);
        check_expression_against::<ProcessError>(data, scope, &assignment.expr, sort, typing)?;
    }
    Ok(())
}

/// Checks that every `names` entry is a declared action, reporting the offending name's own
/// [Span] (rather than the enclosing expression's), since each [ActionName] carries one. On
/// success, also pushes a [`ResolvedName::ActionSet`] at each name's own span: unlike
/// [`check_action_or_process`], there is no argument list here to narrow an overloaded name down
/// to one declaration, so every declaration sharing the name is offered.
fn check_action_names(
    tables: &DeclarationTables,
    names: &[ActionName],
    typing: &mut TypingInfo,
) -> Result<(), ProcessError> {
    for name in names {
        let Some(indices) = tables.actions.actions_by_name.get(name.as_str()) else {
            return Err(ProcessError::UndeclaredAction {
                name: name.node.clone(),
                span: name.span.clone(),
                candidates: tables.actions.actions_by_name.keys().cloned().collect(),
            });
        };

        let declarations = indices
            .iter()
            .filter_map(|&index| declared_span(&tables.actions.action_decl_spans[index]))
            .collect();
        typing.push(
            name.span.clone(),
            ResolvedName::ActionSet {
                name: name.node.clone(),
                declarations,
            },
        );
    }
    Ok(())
}

/// Checks that `comm`'s combined actions have a jointly compatible sort.
///
/// Called after [`check_action_names`] has already confirmed every name `comm` mentions is
/// declared, so every name here has at least one candidate overload.
fn check_comm_sorts(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    comm: &CommExpr,
) -> Result<(), ProcessError> {
    let from_options: Vec<&[usize]> = comm
        .from
        .actions
        .iter()
        .map(|name| {
            tables
                .actions
                .actions_by_name
                .get(name.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[])
        })
        .collect();
    let to_options: &[usize] = tables
        .actions
        .actions_by_name
        .get(comm.to.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut chosen = Vec::with_capacity(from_options.len());
    let mut reason = None;
    if try_from_overloads(data, tables, &from_options, &mut chosen, to_options, &mut reason) {
        return Ok(());
    }

    Err(ProcessError::IncompatibleCommunication {
        lhs: comm
            .from
            .actions
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join("|"),
        result: comm.to.node.clone(),
        reason: reason.unwrap_or_else(|| "no declared overload combination matches".to_string()),
        span: comm.to.span.clone(),
    })
}

/// Backtracks over every combination of one overload per left-hand action name.
fn try_from_overloads(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    remaining: &[&[usize]],
    chosen: &mut Vec<usize>,
    to_options: &[usize],
    reason: &mut Option<String>,
) -> bool {
    let Some((options, rest)) = remaining.split_first() else {
        for &to_index in to_options {
            match combined_sort_matches(data, tables, chosen, to_index) {
                Ok(()) => return true,
                Err(message) => {
                    reason.get_or_insert(message);
                }
            }
        }
        return false;
    };

    for &index in *options {
        chosen.push(index);
        let matched = try_from_overloads(data, tables, rest, chosen, to_options, reason);
        chosen.pop();
        if matched {
            return true;
        }
    }
    false
}

/// The `rename` counterpart of [`check_comm_sorts`].
fn check_rename_sorts(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    rename: &Rename,
) -> Result<(), ProcessError> {
    let from_options: &[usize] = tables
        .actions
        .actions_by_name
        .get(rename.from.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let to_options: &[usize] = tables
        .actions
        .actions_by_name
        .get(rename.to.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut reason = None;
    for &from_index in from_options {
        for &to_index in to_options {
            match combined_sort_matches(data, tables, std::slice::from_ref(&from_index), to_index) {
                Ok(()) => return Ok(()),
                Err(message) => {
                    reason.get_or_insert(message);
                }
            }
        }
    }

    Err(ProcessError::IncompatibleRename {
        from: rename.from.node.clone(),
        to: rename.to.node.clone(),
        reason: reason.unwrap_or_else(|| "no declared overload combination matches".to_string()),
        span: rename.to.span.clone(),
    })
}

/// Checks one concrete choice of overloads.
fn combined_sort_matches(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    from_indices: &[usize],
    to_index: usize,
) -> Result<(), String> {
    let to_domain = &tables.actions.action_domains[to_index];
    let arity = to_domain.len();
    if let Some(&mismatched) = from_indices
        .iter()
        .find(|&&index| tables.actions.action_domains[index].len() != arity)
    {
        return Err(format!(
            "one action takes {} parameter(s), another takes {arity}",
            tables.actions.action_domains[mismatched].len(),
        ));
    }

    let mut failure: Option<(usize, ResolvedSortId, ResolvedSortId)> = None;
    {
        let (ctx, _) = data.context_and_specs_mut();
        'positions: for (position, &expected) in to_domain.iter().enumerate() {
            let mut joined = tables.actions.action_domains[from_indices[0]][position];
            for &index in &from_indices[1..] {
                let candidate = tables.actions.action_domains[index][position];
                let next = ctx.sorts.join(joined, candidate).filter(|&sort| {
                    ctx.sorts.is_materializable(joined, sort) && ctx.sorts.is_materializable(candidate, sort)
                });
                joined = match next {
                    Some(sort) => sort,
                    None => {
                        failure = Some((position, joined, candidate));
                        break 'positions;
                    }
                };
            }
            if !ctx.sorts.is_materializable(joined, expected) {
                failure = Some((position, joined, expected));
                break;
            }
        }
    }

    match failure {
        None => Ok(()),
        Some((position, lhs, rhs)) => {
            let (ctx, spec) = data.context_and_specs_mut();
            Err(format!(
                "parameter {} has sort '{}', which is not compatible with '{}'",
                position + 1,
                DisplaySortContext::new(ctx, spec, lhs),
                DisplaySortContext::new(ctx, spec, rhs),
            ))
        }
    }
}

#[cfg(test)]
mod stack_depth_probe {
    //! Isolates `check_process_expr`'s own recursion from the parser's: the tree here is built
    //! directly, so a stack overflow can only come from this module's own walk.
    use merc_syntax::ProcessExprKind;
    use merc_syntax::Span;
    use merc_syntax::UntypedDataSpecification;

    use super::*;
    use crate::checking::ActionTable;
    use crate::process::process_specification::DeclarationTables;

    fn deep_hide(depth: usize) -> ProcessExpr {
        let mut expr = ProcessExprKind::Delta.spanned(Span::default());
        for _ in 0..depth {
            expr = ProcessExprKind::Hide {
                actions: Vec::new(),
                operand: Box::new(expr),
            }
            .spanned(Span::default());
        }
        expr
    }

    #[test]
    fn deeply_nested_hide_does_not_overflow_the_stack() {
        let mut data = DataSpecification::from_untyped(UntypedDataSpecification::parse("").unwrap()).unwrap();
        let actions: ActionTable = ActionTable::build(&mut data, &[], resolve_declared_sort).unwrap();
        let tables = DeclarationTables {
            global_sorts: Vec::new(),
            process_params: Vec::new(),
            process_decl_spans: Vec::new(),
            processes_by_name: Default::default(),
            actions,
        };
        let mut typing = TypingInfo::default();
        let scope: Vec<(VarId, ResolvedSortId, Span)> = Vec::new();

        // 100,000 nested `hide({}, ...)`s, exactly as `deeply_nested_negation_does_not_overflow_the_stack`
        // does for `modal::check::check_state_formula`.
        let expr = deep_hide(100_000);
        let result = check_process_expr(&mut data, &tables, &scope, &expr, &mut typing);
        assert!(result.is_ok());
    }
}
