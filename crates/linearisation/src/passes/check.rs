use merc_process::ProcessExpression;
use merc_process::ProcessKind;
use merc_process::ProcessSpecification;
use merc_process::TERMINATE;

use crate::LinearisationError;
use crate::process::multi_action;
use crate::process::operator_name;

/// The check pass: rejects the specifications that the later passes do not support, so that they
/// can assume their input is supported.
///
/// Whether parallel composition only occurs at the outer level depends on which calls are expanded,
/// so that is checked by the Greibach normal form pass instead.
pub(crate) fn check(spec: &ProcessSpecification) -> Result<(), LinearisationError> {
    if let Some(label) = spec
        .action_labels
        .iter()
        .find(|label| label.name().value() == TERMINATE)
    {
        return Err(LinearisationError::ReservedActionName {
            action: label.name().value().to_string(),
        });
    }
    for body in spec
        .equations
        .iter()
        .map(|equation| &equation.body)
        .chain(spec.init.as_ref())
    {
        check_body(body)?;
    }
    Ok(())
}

/// Rejects time, stochastic operators, `<<`, `||_`, and `|` on anything but actions.
fn check_body(body: &ProcessExpression) -> Result<(), LinearisationError> {
    let mut stack = vec![body.copy()];
    while let Some(expr) = stack.pop() {
        let kind = expr.kind();
        let unsupported = match &kind {
            ProcessKind::Stochastic(..)
            | ProcessKind::AtTime(..)
            | ProcessKind::BInit(..)
            | ProcessKind::LeftMerge(..) => true,
            ProcessKind::Sync(..) => multi_action(&expr).is_none(),
            _ => false,
        };
        if unsupported {
            return Err(LinearisationError::UnsupportedConstruct {
                construct: operator_name(&kind),
                expression: expr.to_string(),
            });
        }

        match kind {
            ProcessKind::Action(_) | ProcessKind::Process(..) | ProcessKind::Delta | ProcessKind::Tau => {}
            // Checked above to consist of actions only.
            ProcessKind::Sync(..) => {}
            ProcessKind::Sum(_, operand)
            | ProcessKind::Stochastic(_, _, operand)
            | ProcessKind::Block(_, operand)
            | ProcessKind::Hide(_, operand)
            | ProcessKind::Rename(_, operand)
            | ProcessKind::Comm(_, operand)
            | ProcessKind::Allow(_, operand)
            | ProcessKind::AtTime(operand, _)
            | ProcessKind::IfThen(_, operand) => stack.push(operand),
            ProcessKind::IfThenElse(_, lhs, rhs)
            | ProcessKind::Seq(lhs, rhs)
            | ProcessKind::BInit(lhs, rhs)
            | ProcessKind::Merge(lhs, rhs)
            | ProcessKind::LeftMerge(lhs, rhs)
            | ProcessKind::Choice(lhs, rhs) => {
                stack.push(rhs);
                stack.push(lhs);
            }
        }
    }
    Ok(())
}
