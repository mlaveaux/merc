use std::collections::HashMap;

use merc_data::DataVariable;
use merc_process::Action;
use merc_process::ActionLabel;
use merc_process::ProcessExpression;
use merc_process::ProcessExpressionRef;
use merc_process::ProcessIdentifier;
use merc_process::ProcessKind;
use merc_process::split_choice;
use merc_process::split_seq;

use crate::data::NameGenerator;
use crate::process::Call;
use crate::process::ProcessTable;

/// Which equations can terminate successfully, i.e. finish without deadlock.
///
/// This is the least fixpoint of [Termination::term] over the equations, which over-approximates
/// actual termination because it ignores data: `c -> a` is considered terminating whatever `c` is.
/// An over-approximation is what the later passes need. Regular form drops everything sequenced
/// after a call that cannot terminate, so an under-approximation would lose behaviour.
///
/// Invariant: `flags` has an entry for every equation in the table it was computed for, and is the
/// least fixpoint for those equations.
pub(crate) struct Termination {
    flags: HashMap<ProcessIdentifier, bool>,
    terminated: Option<ProcessIdentifier>,
}

impl Termination {
    pub(crate) fn analyse(table: &ProcessTable) -> Self {
        let mut result = Termination {
            flags: table.ids().map(|id| (id.clone(), false)).collect(),
            terminated: None,
        };
        let mut changed = true;
        while changed {
            changed = false;
            for id in table.ids() {
                if !result.flags[id] && result.term(&table.body(id).copy()) {
                    result.flags.insert(id.clone(), true);
                    changed = true;
                }
            }
        }
        result
    }

    pub(crate) fn of(&self, id: &ProcessIdentifier) -> bool {
        self.flags[id]
    }

    /// Whether `expr` can terminate, given the flags of the equations it calls. The input must have
    /// passed the check pass.
    ///
    /// Chains of choices and sequences are split without recursion, so the recursion depth is the
    /// nesting depth of alternating operators.
    pub(crate) fn term(&self, expr: &ProcessExpressionRef<'_>) -> bool {
        match expr.kind() {
            ProcessKind::Action(_) | ProcessKind::Tau | ProcessKind::Sync(..) => true,
            ProcessKind::Delta => false,
            ProcessKind::Process(identifier, _) => self.of(&identifier.protect()),
            ProcessKind::Sum(_, operand)
            | ProcessKind::Stochastic(_, _, operand)
            | ProcessKind::Block(_, operand)
            | ProcessKind::Hide(_, operand)
            | ProcessKind::Rename(_, operand)
            | ProcessKind::Comm(_, operand)
            | ProcessKind::Allow(_, operand)
            | ProcessKind::AtTime(operand, _)
            | ProcessKind::IfThen(_, operand) => self.term(&operand),
            ProcessKind::IfThenElse(_, then, else_) => self.term(&then) || self.term(&else_),
            ProcessKind::Choice(..) => split_choice(expr).iter().any(|operand| self.term(operand)),
            ProcessKind::Seq(..) => split_seq(expr).iter().all(|operand| self.term(operand)),
            ProcessKind::Merge(lhs, rhs) | ProcessKind::LeftMerge(lhs, rhs) | ProcessKind::BInit(lhs, rhs) => {
                self.term(&lhs) && self.term(&rhs)
            }
        }
    }

    /// Adds an equation that the passes introduce; its body may only call equations that are
    /// already in the table, so its flag follows from theirs without iterating.
    pub(crate) fn add(
        &mut self,
        table: &mut ProcessTable,
        names: &mut NameGenerator,
        hint: &str,
        parameters: &[DataVariable],
        body: ProcessExpression,
    ) -> ProcessIdentifier {
        let terminates = self.term(&body.copy());
        let id = table.add(names, hint, parameters, body);
        self.flags.insert(id.clone(), terminates);
        id
    }

    /// The process the leaf `root(arguments)` of a parallel composition stands for, with its
    /// termination made observable: when `root` can terminate, a fresh equation
    /// `Root'(x⃗) = root(x⃗) · Terminate · δ` takes its place.
    ///
    /// A fresh equation, rather than appending to `root`'s own body, leaves the recursive calls to
    /// `root` alone, and `root` may also be used elsewhere.
    pub(crate) fn make_explicit(&mut self, table: &mut ProcessTable, names: &mut NameGenerator, root: Call) -> Call {
        if !self.of(&root.process) {
            return root;
        }

        let terminated = self.terminated(table, names);
        let parameters = root.process.parameters().to_vec();
        let body = ProcessExpression::seq(
            &Call::identity(root.process.clone()).expression(),
            &Call::identity(terminated).expression(),
        );
        let hint = root.process.name().value().to_string();
        let process = self.add(table, names, &hint, &parameters, body);
        Call {
            process,
            arguments: root.arguments,
        }
    }

    /// The equation `Terminated() = Terminate · δ`, created on first use.
    fn terminated(&mut self, table: &mut ProcessTable, names: &mut NameGenerator) -> ProcessIdentifier {
        if let Some(id) = &self.terminated {
            return id.clone();
        }
        let terminate = Action::new(&ActionLabel::terminate(), &[]);
        let body = ProcessExpression::seq(&ProcessExpression::action(&terminate), &ProcessExpression::delta());
        let id = self.add(table, names, "Terminated", &[], body);
        self.terminated = Some(id.clone());
        id
    }
}
