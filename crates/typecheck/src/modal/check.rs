// The scoped walk over the state formula, checking each `val(...)` expression, action instance and
// fixpoint-variable reference. See `ValSort` for what a state-level `val`'s declared sort allows.
//
// `collect_scope` and `check_state_formula` each cross from a `StateFrm` into the `RegFrm`/`ActFrm`
// of its modalities via `Traverse::try_visit_mixed`, rather than each maintaining its own hand-written
// `_regfrm`/`_actfrm` recursive cousin; `check_state_formula`'s own scoped `state_vars` stack (pushed
// for a `FixedPoint`'s body, popped once it's done) is threaded through `Traverse::visit_scoped`'s
// `enter`/`exit` hooks. See `review/stack-overflow-recursion.md` for why this needed two additions to
// `Traverse` before it was possible at all.

use std::collections::HashSet;
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
use merc_utilities::Step;

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

    check_state_formula(data, tables, &scope, &spec.formula, formula_type, &mut typing)?;

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Collects the scope for a state formula, resolving the declared sorts of every
/// `forall`/`exists`/`inf`/`sup`/`sum` binder — in the state formula itself, or nested inside one of
/// its modalities' action formulas — and of every fixpoint variable's own parameters, the latter
/// ordinary data variables throughout the fixpoint's body, exactly like a PRES equation's
/// parameters.
///
/// The fixpoint variable *names* are not collected here; they live on their own stack, maintained
/// by [`check_state_formula`]'s own scoped walk as it enters and leaves each binder.
///
/// One [`Traverse::try_visit_mixed`] walk replaces what used to be three hand-written recursive
/// functions, one per node type (`collect_scope`/`collect_scope_regfrm`/`collect_scope_actfrm`):
/// crossing from a `StateFrm`'s `Modality` into its [`RegFrm`], and from there into an `Action`'s
/// [`ActFrm`], is `Traverse::push_mixed_children`'s job now, not this function's, and descending
/// into a node's own same-type children no longer needs writing out either — only the per-node work
/// (pushing a binder's variables into `scope`) remains.
fn collect_scope(
    data: &mut DataSpecification,
    formula: &StateFrm,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
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
                    StateFrmKind::FixedPoint { variable, .. } => {
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
                    }
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

/// Type-checks a state formula against the declared sorts: every `val(...)` expression, action
/// instance and fixpoint-variable reference in it.
///
/// The scoped stack of fixpoint variables currently in scope (`state_vars`, pushed for a
/// `FixedPoint`'s body and popped again once that body is done being checked, so an enclosing
/// formula never sees an inner fixpoint's variable) is local to this walk, not threaded in from
/// [`check_modal_specification`] as it used to be: [`Traverse::visit_scoped`]'s `enter`/`exit` hooks
/// push and pop it directly, in place of the recursive `check_fixed_point` helper this used to call
/// out to. Crossing from a `Modality` into its [`RegFrm`] is still a plain nested call to
/// [`check_reg_formula`] (not a further `visit_mixed` from in here), since it needs to happen
/// *before* [`Traverse::visit_scoped`]'s own descent reaches the modality's `StateFrm` operand, to
/// keep the same left-to-right checking order the original recursive-descent version had.
fn check_state_formula(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    formula: &StateFrm,
    formula_type: FormulaType,
    typing: &mut TypingInfo,
) -> Result<(), ModalError> {
    let mut state_vars = StateVarStack::new();
    formula
        .visit_scoped::<(), StateVarStack, Infallible, ModalError, _, _>(
            (),
            &mut state_vars,
            |formula, context, state_vars| {
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
                            state_vars,
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

                    StateFrmKind::DataValExprLeftMult(constant, _)
                    | StateFrmKind::DataValExprRightMult(_, constant) => {
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

                    StateFrmKind::FixedPoint { variable, .. } => {
                        let params = check_fixed_point_declaration(data, scope, variable, typing)?;
                        let state_var_id = variable.id.expect("resolve_modal_variables ran before checking");
                        state_vars.push((state_var_id, variable.span.clone(), params));
                    }
                }
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |formula, _context, state_vars| {
                if let StateFrmKind::FixedPoint { .. } = &formula.node {
                    state_vars.pop();
                }
            },
        )
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

/// Checks a fixpoint variable's own declaration: each parameter's initial value against its
/// declared sort, in the *outer* scope since the parameter it initializes isn't bound yet. Returns
/// the resolved parameter sorts for [`check_state_formula`]'s `visit_scoped` `enter` hook to push
/// onto `state_vars`, alongside the variable's own [`StateVarId`], for its body to reference
/// recursively; the corresponding pop happens in that same walk's `exit` hook, once the body is
/// done being checked — this function itself no longer recurses into the body at all.
fn check_fixed_point_declaration(
    data: &mut DataSpecification,
    scope: &Scope,
    variable: &StateVarDecl,
    typing: &mut TypingInfo,
) -> Result<Vec<ResolvedSortId>, ModalError> {
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
    Ok(params)
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
             `FixedPoint` pushed onto `state_vars` by `check_state_formula`'s `visit_scoped` `enter` \
             hook, since `resolve_modal_variables` only ever resolves a name against a genuinely \
             enclosing binder",
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

/// Type-checks a modality's regular formula: every action instance inside it against the `act`
/// table, and every `val(...)`/`@`-time data expression against its expected sort.
///
/// One [`Traverse::try_visit_mixed`] walk starting at `formula` replaces the previous
/// `check_reg_formula`/`check_action_formula` pair of hand-written recursive functions: a `RegFrm`
/// node itself never needs checking, only dispatching towards an `Action`'s [`ActFrm`], so all the
/// per-node work below lives in the `MixedNode::ActFrm` arm; crossing from a `RegFrm`'s `Action`
/// into that `ActFrm`, and descending into both node types' own same-type children (an `ActFrm`
/// `Negation`/`Quantifier`/`Binary`/`At`'s nested formula), no longer needs writing out here either.
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

                    // Like `StateFrmKind::Delay`/`Yaled`'s own `@`-time argument, `operand` is
                    // checked against `Real` regardless of `expr`'s own sort — there's no
                    // `ValSort`-style ambiguity here since an action formula's `val(...)` is always
                    // `Bool` (see `ValSort`'s doc comment). `expr` itself is a same-type `ActFrm`
                    // child, checked separately when the walk reaches it in its own right.
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
    //!
    //! This used to SIGABRT (see `review/stack-overflow-recursion.md`) before `check_state_formula`
    //! and `collect_scope` were migrated onto `Traverse::visit_scoped`/`try_visit_mixed`, which walk
    //! with an explicit heap stack rather than native recursion; it now passes, and stays here as a
    //! regression test for that.
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
        let scope: Vec<(VarId, ResolvedSortId, Span)> = Vec::new();

        // 100,000 nested `!`s: well within what a real, if pathological, `.mcf` file could contain
        // (the parser itself already overflows at a similar depth — a separate, out-of-scope
        // concern), but small enough that a bounded-recursion walk would handle it trivially.
        let formula = deep_negation(100_000);
        let result = check_state_formula(&mut data, &tables, &scope, &formula, FormulaType::Bool, &mut typing);
        assert!(result.is_ok());

        // Dropping a 100,000-deep `Box<StateFrm>` chain recurses through the default drop glue one
        // `Box` at a time and overflows the stack on its own -- a known, separate bug from the one
        // this test exists to check (see `review/stack-overflow-recursion.md` and
        // `crates/syntax/src/traverse.rs`'s own `stack_depth_probe` tests, which forget for the same
        // reason). Leak it so this test only exercises `check_state_formula`'s own walk.
        std::mem::forget(formula);
    }
}
