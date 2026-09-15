//! The scoped walk over the state formula: checks each `val(...)` expression.
//!
//! At the action-formula level (nested inside a `<...>`/`[...]` modality) a `val` is always
//! `Bool`. At the state-formula level a `val` can be either `Real` — combined via
//! `DataValExprLeftMult`/`DataValExprRightMult` into a PRES-style quantitative formula — or
//! `Bool`, a plain mu-calculus atom. Which one applies isn't declared anywhere, so the first
//! state-level `val(...)` the checker reaches tries `Real` then `Bool` and fixates the whole
//! formula's [`ValSort`] to whichever matches; every later `val(...)` is then held to that same
//! sort, so a formula can't mix the two. See `check_val_expr`.
//!
//! To resolve a state variable's sort, the checker uses the `state_vars` stack,
//! which pairs each fixpoint variable's own [`StateVarId`] with its declaring
//! span (for reporting) and its declared parameter sorts.

use std::collections::HashSet;

use merc_syntax::ActFrm;
use merc_syntax::ActFrmKind;
use merc_syntax::Action;
use merc_syntax::DataExpr;
use merc_syntax::RegFrm;
use merc_syntax::RegFrmKind;
use merc_syntax::Span;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmKind;
use merc_syntax::StateVarDecl;
use merc_syntax::StateVarId;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::VarId;

use crate::DataSpecification;
use crate::ResolvedName;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking::Scope;
use crate::checking::check_expression_against;
use crate::checking::collect_binder_sorts;
use crate::declared_span;
use crate::typing_info;

use super::ModalError;
use super::modal_specification::DeclarationTables;
use super::modal_specification::ValSort;
use super::modal_specification::resolve_declared_sort;

/// One fixpoint variable currently in scope: its own [`StateVarId`] — matching a
/// `StateFrmKind::Resolved` occurrence's own declaration field, assigned by
/// `resolve_modal_variables` — paired with its declaring `StateVarDecl`'s own span (kept only for
/// [`ResolvedName::StateVariable::declaration`], the same way `ConstructorId`/`MapId` keep a
/// separately-derived span alongside their id) and its declared parameter sorts (in order).
type StateVarStack = Vec<(StateVarId, Span, Vec<ResolvedSortId>)>;

/// Checks a state formula specification against the declared sorts, returning the merged typing
/// information together with the [`ValSort`] the formula's `val(...)` occurrences fixed on (see
/// this module's doc comment).
pub(super) fn check_modal_specification(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    spec: &UntypedStateFrmSpec,
) -> Result<(TypingInfo, ValSort), ModalError> {
    let mut typing = TypingInfo::default();
    let mut sort_references = Vec::new();

    for decl in &spec.action_declarations {
        for sort in &decl.args {
            typing_info::collect_sort_name_references(sort, &mut sort_references);
        }
    }

    let mut scope = Vec::new();
    collect_scope(data, &spec.formula, &mut scope, &mut sort_references, &mut typing)?;

    let mut state_vars = StateVarStack::new();
    let mut val_sort = ValSort::Unknown;
    check_state_formula(
        data,
        tables,
        &scope,
        &mut state_vars,
        &spec.formula,
        &mut val_sort,
        &mut typing,
    )?;

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok((typing, val_sort))
}

/// Collects the scope for a state formula, resolving the declared sorts of every
/// `forall`/`exists`/`inf`/`sup`/`sum` binder and every fixpoint variable's own parameters — the
/// latter are ordinary data variables throughout the fixpoint's body, exactly like a PRES
/// equation's own parameters (see `crate::pres::check::check_pres_specification`).
///
/// Does *not* collect anything about the fixpoint variable *name* itself — that is resolved
/// lexically by [`check_state_formula`] instead, see this module's doc comment.
fn collect_scope(
    data: &mut DataSpecification,
    formula: &StateFrm,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        StateFrmKind::True
        | StateFrmKind::False
        | StateFrmKind::Delay(_)
        | StateFrmKind::Yaled(_)
        | StateFrmKind::Id(_, _)
        | StateFrmKind::Resolved(_, _, _)
        | StateFrmKind::DataValExpr(_) => Ok(()),
        StateFrmKind::DataValExprLeftMult(_, expr) | StateFrmKind::DataValExprRightMult(expr, _) => {
            collect_scope(data, expr, scope, sort_references, typing)
        }
        StateFrmKind::Modality { formula, expr, .. } => {
            collect_scope_regfrm(data, formula, scope, sort_references, typing)?;
            collect_scope(data, expr, scope, sort_references, typing)
        }
        StateFrmKind::Unary { expr, .. } => collect_scope(data, expr, scope, sort_references, typing),
        StateFrmKind::Binary { lhs, rhs, .. } => {
            collect_scope(data, lhs, scope, sort_references, typing)?;
            collect_scope(data, rhs, scope, sort_references, typing)
        }
        StateFrmKind::Quantifier { variables, body, .. } | StateFrmKind::Bound { variables, body, .. } => {
            collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
            collect_scope(data, body, scope, sort_references, typing)
        }
        StateFrmKind::FixedPoint { variable, body, .. } => {
            for argument in &variable.arguments {
                typing_info::collect_sort_name_references(&argument.sort, sort_references);
                let sort = resolve_declared_sort(data, &argument.sort)?;
                typing_info::push_binder_declaration(
                    data,
                    typing,
                    argument.identifier.span.clone(),
                    argument.identifier.node.clone(),
                    sort,
                );
                let var_id = argument.id.expect("resolve_modal_variables ran before checking");
                scope.push((var_id, sort, argument.identifier.span.clone()));
            }
            collect_scope(data, body, scope, sort_references, typing)
        }
    }
}

fn collect_scope_regfrm(
    data: &mut DataSpecification,
    formula: &RegFrm,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        RegFrmKind::Action(action) => collect_scope_actfrm(data, action, scope, sort_references, typing),
        RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => {
            collect_scope_regfrm(data, inner, scope, sort_references, typing)
        }
        RegFrmKind::Sequence { lhs, rhs } | RegFrmKind::Choice { lhs, rhs } => {
            collect_scope_regfrm(data, lhs, scope, sort_references, typing)?;
            collect_scope_regfrm(data, rhs, scope, sort_references, typing)
        }
    }
}

fn collect_scope_actfrm(
    data: &mut DataSpecification,
    formula: &ActFrm,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        ActFrmKind::True | ActFrmKind::False | ActFrmKind::MultAct(_) | ActFrmKind::DataExprVal(_) => Ok(()),
        ActFrmKind::Negation(inner) => collect_scope_actfrm(data, inner, scope, sort_references, typing),
        ActFrmKind::Quantifier { variables, body, .. } => {
            collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
            collect_scope_actfrm(data, body, scope, sort_references, typing)
        }
        ActFrmKind::Binary { lhs, rhs, .. } => {
            collect_scope_actfrm(data, lhs, scope, sort_references, typing)?;
            collect_scope_actfrm(data, rhs, scope, sort_references, typing)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_state_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    state_vars: &mut StateVarStack,
    formula: &StateFrm,
    val_sort: &mut ValSort,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        StateFrmKind::True | StateFrmKind::False => Ok(()),

        StateFrmKind::Delay(time) | StateFrmKind::Yaled(time) => match time {
            Some(time) => {
                let real_sort = data.context().sorts.real_sort();
                check_expression_against::<ModalError>(data, scope, time, real_sort, typing)
            }
            None => Ok(()),
        },

        // `resolve_modal_variables` runs before checking and rewrites every `Id` naming an
        // enclosing `mu`/`nu` into `Resolved`; one surviving here refers to no enclosing binder.
        StateFrmKind::Id(name, _arguments) => Err(ModalError::UndeclaredStateVariable {
            name: name.to_string(),
            span: formula.span.clone(),
        }),

        StateFrmKind::Resolved(name, arguments, declaration) => check_state_var_inst(
            data,
            state_vars,
            scope,
            name,
            arguments,
            *declaration,
            &formula.span,
            typing,
        ),

        StateFrmKind::DataValExpr(data_expr) => check_val_expr(data, scope, data_expr, val_sort, typing),

        StateFrmKind::DataValExprLeftMult(constant, expr) | StateFrmKind::DataValExprRightMult(expr, constant) => {
            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ModalError>(data, scope, constant, real_sort, typing)?;
            check_state_formula(data, tables, scope, state_vars, expr, val_sort, typing)
        }

        StateFrmKind::Modality { formula: reg, expr, .. } => {
            check_reg_formula(data, tables, scope, reg, typing)?;
            check_state_formula(data, tables, scope, state_vars, expr, val_sort, typing)
        }

        StateFrmKind::Unary { expr, .. } => {
            check_state_formula(data, tables, scope, state_vars, expr, val_sort, typing)
        }

        StateFrmKind::Binary { lhs, rhs, .. } => {
            check_state_formula(data, tables, scope, state_vars, lhs, val_sort, typing)?;
            check_state_formula(data, tables, scope, state_vars, rhs, val_sort, typing)
        }

        StateFrmKind::Quantifier { body, .. } | StateFrmKind::Bound { body, .. } => {
            check_state_formula(data, tables, scope, state_vars, body, val_sort, typing)
        }

        StateFrmKind::FixedPoint { variable, body, .. } => {
            check_fixed_point(data, tables, scope, state_vars, variable, body, val_sort, typing)
        }
    }
}

/// Type-checks a state-formula-level `val(...)` occurrence. On the first one reached
/// (`*val_sort == ValSort::Unknown`), tries `Real` then `Bool`, fixating `val_sort` to whichever
/// sort the expression actually type-checks against; every `val(...)` reached afterward — once
/// `val_sort` is no longer `Unknown` — is held to that same sort. See the module doc comment above
/// for why this is necessary.
fn check_val_expr(
    data: &mut DataSpecification,
    scope: &Scope,
    data_expr: &DataExpr,
    val_sort: &mut ValSort,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match *val_sort {
        ValSort::Real => {
            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ModalError>(data, scope, data_expr, real_sort, typing)
        }
        ValSort::Bool => {
            let bool_sort = data.context().sorts.bool_sort();
            check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, typing)
        }
        ValSort::Unknown => {
            let real_sort = data.context().sorts.real_sort();
            match check_expression_against::<ModalError>(data, scope, data_expr, real_sort, typing) {
                Ok(()) => {
                    *val_sort = ValSort::Real;
                    Ok(())
                }
                // `check_expression_against` never touches `typing` before returning an error
                // (it fails inside `infer_expression_in_scope`, before the merge), so retrying
                // against `Bool` here does not need a scratch `TypingInfo` to undo the first try.
                Err(real_error) => {
                    let bool_sort = data.context().sorts.bool_sort();
                    match check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, typing) {
                        Ok(()) => {
                            *val_sort = ValSort::Bool;
                            Ok(())
                        }
                        Err(_) => Err(real_error),
                    }
                }
            }
        }
    }
}

/// Checks a fixpoint variable's own declaration — each parameter's initial value against its
/// declared sort, checked in the *outer* scope since the parameter it initializes isn't bound yet
/// (mirrors a process instantiation's assignment value, `crate::process::check::check_one_instantiation`)
/// — then pushes it onto `state_vars` for `body` to reference recursively, popping it again once
/// `body` is checked so an enclosing formula never sees an inner fixpoint's own variable.
#[allow(clippy::too_many_arguments)]
fn check_fixed_point(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    state_vars: &mut StateVarStack,
    variable: &StateVarDecl,
    body: &StateFrm,
    val_sort: &mut ValSort,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let mut seen = HashSet::new();
    let mut params = Vec::with_capacity(variable.arguments.len());
    for argument in &variable.arguments {
        if !seen.insert(argument.identifier.as_str()) {
            return Err(ModalError::DuplicateFixedPointParameter {
                variable: variable.identifier.clone(),
                name: argument.identifier.node.clone(),
                span: argument.identifier.span.clone(),
            });
        }
        let sort = resolve_declared_sort(data, &argument.sort)?;
        check_expression_against::<ModalError>(data, scope, &argument.expr, sort, typing)?;
        params.push(sort);
    }

    let state_var_id = variable.id.expect("resolve_modal_variables ran before checking");
    state_vars.push((state_var_id, variable.span.clone(), params));
    let result = check_state_formula(data, tables, scope, state_vars, body, val_sort, typing);
    state_vars.pop();
    result
}

/// Type-checks an already-[`resolved`](StateFrmKind::Resolved) `name(args)` reference against its
/// enclosing fixpoint variable's declared parameter sorts, found in `state_vars` by matching
/// `declaration` — not `name`: shadowing is already resolved, by `resolve_modal_variables`, into
/// the exact declaring [`StateVarId`] this occurrence carries. Checks the argument count
/// (`ArityMismatch`) and each argument against its parameter's sort. On success, also pushes a
/// [`ResolvedName::StateVariable`] at `span` (the whole `name(args)`/bare `name` node — see
/// `StateFrmKind::Id`'s doc comment for why there is no narrower span available here).
fn check_state_var_inst(
    data: &mut DataSpecification,
    state_vars: &StateVarStack,
    scope: &Scope,
    name: &str,
    arguments: &[DataExpr],
    declaration: StateVarId,
    span: &Span,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let (_, decl_span, params) = state_vars.iter().rev().find(|(id, _, _)| *id == declaration).expect(
        "a `StateFrmKind::Resolved` occurrence's declaration always matches an enclosing \
             `FixedPoint` pushed onto `state_vars` by `check_fixed_point`, since \
             `resolve_modal_variables` only ever resolves a name against a genuinely enclosing \
             binder",
    );
    typing.push(
        span.clone(),
        ResolvedName::StateVariable {
            name: name.to_string(),
            declaration: declared_span(decl_span),
        },
    );

    if arguments.len() != params.len() {
        return Err(ModalError::ArityMismatch {
            name: name.to_string(),
            expected: params.len(),
            found: arguments.len(),
            span: span.clone(),
        });
    }

    for (argument, &sort) in arguments.iter().zip(params) {
        check_expression_against::<ModalError>(data, scope, argument, sort, typing)?;
    }
    Ok(())
}

fn check_reg_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    formula: &RegFrm,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        RegFrmKind::Action(action) => check_action_formula(data, tables, scope, action, typing),
        RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => check_reg_formula(data, tables, scope, inner, typing),
        RegFrmKind::Sequence { lhs, rhs } | RegFrmKind::Choice { lhs, rhs } => {
            check_reg_formula(data, tables, scope, lhs, typing)?;
            check_reg_formula(data, tables, scope, rhs, typing)
        }
    }
}

fn check_action_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    formula: &ActFrm,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    match &formula.node {
        ActFrmKind::True | ActFrmKind::False => Ok(()),

        ActFrmKind::MultAct(multi_action) => {
            for action in &multi_action.actions {
                check_action(data, tables, scope, action, typing)?;
            }
            Ok(())
        }

        ActFrmKind::DataExprVal(data_expr) => {
            let bool_sort = data.context().sorts.bool_sort();
            check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, typing)
        }

        ActFrmKind::Negation(inner) => check_action_formula(data, tables, scope, inner, typing),

        ActFrmKind::Quantifier { body, .. } => check_action_formula(data, tables, scope, body, typing),

        ActFrmKind::Binary { lhs, rhs, .. } => {
            check_action_formula(data, tables, scope, lhs, typing)?;
            check_action_formula(data, tables, scope, rhs, typing)
        }
    }
}

/// Resolves one action instance inside a multi-action against the `act` table, trying every
/// same-named overload of the right arity and requiring exactly one to succeed — the action-only
/// counterpart of `crate::process::check::check_action_or_process` (no process table applies to a
/// state formula's modalities).
///
/// Each candidate is checked against its own scratch `TypingInfo`, merged into `typing` only once
/// the single successful candidate is known, exactly as `check_action_or_process` does. On success,
/// also pushes a [`ResolvedName::Action`] at `action.id`'s own span.
fn check_action(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    action: &Action,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let candidates: Vec<usize> = tables
        .actions_by_name
        .get(action.id.as_str())
        .into_iter()
        .flatten()
        .copied()
        .filter(|&index| tables.action_domains[index].len() == action.args.len())
        .collect();

    if candidates.is_empty() {
        return Err(ModalError::UndeclaredAction {
            name: action.id.node.clone(),
            arity: action.args.len(),
            span: action.id.span.clone(),
            candidates: tables.actions_by_name.keys().cloned().collect(),
        });
    }

    let mut successes = 0usize;
    let mut first_error = None;
    let mut matched: Option<(usize, TypingInfo)> = None;
    for &index in &candidates {
        let mut candidate_typing = TypingInfo::default();
        match check_action_arguments(
            data,
            scope,
            &action.args,
            &tables.action_domains[index],
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
        0 => Err(ModalError::NoMatchingOverload {
            name: action.id.node.clone(),
            span: action.id.span.clone(),
            cause: Box::new(
                first_error.expect("at least one candidate, so at least one recorded error when none succeed"),
            ),
        }),
        1 => {
            let (index, mut matched_typing) = matched.expect("successes == 1 implies a matched candidate");
            matched_typing.push(
                action.id.span.clone(),
                ResolvedName::Action {
                    name: action.id.node.clone(),
                    declaration: declared_span(&tables.action_decl_spans[index]),
                },
            );
            typing.merge(matched_typing);
            Ok(())
        }
        count => Err(ModalError::AmbiguousAction {
            name: action.id.node.clone(),
            count,
            span: action.id.span.clone(),
        }),
    }
}

fn check_action_arguments(
    data: &mut DataSpecification,
    scope: &Scope,
    args: &[DataExpr],
    expected: &[ResolvedSortId],
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    for (arg, &sort) in args.iter().zip(expected) {
        check_expression_against::<ModalError>(data, scope, arg, sort, typing)?;
    }
    Ok(())
}
