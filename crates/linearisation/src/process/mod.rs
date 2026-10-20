mod substitute;
mod table;

pub(crate) use substitute::*;
pub(crate) use table::*;

use merc_process::MultiAction;
use merc_process::ProcessExpressionRef;
use merc_process::ProcessKind;

/// The multi-action `expr` stands for, when it is a synchronisation `a₁ | … | aₙ` of actions and
/// `tau`.
pub(crate) fn multi_action(expr: &ProcessExpressionRef<'_>) -> Option<MultiAction> {
    let mut actions = Vec::new();
    let mut stack = vec![expr.reborrow()];
    while let Some(expr) = stack.pop() {
        match expr.kind() {
            ProcessKind::Action(action) => actions.push(action.protect()),
            ProcessKind::Tau => {}
            ProcessKind::Sync(lhs, rhs) => {
                stack.push(lhs);
                stack.push(rhs);
            }
            _ => return None,
        }
    }
    Some(MultiAction::new(actions))
}

/// The operators of the parallel layer: those that apply to a parallel composition.
pub(crate) fn is_parallel_layer(expr: &ProcessExpressionRef<'_>) -> bool {
    matches!(
        expr.kind(),
        ProcessKind::Merge(..)
            | ProcessKind::Block(..)
            | ProcessKind::Hide(..)
            | ProcessKind::Rename(..)
            | ProcessKind::Comm(..)
            | ProcessKind::Allow(..)
    )
}

/// The operator of mCRL2's concrete syntax that `kind` is, for error messages.
pub(crate) fn operator_name(kind: &ProcessKind<'_>) -> &'static str {
    match kind {
        ProcessKind::Action(_) => "action",
        ProcessKind::Process(..) => "process call",
        ProcessKind::Delta => "delta",
        ProcessKind::Tau => "tau",
        ProcessKind::Sum(..) => "sum",
        ProcessKind::Stochastic(..) => "dist",
        ProcessKind::Block(..) => "block",
        ProcessKind::Hide(..) => "hide",
        ProcessKind::Rename(..) => "rename",
        ProcessKind::Comm(..) => "comm",
        ProcessKind::Allow(..) => "allow",
        ProcessKind::Sync(..) => "|",
        ProcessKind::AtTime(..) => "@",
        ProcessKind::Seq(..) => ".",
        ProcessKind::IfThen(..) => "->",
        ProcessKind::IfThenElse(..) => "-> <>",
        ProcessKind::BInit(..) => "<<",
        ProcessKind::Merge(..) => "||",
        ProcessKind::LeftMerge(..) => "||_",
        ProcessKind::Choice(..) => "+",
    }
}
