use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_process::Action;
use merc_process::MultiAction;
use merc_process::ProcessExpression;
use merc_process::ProcessExpressionRef;
use merc_process::ProcessIdentifier;
use merc_process::ProcessKind;
use merc_process::split_choice;
use merc_process::split_seq;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_and;
use merc_utilities::optimized_not;

use crate::LinearisationError;
use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::data::free_variables;
use crate::passes::termination::Termination;
use crate::process::Call;
use crate::process::ProcessTable;
use crate::process::multi_action;
use crate::process::occurring_in;
use crate::process::operator_name;
use crate::process::substitute;

/// What a summand does first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    Action(MultiAction),
    Delta,
    /// Only in variable-head form; [Gnf::summands] substitutes these away.
    Call(Call),
}

/// A summand `Σ sums. condition → head · tail₁ · … · tailₙ` of a body in Greibach normal form.
///
/// Invariants: `sums` are pairwise distinct and distinct from the variables in scope of the
/// equation the summand belongs to, and a summand headed by `δ` has an empty tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summand {
    pub(crate) sums: Vec<DataVariable>,
    pub(crate) condition: DataExpression,
    pub(crate) head: Head,
    pub(crate) tail: Vec<Call>,
}

impl Summand {
    fn new(head: Head) -> Self {
        Summand {
            sums: Vec::new(),
            condition: DataExpression::true_(),
            head,
            tail: Vec::new(),
        }
    }

    /// The names of the free variables of the condition, the head and the tail.
    fn names(&self) -> impl Iterator<Item = String> + '_ {
        let (actions, head): (&[Action], &[DataExpression]) = match &self.head {
            Head::Action(action) => (action.actions(), &[]),
            Head::Call(call) => (&[], &call.arguments),
            Head::Delta => (&[], &[]),
        };
        std::iter::once(self.condition.clone())
            .chain(actions.iter().flat_map(|action| action.arguments().iter()))
            .chain(head.iter().cloned())
            .chain(self.tail.iter().flat_map(|call| call.arguments.iter().cloned()))
            .flat_map(|expr| free_variables(&expr))
            .map(|variable| variable.name().to_string())
    }
}

/// Applies `sigma` to the arguments of a call.
fn substitute_call(call: &Call, sigma: &Substitution, names: &mut NameGenerator) -> Call {
    Call {
        process: call.process.clone(),
        arguments: sigma.apply_all(&call.arguments, names),
    }
}

/// Applies `sigma` to the arguments of the actions of a multi-action.
pub(crate) fn substitute_action(action: &MultiAction, sigma: &Substitution, names: &mut NameGenerator) -> MultiAction {
    action.map_actions(|action| {
        let arguments = sigma.apply_all(&action.arguments().to_vec(), names);
        Some(Action::new(&action.label().protect(), &arguments))
    })
}

/// The Greibach normal form pass.
///
/// Brings the bodies of equations into the form of a choice of [Summand]s headed by an action or
/// `δ`, followed by a sequence of calls. Subterms that do not fit, such as a choice in a tail, are
/// lifted into fresh equations over their free variables.
pub(crate) struct Gnf<'a> {
    table: &'a mut ProcessTable,
    termination: &'a mut Termination,
    names: &'a mut NameGenerator,
    /// Lifted subterms, keyed by the term and its parameters, so equal subterms share an equation.
    lifted: HashMap<(ProcessExpression, Vec<DataVariable>), ProcessIdentifier>,
    /// The bodies in GNF computed so far.
    done: HashMap<ProcessIdentifier, Rc<Vec<Summand>>>,
}

impl<'a> Gnf<'a> {
    pub(crate) fn new(
        table: &'a mut ProcessTable,
        termination: &'a mut Termination,
        names: &'a mut NameGenerator,
    ) -> Self {
        Gnf {
            table,
            termination,
            names,
            lifted: HashMap::new(),
            done: HashMap::new(),
        }
    }

    pub(crate) fn terminates(&self, id: &ProcessIdentifier) -> bool {
        self.termination.of(id)
    }

    /// Adds an equation whose body only calls equations already in the table.
    pub(crate) fn add_equation(
        &mut self,
        hint: &str,
        parameters: &[DataVariable],
        body: ProcessExpression,
    ) -> ProcessIdentifier {
        self.termination.add(self.table, self.names, hint, parameters, body)
    }

    /// The body of `root` in GNF.
    ///
    /// Leading calls are substituted by the GNF of the called equation. The equations are
    /// processed with an explicit stack in dependency order, so a long chain of leading calls
    /// cannot overflow the call stack; a cycle of leading calls is unguarded recursion.
    pub(crate) fn summands(&mut self, root: &ProcessIdentifier) -> Result<Rc<Vec<Summand>>, LinearisationError> {
        let mut stack = vec![root.clone()];
        let mut busy: HashSet<ProcessIdentifier> = HashSet::new();
        let mut variable_head: HashMap<ProcessIdentifier, Vec<Summand>> = HashMap::new();

        while let Some(id) = stack.last().cloned() {
            if self.done.contains_key(&id) {
                stack.pop();
                continue;
            }

            if !variable_head.contains_key(&id) {
                let body = self.table.body(&id).clone();
                let parameters = id.parameters().to_vec();
                let summands = self.variable_head(&body.copy(), &parameters)?;
                variable_head.insert(id.clone(), summands);
            }
            busy.insert(id.clone());

            let mut pending: Vec<ProcessIdentifier> = Vec::new();
            for summand in &variable_head[&id] {
                if let Head::Call(call) = &summand.head
                    && !self.done.contains_key(&call.process)
                    && !pending.contains(&call.process)
                {
                    if busy.contains(&call.process) {
                        return Err(LinearisationError::UnguardedRecursion {
                            process: call.process.name().value().to_string(),
                        });
                    }
                    pending.push(call.process.clone());
                }
            }
            if !pending.is_empty() {
                stack.extend(pending);
                continue;
            }

            let summands = variable_head.remove(&id).expect("computed above");
            let scope = id.parameters().to_vec();
            let expanded = self.substitute_heads(summands, &scope);
            self.done.insert(id.clone(), Rc::new(expanded));
            busy.remove(&id);
            stack.pop();
        }

        Ok(self.done[root].clone())
    }

    /// Replaces each summand `Σ d. c → Q(e) · tail` by the summands of `Q`'s GNF instantiated with
    /// `e`. The GNF of every called `Q` must be done.
    fn substitute_heads(&mut self, summands: Vec<Summand>, scope: &[DataVariable]) -> Vec<Summand> {
        let mut result = Vec::with_capacity(summands.len());
        for summand in summands {
            let call = match &summand.head {
                Head::Call(call) => call.clone(),
                _ => {
                    result.push(summand);
                    continue;
                }
            };

            let called = self.done[&call.process].clone();
            let mut sigma = Substitution::new(call.process.parameters().iter().zip(call.arguments.iter().cloned()));
            // The instantiated summands end up under the sums of `summand` and next to its
            // condition and tail, so their own sums must not capture anything those refer to.
            for name in scope
                .iter()
                .chain(&summand.sums)
                .map(|variable| variable.name().to_string())
                .chain(summand.names())
            {
                sigma.avoid(&name);
            }

            for inner in called.iter() {
                let (sums, sigma) = sigma.bind(&inner.sums, self.names);
                let condition = optimized_and(summand.condition.clone(), sigma.apply(&inner.condition, self.names));
                if condition.is_false() {
                    continue;
                }
                let head = match &inner.head {
                    Head::Action(action) => Head::Action(substitute_action(action, &sigma, self.names)),
                    Head::Delta => Head::Delta,
                    Head::Call(_) => unreachable!("the GNF of a done equation has no leading calls"),
                };
                let tail = if head == Head::Delta {
                    Vec::new()
                } else {
                    let mut tail: Vec<Call> = inner
                        .tail
                        .iter()
                        .map(|call| substitute_call(call, &sigma, self.names))
                        .collect();
                    tail.extend(summand.tail.iter().cloned());
                    tail
                };
                let mut all_sums = summand.sums.clone();
                all_sums.extend(sums);
                result.push(Summand {
                    sums: all_sums,
                    condition,
                    head,
                    tail,
                });
            }
        }
        result
    }

    /// The summands of `expr` in variable-head form, with `scope` the variables in scope.
    ///
    /// Chains of choices and sequences are split without recursion, so the recursion depth is the
    /// nesting depth of sums, conditions and alternating choice and sequence.
    fn variable_head(
        &mut self,
        expr: &ProcessExpressionRef<'_>,
        scope: &[DataVariable],
    ) -> Result<Vec<Summand>, LinearisationError> {
        if let Some(action) = multi_action(expr) {
            return Ok(vec![Summand::new(Head::Action(action))]);
        }
        Ok(match expr.kind() {
            ProcessKind::Delta => vec![Summand::new(Head::Delta)],
            ProcessKind::Process(identifier, arguments) => vec![Summand::new(Head::Call(Call {
                process: identifier.protect(),
                arguments: arguments.to_vec(),
            }))],
            ProcessKind::Choice(..) => {
                let mut result = Vec::new();
                for operand in split_choice(expr) {
                    result.extend(self.variable_head(&operand, scope)?);
                }
                result
            }
            ProcessKind::Sum(variables, operand) => {
                // Rename binders that shadow a variable in scope, so that sums never shadow.
                let mut sigma = Substitution::default();
                for variable in scope {
                    sigma.avoid(variable.name());
                }
                let (variables, sigma) = sigma.bind(&variables.to_vec(), self.names);
                let operand = substitute(&operand.protect(), &sigma, self.names);

                let mut inner_scope = scope.to_vec();
                inner_scope.extend(variables.iter().cloned());
                let mut result = self.variable_head(&operand.copy(), &inner_scope)?;
                for summand in &mut result {
                    let mut sums = variables.clone();
                    sums.append(&mut summand.sums);
                    summand.sums = sums;
                }
                result
            }
            ProcessKind::IfThen(condition, then) => {
                let mut result = Vec::new();
                self.guarded(condition.protect(), &then, scope, &mut result)?;
                result
            }
            ProcessKind::IfThenElse(condition, then, else_) => {
                let condition = condition.protect();
                let mut result = Vec::new();
                self.guarded(condition.clone(), &then, scope, &mut result)?;
                self.guarded(optimized_not(condition), &else_, scope, &mut result)?;
                result
            }
            ProcessKind::Seq(..) => {
                let operands = split_seq(expr);
                let (first, rest) = operands.split_first().expect("a sequence has at least two operands");
                let mut tail = Vec::new();
                for operand in rest {
                    self.tail_calls(operand, scope, &mut tail);
                }
                let mut result = self.variable_head(first, scope)?;
                for summand in &mut result {
                    if summand.head != Head::Delta {
                        summand.tail.extend(tail.iter().cloned());
                    }
                }
                result
            }
            kind @ (ProcessKind::Merge(..)
            | ProcessKind::Block(..)
            | ProcessKind::Hide(..)
            | ProcessKind::Rename(..)
            | ProcessKind::Comm(..)
            | ProcessKind::Allow(..)) => {
                return Err(LinearisationError::NotParallelPcrl {
                    construct: operator_name(&kind),
                    expression: expr.to_string(),
                });
            }
            ProcessKind::Action(_)
            | ProcessKind::Tau
            | ProcessKind::Sync(..)
            | ProcessKind::Stochastic(..)
            | ProcessKind::AtTime(..)
            | ProcessKind::BInit(..)
            | ProcessKind::LeftMerge(..) => unreachable!("a multi-action, or rejected by the check pass"),
        })
    }

    /// Appends the summands of `branch` to `result`, each under the conjunction of `guard` and its
    /// condition.
    fn guarded(
        &mut self,
        guard: DataExpression,
        branch: &ProcessExpressionRef<'_>,
        scope: &[DataVariable],
        result: &mut Vec<Summand>,
    ) -> Result<(), LinearisationError> {
        for mut summand in self.variable_head(branch, scope)? {
            summand.condition = optimized_and(guard.clone(), summand.condition);
            if !summand.condition.is_false() {
                result.push(summand);
            }
        }
        Ok(())
    }

    /// Appends `expr` as a sequence of calls, lifting every operand that is not a call.
    fn tail_calls(&mut self, expr: &ProcessExpressionRef<'_>, scope: &[DataVariable], tail: &mut Vec<Call>) {
        match expr.kind() {
            ProcessKind::Process(identifier, arguments) => tail.push(Call {
                process: identifier.protect(),
                arguments: arguments.to_vec(),
            }),
            ProcessKind::Seq(..) => {
                for operand in split_seq(expr) {
                    self.tail_calls(&operand, scope, tail);
                }
            }
            _ => tail.push(self.lift(&expr.protect(), scope)),
        }
    }

    /// A call to a fresh equation, over the variables of `scope` that `expr` uses, whose body is
    /// `expr`.
    fn lift(&mut self, expr: &ProcessExpression, scope: &[DataVariable]) -> Call {
        let parameters = occurring_in(expr, scope);
        let arguments = parameters.iter().cloned().map(DataExpression::from).collect();
        let key = (expr.clone(), parameters);
        let process = match self.lifted.get(&key) {
            Some(process) => process.clone(),
            None => {
                let process = self.add_equation("X", &key.1, expr.clone());
                self.lifted.insert(key, process.clone());
                process
            }
        };
        Call { process, arguments }
    }
}
