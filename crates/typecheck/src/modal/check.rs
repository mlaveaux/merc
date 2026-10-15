//! The scoped walk over the state formula, checking each `val(...)` expression, action instance and
//! fixpoint-variable reference. See `ValSort` for what a state-level `val`'s declared sort allows.

use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_syntax::ActFrmKind;
use merc_syntax::Action;
use merc_syntax::DataExpr;
use merc_syntax::MixedNode;
use merc_syntax::RegFrm;
use merc_syntax::Span;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmKind;
use merc_syntax::StateVarDecl;
use merc_syntax::StateVarId;
use merc_syntax::Traverse;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::VarId;

use crate::DataSpecification;
use crate::ResolvedName;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking::Scope;
use crate::checking::check_expression_against;
use crate::checking::collect_binder_sorts;
use crate::checking::declare_variables;
use crate::checking::resolve_single_candidate;
use crate::declared_span;
use crate::typing_info;

use super::ModalError;
use super::modal_specification::ActionChecking;
use super::modal_specification::DeclarationTables;
use super::modal_specification::FormulaType;
use super::modal_specification::resolve_declared_sort;

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

    // One scope, shared by every equation, built from the equation system directly.
    let mut scope = Vec::new();
    for index in 0..tables.equation_system.len() {
        let equation = tables.equation_system.equation(index);
        let params = &tables.equation_params[index];

        let arguments = &equation.variable().arguments;
        for argument in arguments {
            typing_info::collect_sort_name_references(&argument.decl.sort, &mut sort_references);
        }
        let declarations = arguments.iter().map(|argument| &argument.decl);
        let sorts = params.iter().map(|&(_, sort)| sort);
        declare_variables(data, &mut scope, &mut typing, declarations, sorts);

        collect_scope(data, equation.body(), &mut scope, &mut sort_references, &mut typing)?;
    }

    for index in 0..tables.equation_system.len() {
        let equation = tables.equation_system.equation(index);
        check_fixed_point_declaration(
            data,
            &scope,
            equation.variable(),
            &tables.equation_params[index],
            &mut typing,
        )?;
        check_state_formula(data, tables, &scope, equation.body(), formula_type, &mut typing)?;
    }

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Collects the scope for one equation body: the declared sorts of every
/// `forall`/`exists`/`inf`/`sup`/`sum` binder in it, or in any action formula nested inside it.
fn collect_scope(
    data: &mut DataSpecification,
    formula: &StateFrm,
    scope: &mut Vec<(VarId, ResolvedSortId)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    formula
        .try_visit_mixed::<Infallible, ModalError>(|node| {
            match node {
                MixedNode::StateFrm(formula) => match &formula.node {
                    StateFrmKind::Quantifier { variables, .. } | StateFrmKind::Bound { variables, .. } => {
                        collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
                    }
                    StateFrmKind::FixedPoint { .. } => unreachable!(
                        "ModalEquationSystem extracts every FixedPoint into its own equation; \
                         an equation's own body never contains one"
                    ),
                    _ => {}
                },
                MixedNode::ActFrm(formula) => {
                    if let ActFrmKind::Quantifier { variables, .. } = &formula.node {
                        collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
                    }
                }
                // A `RegFrm` node itself never carries a binder, only crossing through one to reach
                // an `ActFrm` does; the other five `MixedNode` variants are unreachable from a
                // `StateFrm` root (nothing crossed into from here reaches them) and are listed only
                // so this match stays exhaustive as `MixedNode` grows further crossings elsewhere.
                MixedNode::RegFrm(_)
                | MixedNode::SortExpression(_)
                | MixedNode::DataExpr(_)
                | MixedNode::ProcessExpr(_)
                | MixedNode::PbesExpr(_)
                | MixedNode::PresExpr(_) => {}
            }
            Ok(ControlFlow::Continue(()))
        })
        .map(|_| ())
}

/// Type-checks one equation's own body against the declared sorts: every `val(...)` expression,
/// action instance and fixpoint-variable reference in it.
fn check_state_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    formula: &StateFrm,
    formula_type: FormulaType,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    formula
        .try_visit::<Infallible, ModalError, _>(|formula| {
            match &formula.node {
                StateFrmKind::True | StateFrmKind::False => {}

                StateFrmKind::Delay(time) | StateFrmKind::Yaled(time) => {
                    if let Some(time) = time {
                        let real_sort = data.context().sorts.real_sort();
                        check_expression_against::<ModalError>(data, scope, time, real_sort, typing)?;
                    }
                }

                // `resolve_modal_variables` runs before checking and rewrites every `Id` naming
                // an enclosing `mu`/`nu` into `Resolved`; one surviving here refers to no
                // enclosing binder.
                StateFrmKind::Id(name, _arguments) => {
                    return Err(ModalError::UndeclaredStateVariable {
                        name: name.to_string(),
                        span: formula.span.clone(),
                    });
                }

                StateFrmKind::Resolved(name, arguments, declaration) => {
                    check_state_var_inst(
                        data,
                        tables,
                        scope,
                        name,
                        arguments,
                        *declaration,
                        &formula.span,
                        typing,
                    )?;
                }

                StateFrmKind::DataValExpr(data_expr) => {
                    check_val_expr(data, scope, data_expr, formula_type, typing)?;
                }

                StateFrmKind::DataValExprLeftMult(constant, _) | StateFrmKind::DataValExprRightMult(_, constant) => {
                    if formula_type == FormulaType::Bool {
                        return Err(ModalError::ConstantMultiplyInBooleanFormula {
                            span: formula.span.clone(),
                        });
                    }

                    let real_sort = data.context().sorts.real_sort();
                    check_expression_against::<ModalError>(data, scope, constant, real_sort, typing)?;
                }

                StateFrmKind::Modality { formula: reg, .. } => {
                    check_reg_formula(data, tables, scope, reg, typing)?;
                }

                StateFrmKind::Unary { .. }
                | StateFrmKind::Binary { .. }
                | StateFrmKind::Quantifier { .. }
                | StateFrmKind::Bound { .. } => {}

                StateFrmKind::FixedPoint { .. } => unreachable!(
                    "ModalEquationSystem extracts every FixedPoint into its own equation; \
                     an equation's own body never contains one"
                ),
            }
            Ok(ControlFlow::Continue(()))
        })
        .map(|_| ())
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

/// Checks a fixpoint variable's own declaration: each parameter's initial value against `params`
/// (already resolved by [`DeclarationTables::build`]), in the *outer* scope since the parameter it
/// initializes isn't bound yet. Duplicate-parameter and arity/sort bookkeeping for *references* to
/// this variable are handled once, up front, in that same table build.
fn check_fixed_point_declaration(
    data: &mut DataSpecification,
    scope: &Scope,
    variable: &StateVarDecl,
    params: &[(String, ResolvedSortId)],
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    for (argument, &(_, sort)) in variable.arguments.iter().zip(params) {
        check_expression_against::<ModalError>(data, scope, &argument.expr, sort, typing)?;
    }
    Ok(())
}

/// Type-checks an already-[`resolved`](StateFrmKind::Resolved) `name(args)` reference against its
/// enclosing fixpoint variable's declared parameter sorts, found in `tables.equations_by_id` by
/// matching `declaration` rather than `name`, since shadowing is already resolved into the
/// [`StateVarId`] this occurrence carries — mirrors
/// `crate::pbes::check::check_prop_var_inst`'s name-keyed lookup. On success, also pushes a
/// [`ResolvedName::StateVariable`] at `span`, the whole `name(args)` node: the name itself is not
/// separately spanned in the syntax tree.
fn check_state_var_inst(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    name: &str,
    arguments: &[DataExpr],
    declaration: StateVarId,
    span: &Span,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let &index = tables.equations_by_id.get(&declaration).expect(
        "a `StateFrmKind::Resolved` occurrence's declaration always matches one of \
             `ModalEquationSystem`'s own equations, since `resolve_modal_variables` only ever \
             resolves a name against a genuinely enclosing binder",
    );
    typing.push(
        span.clone(),
        ResolvedName::StateVariable {
            name: name.to_string(),
            declaration: declared_span(&tables.equation_decl_spans[index]),
        },
    );

    let params = &tables.equation_params[index];
    if arguments.len() != params.len() {
        return Err(ModalError::ArityMismatch {
            name: name.to_string(),
            expected: params.len(),
            found: arguments.len(),
            span: span.clone(),
        });
    }

    for (argument, &(_, sort)) in arguments.iter().zip(params) {
        check_expression_against::<ModalError>(data, scope, argument, sort, typing)?;
    }
    Ok(())
}

/// Type-checks a modality's regular formula: every action instance inside it against the `act`
/// table, and every `val(...)`/`@`-time data expression against its expected sort.
///
/// One [`Traverse::try_visit_mixed`] walk crosses from a `RegFrm`'s `Action` into its `ActFrm`;
/// a `RegFrm` node itself needs no checking, so all the per-node work lives in the
/// `MixedNode::ActFrm` arm.
fn check_reg_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    formula: &RegFrm,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    formula
        .try_visit_mixed::<Infallible, ModalError>(|node| {
            if let MixedNode::ActFrm(formula) = node {
                match &formula.node {
                    ActFrmKind::MultAct(multi_action) => {
                        for action in &multi_action.actions {
                            check_action(data, tables, scope, action, typing)?;
                        }
                    }
                    ActFrmKind::DataExprVal(data_expr) => {
                        let bool_sort = data.context().sorts.bool_sort();
                        check_expression_against::<ModalError>(data, scope, data_expr, bool_sort, typing)?;
                    }
                    ActFrmKind::At { operand, .. } => {
                        let real_sort = data.context().sorts.real_sort();
                        check_expression_against::<ModalError>(data, scope, operand, real_sort, typing)?;
                    }

                    ActFrmKind::True
                    | ActFrmKind::False
                    | ActFrmKind::Negation(_)
                    | ActFrmKind::Quantifier { .. }
                    | ActFrmKind::Binary { .. } => {}
                }
            }
            Ok(ControlFlow::Continue(()))
        })
        .map(|_| ())
}

/// Resolves one action instance inside a multi-action. With [`ActionChecking::Declared`], tries
/// every same-named overload of the right arity and requires exactly one to succeed — the
/// action-only counterpart of `crate::process::check::check_action_or_process`, as no process
/// table applies to a state formula's modalities. Each candidate is checked against its own
/// scratch `TypingInfo`, merged into `typing` only once the single successful candidate is known:
/// a failed or ambiguous candidate's typing must never reach `typing`, or it would misreport a
/// sort for the wrong overload at the same span.
///
/// With [`ActionChecking::Simple`], every action is accepted unconditionally instead — see that
/// variant's own doc comment.
///
/// On success, also pushes a [`ResolvedName::Action`] at `action.id`'s own span.
fn check_action(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    action: &Action,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let actions = match &tables.actions {
        ActionChecking::Simple => {
            typing.push(
                action.id.span.clone(),
                ResolvedName::Action {
                    name: action.id.node.clone(),
                    declaration: None,
                },
            );
            return Ok(());
        }
        ActionChecking::Declared(actions) => actions,
    };

    let candidates: Vec<usize> = actions
        .actions_by_name
        .get(action.id.as_str())
        .into_iter()
        .flatten()
        .copied()
        .filter(|&index| actions.action_domains[index].len() == action.args.len())
        .collect();

    if candidates.is_empty() {
        let mut candidates: Vec<String> = actions.actions_by_name.keys().cloned().collect();
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
                &actions.action_domains[index],
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
            declaration: declared_span(&actions.action_decl_spans[index]),
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
