//! The scoped walk over every PRES equation's formula and `init`: checks each
//! data expression embedded via `val(...)` against `Real`, and each
//! `PropVarInst` against the equation table. Resolves the declared sorts of all
//! bound variables.

use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_syntax::PresExpr;
use merc_syntax::PresExprKind;
use merc_syntax::PropVarInst;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedPres;
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

use super::PresError;
use super::pres_specification::DeclarationTables;
use super::pres_specification::resolve_declared_sort;

/// Checks a PRES specification against the declared sorts, returning the merged
/// typing information.
pub(super) fn check_pres_specification(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    spec: &UntypedPres,
) -> Result<TypingInfo, PresError> {
    let mut typing = TypingInfo::default();
    let mut sort_references = Vec::new();

    for decl in &spec.global_variables {
        typing_info::collect_sort_name_references(&decl.sort, &mut sort_references);
    }
    for eqn in &spec.equations {
        for param in &eqn.variable.parameters {
            typing_info::collect_sort_name_references(&param.sort, &mut sort_references);
        }
    }

    let globals: Vec<(VarId, ResolvedSortId, Span)> = spec
        .global_variables
        .iter()
        .zip(&tables.global_sorts)
        .map(|(decl, &sort)| {
            (
                decl.var_id.expect("resolve_pres_variables ran before checking"),
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

    for (eqn, params) in spec.equations.iter().zip(&tables.equation_params) {
        let mut scope = globals.clone();
        // An equation's own parameters are in scope throughout its formula.
        scope.extend(eqn.variable.parameters.iter().zip(params).map(|(decl, &(_, sort))| {
            (
                decl.var_id.expect("resolve_pres_variables ran before checking"),
                sort,
                decl.identifier.span.clone(),
            )
        }));
        for (decl, &(_, sort)) in eqn.variable.parameters.iter().zip(params) {
            typing_info::push_binder_declaration(
                data,
                &mut typing,
                decl.identifier.span.clone(),
                decl.identifier.node.clone(),
                sort,
            );
        }
        collect_scope(data, &eqn.formula, &mut scope, &mut sort_references, &mut typing)?;
        check_pres_expr(data, tables, &scope, &eqn.formula, &mut typing)?;
    }

    // `init` is a bare `PropVarInst`, checked the same way as one appearing inside a formula —
    // scope = globals only, since it sits outside every equation's own parameter scope.
    check_prop_var_inst(data, tables, &globals, &spec.init, &mut typing)?;

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Collects the scope for a PRES expression, resolving the declared sorts of all `Bound` binders.
///
/// A plain [`Traverse::try_visit`] walk replaces the previous hand-written recursive-descent
/// version, the same way `crate::process::check::collect_scope` does: the one arm with real
/// per-node work (`Bound`) does it and returns, leaving the traversal's own descent into
/// `PresExpr`'s same-type children (`Traverse::push_children`) to reach the rest, in the same
/// left-to-right, pre-order sequence the original recursive calls did.
fn collect_scope(
    data: &mut DataSpecification,
    expr: &PresExpr,
    scope: &mut Vec<(VarId, ResolvedSortId, Span)>,
    sort_references: &mut Vec<typing_info::SortReference>,
    typing: &mut TypingInfo,
) -> Result<(), PresError> {
    expr.try_visit(|expr| {
        if let PresExprKind::Bound { variables, .. } = &expr.node {
            collect_binder_sorts(data, scope, sort_references, typing, variables, resolve_declared_sort)?;
        }
        Ok(ControlFlow::Continue(()))
    })
    .map(|_: Option<Infallible>| ())
}

/// Type-checks a PRES expression: every `val(...)` against `Real`, every `PropVarInst` against the
/// equation table, and a constant-multiply's own constant against `Real`.
///
/// A plain [`Traverse::try_visit`] walk replaces the previous hand-written recursive-descent
/// version, the same way [`collect_scope`] does above.
fn check_pres_expr(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    expr: &PresExpr,
    typing: &mut TypingInfo,
) -> Result<(), PresError> {
    expr.try_visit(|expr| {
        match &expr.node {
            PresExprKind::True
            | PresExprKind::False
            | PresExprKind::Negation(_)
            | PresExprKind::Binary { .. }
            | PresExprKind::Equal { .. }
            | PresExprKind::Condition { .. }
            | PresExprKind::Bound { .. } => {}

            PresExprKind::DataValExpr(data_expr) => {
                let real_sort = data.context().sorts.real_sort();
                check_expression_against::<PresError>(data, scope, data_expr, real_sort, typing)?;
            }

            PresExprKind::PropVarInst(inst) => check_prop_var_inst(data, tables, scope, inst, typing)?,

            PresExprKind::RightConstantMultiply { constant, .. }
            | PresExprKind::LeftConstantMultiply { constant, .. } => {
                let real_sort = data.context().sorts.real_sort();
                check_expression_against::<PresError>(data, scope, constant, real_sort, typing)?;
            }
        }
        Ok(ControlFlow::Continue(()))
    })
    .map(|_: Option<Infallible>| ())
}

/// Resolves `inst.identifier` against the equation table (`UndeclaredPropositionalVariable` if
/// missing), checks its argument count against the declared parameter count (`ArityMismatch`), and
/// checks each argument against its parameter's sort. On success, also pushes a
/// [`ResolvedName::PropositionalVariable`] at `inst.identifier`'s own span (not `inst.span`, the
/// whole `name(args)` node): unlike an action/process name, a PRES equation is never overloaded,
/// so the equation table's single match is the answer. Mirrors `crate::pbes::check::check_prop_var_inst`.
fn check_prop_var_inst(
    data: &mut DataSpecification,
    tables: &DeclarationTables,
    scope: &Scope,
    inst: &PropVarInst,
    typing: &mut TypingInfo,
) -> Result<(), PresError> {
    let Some(&index) = tables.equations_by_name.get(&inst.identifier.node) else {
        return Err(PresError::UndeclaredPropositionalVariable {
            name: inst.identifier.node.clone(),
            span: inst.span.clone(),
            candidates: tables.equations_by_name.keys().cloned().collect(),
        });
    };
    typing.push(
        inst.identifier.span.clone(),
        ResolvedName::PropositionalVariable {
            name: inst.identifier.node.clone(),
            declaration: declared_span(&tables.equation_decl_spans[index]),
        },
    );

    let params = &tables.equation_params[index];
    if inst.arguments.len() != params.len() {
        return Err(PresError::ArityMismatch {
            name: inst.identifier.node.clone(),
            expected: params.len(),
            found: inst.arguments.len(),
            span: inst.span.clone(),
        });
    }

    for (arg, (_, sort)) in inst.arguments.iter().zip(params) {
        check_expression_against::<PresError>(data, scope, arg, *sort, typing)?;
    }
    Ok(())
}

#[cfg(test)]
mod stack_depth_probe {
    //! Isolates `check_pres_expr`'s own recursion from the parser's: the tree here is built
    //! directly, so a stack overflow can only come from this module's own walk. Structurally the
    //! same pattern as `modal::check`/`process::check`'s own probes (see
    //! `review/stack-overflow-recursion.md`, which originally flagged this checker's exposure as
    //! present but untested) — `check_pres_expr`/`collect_scope` are `Traverse::try_visit` walks
    //! from the start here, so there is no "before" state to compare against, only this regression
    //! test. `expr` is let drop normally at the end of the test (rather than leaked with
    //! `mem::forget`) as proof the separate recursive-`Drop` bug covering the same document
    //! doesn't fire here either.
    use merc_syntax::Span;
    use merc_syntax::UntypedDataSpecification;

    use super::*;

    fn deep_negation(depth: usize) -> PresExpr {
        let mut expr = PresExprKind::True.spanned(Span::default());
        for _ in 0..depth {
            expr = PresExprKind::Negation(Box::new(expr)).spanned(Span::default());
        }
        expr
    }

    #[test]
    fn deeply_nested_negation_does_not_overflow_the_stack() {
        let mut data = DataSpecification::from_untyped(UntypedDataSpecification::parse("").unwrap()).unwrap();
        let tables = DeclarationTables {
            global_sorts: Vec::new(),
            equation_params: Vec::new(),
            equation_decl_spans: Vec::new(),
            equations_by_name: Default::default(),
        };
        let mut typing = TypingInfo::default();
        let scope: Vec<(VarId, ResolvedSortId, Span)> = Vec::new();

        // 100,000 nested negations, exactly as `modal::check`/`process::check`'s own probes do.
        let expr = deep_negation(100_000);
        let result = check_pres_expr(&mut data, &tables, &scope, &expr, &mut typing);
        assert!(result.is_ok());
    }
}
