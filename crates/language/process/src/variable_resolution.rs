//! Variable resolution for process specifications: ties every free variable occurrence in a
//! `proc` body or `init` to the declaration (a global, a process parameter, or a `sum`/`dist`
//! binder) it refers to.

use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::VarIdAllocator;
use merc_typecheck::internal::VariableScope as Scope;
use merc_typecheck::internal::resolve_in_data_expr;

/// Resolves every free variable reference in `spec`'s `proc` bodies and `init`.
pub(crate) fn resolve_process_variables(spec: &mut UntypedProcessSpecification) {
    let mut ids = VarIdAllocator::default();
    let globals = Scope::from_declarations(&mut spec.global_variables, &mut ids);

    for proc_decl in &mut spec.process_declarations {
        // A process's own parameters shadow a global variable of the same name.
        let mut scope = globals.clone();
        scope.push_declarations(&mut proc_decl.params, &mut ids);
        resolve_in_process_expr(&mut proc_decl.body, &mut scope, &mut ids);
    }

    if let Some(init) = &mut spec.init {
        // `init` sits outside every process's own parameter scope — only globals apply.
        let mut scope = globals.clone();
        resolve_in_process_expr(init, &mut scope, &mut ids);
    }
}

fn resolve_in_process_expr(expr: &mut ProcessExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        ProcessExprKind::Delta | ProcessExprKind::Tau => {}
        ProcessExprKind::Action(_, args) => {
            for arg in args {
                resolve_in_data_expr(arg, scope, ids);
            }
        }
        ProcessExprKind::Id(_, assignments) => {
            // Only the assignment's *value* is a free variable read.
            for assignment in assignments {
                resolve_in_data_expr(&mut assignment.expr, scope, ids);
            }
        }
        ProcessExprKind::Sum { variables, operand } => {
            let pushed = scope.push_declarations(variables, ids);
            resolve_in_process_expr(operand, scope, ids);
            scope.pop(pushed);
        }
        ProcessExprKind::Dist {
            variables,
            expr: weight,
            operand,
        } => {
            let pushed = scope.push_declarations(variables, ids);
            // `dist`'s weight is resolved with its own bound variables already in scope.
            resolve_in_data_expr(weight, scope, ids);
            resolve_in_process_expr(operand, scope, ids);
            scope.pop(pushed);
        }
        ProcessExprKind::Binary { lhs, rhs, .. } => {
            resolve_in_process_expr(lhs, scope, ids);
            resolve_in_process_expr(rhs, scope, ids);
        }
        ProcessExprKind::Hide { operand, .. }
        | ProcessExprKind::Rename { operand, .. }
        | ProcessExprKind::Allow { operand, .. }
        | ProcessExprKind::Block { operand, .. }
        | ProcessExprKind::Comm { operand, .. } => resolve_in_process_expr(operand, scope, ids),
        ProcessExprKind::Condition { condition, then, else_ } => {
            resolve_in_data_expr(condition, scope, ids);
            resolve_in_process_expr(then, scope, ids);
            if let Some(else_) = else_ {
                resolve_in_process_expr(else_, scope, ids);
            }
        }
        ProcessExprKind::At { expr, operand } => {
            resolve_in_process_expr(expr, scope, ids);
            resolve_in_data_expr(operand, scope, ids);
        }
    }
}
