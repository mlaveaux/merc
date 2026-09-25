// The scoped walk over the state formula, checking each `val(...)` expression, action instance and
// fixpoint-variable reference. See `ValSort` for what a state-level `val`'s declared sort allows.

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
use crate::checking::resolve_single_candidate;
use crate::declared_span;
use crate::typing_info;

use super::ModalError;
use super::modal_specification::DeclarationTables;
use super::modal_specification::FormulaType;
use super::modal_specification::resolve_declared_sort;

/// One fixpoint variable currently in scope, keyed by the [`StateVarId`] `resolve_modal_variables`
/// assigned to its declaration rather than by name.
type StateVarStack = Vec<(StateVarId, Span, Vec<ResolvedSortId>)>;

/// Checks a state formula specification against the declared sorts.
pub(super) fn check_modal_specification(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    spec: &UntypedStateFrmSpec,
    formula_type: FormulaType,
) -> Result<TypingInfo, ModalError> {
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
    check_state_formula(
        data,
        tables,
        &scope,
        &mut state_vars,
        &spec.formula,
        formula_type,
        &mut typing,
    )?;

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Collects the scope for a state formula, resolving the declared sorts of every
/// `forall`/`exists`/`inf`/`sup`/`sum` binder and of every fixpoint variable's own parameters —
/// the latter are ordinary data variables throughout the fixpoint's body, exactly like a PRES
/// equation's parameters.
///
/// The fixpoint variable *names* are not collected here; they live on their own stack, maintained
/// by [`check_fixed_point`] as the walk enters and leaves each binder.
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
        ActFrmKind::At { expr, .. } => collect_scope_actfrm(data, expr, scope, sort_references, typing),
    }
}

#[allow(clippy::too_many_arguments)]
fn check_state_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    state_vars: &mut StateVarStack,
    formula: &StateFrm,
    formula_type: FormulaType,
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

        StateFrmKind::DataValExpr(data_expr) => check_val_expr(data, scope, data_expr, formula_type, typing),

        StateFrmKind::DataValExprLeftMult(constant, expr) | StateFrmKind::DataValExprRightMult(expr, constant) => {
            if formula_type == FormulaType::Bool {
                return Err(ModalError::ConstantMultiplyInBooleanFormula {
                    span: formula.span.clone(),
                });
            }

            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ModalError>(data, scope, constant, real_sort, typing)?;
            check_state_formula(data, tables, scope, state_vars, expr, formula_type, typing)
        }

        StateFrmKind::Modality { formula: reg, expr, .. } => {
            check_reg_formula(data, tables, scope, reg, typing)?;
            check_state_formula(data, tables, scope, state_vars, expr, formula_type, typing)
        }

        StateFrmKind::Unary { expr, .. } => {
            check_state_formula(data, tables, scope, state_vars, expr, formula_type, typing)
        }

        StateFrmKind::Binary { lhs, rhs, .. } => {
            check_state_formula(data, tables, scope, state_vars, lhs, formula_type, typing)?;
            check_state_formula(data, tables, scope, state_vars, rhs, formula_type, typing)
        }

        StateFrmKind::Quantifier { body, .. } | StateFrmKind::Bound { body, .. } => {
            check_state_formula(data, tables, scope, state_vars, body, formula_type, typing)
        }

        StateFrmKind::FixedPoint { variable, body, .. } => {
            check_fixed_point(data, tables, scope, state_vars, variable, body, formula_type, typing)
        }
    }
}

/// Type-checks a state-formula-level `val(...)` occurrence against the declared `formula_type`.
fn check_val_expr(
    data: &mut DataSpecification,
    scope: &Scope,
    data_expr: &DataExpr,
    formula_type: FormulaType,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let bool_sort = data.context().sorts.bool_sort();

    if formula_type == FormulaType::Bool {
        return check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, typing);
    }

    let real_sort = data.context().sorts.real_sort();
    let mut real_typing = TypingInfo::default();
    match check_expression_against::<ModalError>(data, scope, data_expr, real_sort, &mut real_typing) {
        Ok(()) => {
            typing.merge(real_typing);
            Ok(())
        }
        Err(real_cause) => {
            let mut bool_typing = TypingInfo::default();
            match check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, &mut bool_typing) {
                Ok(()) => {
                    typing.merge(bool_typing);
                    Ok(())
                }
                Err(bool_cause) => Err(ModalError::NoMatchingValSort {
                    span: data_expr.span.clone(),
                    real_cause: Box::new(real_cause),
                    bool_cause: Box::new(bool_cause),
                }),
            }
        }
    }
}

/// Checks a fixpoint variable's own declaration: each parameter's initial value against its
/// declared sort, in the *outer* scope since the parameter it initializes isn't bound yet. The
/// variable is then pushed onto `state_vars` for `body` to reference recursively, and popped again
/// afterwards so an enclosing formula never sees an inner fixpoint's variable.
#[allow(clippy::too_many_arguments)]
fn check_fixed_point(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    state_vars: &mut StateVarStack,
    variable: &StateVarDecl,
    body: &StateFrm,
    formula_type: FormulaType,
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
    let result = check_state_formula(data, tables, scope, state_vars, body, formula_type, typing);
    state_vars.pop();
    result
}

/// Type-checks an already-[`resolved`](StateFrmKind::Resolved) `name(args)` reference against its
/// enclosing fixpoint variable's declared parameter sorts, found in `state_vars` by matching
/// `declaration` rather than `name`, since shadowing is already resolved into the [`StateVarId`]
/// this occurrence carries. On success, also pushes a [`ResolvedName::StateVariable`] at `span`,
/// the whole `name(args)` node: the name itself is not separately spanned in the syntax tree.
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
    // `StateVarId`s are unique, so at most one entry can match; no need to search from the top for
    // the innermost shadowing declaration the way a name-keyed lookup would.
    let (_, decl_span, params) = state_vars.iter().find(|(id, _, _)| *id == declaration).expect(
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

        // Like `StateFrmKind::Delay`/`Yaled`'s own `@`-time argument, `operand` is checked against
        // `Real` regardless of `expr`'s own sort — there's no `ValSort`-style ambiguity here since
        // an action formula's `val(...)` is always `Bool` (see `ValSort`'s doc comment).
        ActFrmKind::At { expr, operand } => {
            check_action_formula(data, tables, scope, expr, typing)?;
            let real_sort = data.context().sorts.real_sort();
            check_expression_against::<ModalError>(data, scope, operand, real_sort, typing)
        }
    }
}

/// Resolves one action instance inside a multi-action against the `act` table, trying every
/// same-named overload of the right arity and requiring exactly one to succeed — the action-only
/// counterpart of `crate::process::check::check_action_or_process`, as no process table applies to
/// a state formula's modalities.
///
/// Each candidate is checked against its own scratch `TypingInfo`, merged into `typing` only once
/// the single successful candidate is known: a failed or ambiguous candidate's typing must never
/// reach `typing`, or it would misreport a sort for the wrong overload at the same span. On
/// success, also pushes a [`ResolvedName::Action`] at `action.id`'s own span.
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
        let mut candidates: Vec<String> = tables.actions_by_name.keys().cloned().collect();
        candidates.sort();
        return Err(ModalError::UndeclaredAction {
            name: action.id.node.clone(),
            arity: action.args.len(),
            span: action.id.span.clone(),
            candidates,
        });
    }

    let (&index, mut matched_typing) = resolve_single_candidate(
        &candidates,
        |&index, candidate_typing| {
            check_action_arguments(
                data,
                scope,
                &action.args,
                &tables.action_domains[index],
                candidate_typing,
            )
        },
        |cause| ModalError::NoMatchingOverload {
            name: action.id.node.clone(),
            span: action.id.span.clone(),
            cause: Box::new(cause),
        },
        |count| ModalError::AmbiguousAction {
            name: action.id.node.clone(),
            count,
            span: action.id.span.clone(),
        },
    )?;

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

#[cfg(test)]
mod stack_depth_probe {
    //! Isolates `check_state_formula`'s own recursion from the parser's and from
    //! `resolve_modal_variables`': the tree here is built directly, bypassing both, so a stack
    //! overflow can only come from this module's own `check_state_formula`/`collect_scope` walk.
    use merc_syntax::Span;
    use merc_syntax::StateFrmKind;
    use merc_syntax::StateFrmUnaryOp;
    use merc_syntax::UntypedDataSpecification;

    use super::*;
    use crate::checking::ActionTable;

    fn deep_negation(depth: usize) -> StateFrm {
        let mut formula = StateFrmKind::True.spanned(Span::default());
        for _ in 0..depth {
            formula = StateFrmKind::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(formula),
            }
            .spanned(Span::default());
        }
        formula
    }

    #[test]
    fn deeply_nested_negation_does_not_overflow_the_stack() {
        let mut data = DataSpecification::from_untyped(UntypedDataSpecification::parse("").unwrap()).unwrap();
        let tables: ActionTable = ActionTable::build(&mut data, &[], resolve_declared_sort).unwrap();
        let mut typing = TypingInfo::default();
        let mut state_vars = StateVarStack::new();
        let scope: Vec<(VarId, ResolvedSortId, Span)> = Vec::new();

        // 100,000 nested `!`s: well within what a real, if pathological, `.mcf` file could contain
        // (the parser itself already overflows at a similar depth — a separate, out-of-scope
        // concern), but small enough that a bounded-recursion walk would handle it trivially.
        let formula = deep_negation(100_000);
        let result = check_state_formula(
            &mut data,
            &tables,
            &scope,
            &mut state_vars,
            &formula,
            FormulaType::Bool,
            &mut typing,
        );
        assert!(result.is_ok());
    }
}
