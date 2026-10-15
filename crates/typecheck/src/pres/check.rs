//! The scoped walk over every PRES equation's formula and `init`: checks each `val(...)`
//! expression against `Real`, resolves each `PropVarInst` against the equation table (name, arity,
//! and each argument's sort), and resolves the declared sorts of all bound variables —
//! accumulating each checked expression's [`TypingInfo`] along the way.

use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_syntax::PresExpr;
use merc_syntax::PresExprKind;
use merc_syntax::PropVarInst;
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
use crate::checking::declare_variables;
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

    let mut globals = Vec::new();
    declare_variables(
        data,
        &mut globals,
        &mut typing,
        &spec.global_variables,
        tables.global_sorts.iter().copied(),
    );

    for (eqn, params) in spec.equations.iter().zip(&tables.equation_params) {
        // An equation's own parameters are in scope throughout its formula.
        let mut scope = globals.clone();
        let sorts = params.iter().map(|&(_, sort)| sort);
        declare_variables(data, &mut scope, &mut typing, &eqn.variable.parameters, sorts);
        collect_scope(data, &eqn.formula, &mut scope, &mut sort_references, &mut typing)?;
        check_pres_expr(data, tables, &scope, &eqn.formula, &mut typing)?;
    }

    // `init` is a bare `PropVarInst`, checked the same way as one appearing inside a formula —
    // scope = globals only, since it sits outside every equation's own parameter scope.
    check_prop_var_inst(data, tables, &globals, &spec.init, &mut typing)?;

    typing_info::push_sort_references(data, &sort_references, &mut typing);
    Ok(typing)
}

/// Resolves the declared sort of every `Bound` binder in `expr`, extending `scope`.
fn collect_scope(
    data: &mut DataSpecification,
    expr: &PresExpr,
    scope: &mut Vec<(VarId, ResolvedSortId)>,
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
/// so the equation table's single match is the answer.
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
