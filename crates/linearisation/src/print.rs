//! Pretty-printing a [`LinearProcessSpecification`] back to mCRL2 concrete
//! syntax.
//!
//! This is the crate's interchange format: merc has no native rewriter/
//! sum-enumerator yet (see the crate README), so an actually explorable state
//! space still has to come from feeding the printed text to an external
//! backend — currently the FFI bridge in `tools/mcrl2`, which wraps the real
//! mCRL2 library.

use std::fmt;

use itertools::Itertools;

use crate::ActionInstance;
use crate::ActionSummand;
use crate::DeadlockSummand;
use crate::LinearProcessSpecification;

impl fmt::Display for ActionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.name.node)
        } else {
            write!(f, "{}({})", self.name.node, self.arguments.iter().format(", "))
        }
    }
}

/// Formats a multi-action; the empty list prints as `tau`.
fn fmt_multi_action(actions: &[ActionInstance], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if actions.is_empty() {
        write!(f, "tau")
    } else {
        write!(f, "{}", actions.iter().format("|"))
    }
}

impl fmt::Display for ActionSummand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.summation_variables.is_empty() {
            write!(f, "sum {} . ", self.summation_variables.iter().format(", "))?;
        }
        write!(f, "{} -> ", self.condition)?;
        fmt_multi_action(&self.multi_action, f)?;
        if !self.next_state.is_empty() {
            write!(f, " . P({})", self.next_state.iter().format(", "))?;
        }
        Ok(())
    }
}

impl fmt::Display for DeadlockSummand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.summation_variables.is_empty() {
            write!(f, "sum {} . ", self.summation_variables.iter().format(", "))?;
        }
        write!(f, "{} -> delta", self.condition)
    }
}

impl fmt::Display for LinearProcessSpecification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "proc P({}) =", self.process_parameters.iter().format(", "))?;

        let bodies = self
            .deadlock_summands
            .iter()
            .map(|s| s.to_string())
            .chain(self.action_summands.iter().map(|s| s.to_string()))
            .collect::<Vec<_>>();

        if bodies.is_empty() {
            // No summands at all: still a well-formed (if useless) process.
            writeln!(f, "  delta;")?;
        } else {
            writeln!(f, "  {};", bodies.iter().format("\n+ "))?;
        }

        write!(f, "\ninit P({});", self.initial_assignment.iter().format(", "))
    }
}
