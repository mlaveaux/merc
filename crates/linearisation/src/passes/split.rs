use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_process::ActionNameSet;
use merc_process::AllowSet;
use merc_process::CommunicationSet;
use merc_process::ProcessExpression;
use merc_process::ProcessIdentifier;
use merc_process::ProcessKind;
use merc_process::RenameMap;
use merc_process::allow_set;
use merc_process::communication_set;
use merc_process::name_set;
use merc_process::rename_map;
use merc_process::split_merge;

use crate::LinearisationError;
use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::passes::termination::Termination;
use crate::process::Call;
use crate::process::ProcessTable;
use crate::process::is_parallel_layer;
use crate::process::occurring_in;
use crate::process::substitute;

/// An operator on a parallel composition, with its action sets validated.
#[derive(Clone, Debug)]
pub(crate) enum Operator {
    Allow(AllowSet),
    Block(ActionNameSet),
    Hide(ActionNameSet),
    Rename(RenameMap),
    Comm(CommunicationSet),
}

/// The outer, parallel layer of a specification: parallel compositions and the operators on them,
/// with sequential processes as leaves; first as calls, then as linear processes.
#[derive(Clone, Debug)]
pub(crate) enum Composition<L> {
    /// A sequential (pCRL) process, whose successful termination is made observable.
    Leaf(L),
    Parallel(Vec<Composition<L>>),
    Operator(Operator, Box<Composition<L>>),
}

/// The split pass: separates the parallel layer of `init` from the sequential processes underneath
/// it. Calls to equations whose body is itself parallel are expanded; every other subterm becomes a
/// leaf, lifted into a fresh equation when it is not already a call.
///
/// Recurses once per operator of the parallel layer and per expanded call.
pub(crate) fn split(
    table: &mut ProcessTable,
    termination: &mut Termination,
    names: &mut NameGenerator,
    init: &ProcessExpression,
    global_variables: &[DataVariable],
) -> Result<Composition<Call>, LinearisationError> {
    let mut splitter = Splitter {
        table,
        termination,
        names,
        scope: global_variables,
        expanding: Vec::new(),
    };
    splitter.split(init)
}

struct Splitter<'a> {
    table: &'a mut ProcessTable,
    termination: &'a mut Termination,
    names: &'a mut NameGenerator,
    scope: &'a [DataVariable],
    /// The calls being expanded, to detect recursion through the parallel layer.
    expanding: Vec<ProcessIdentifier>,
}

impl Splitter<'_> {
    fn split(&mut self, expr: &ProcessExpression) -> Result<Composition<Call>, LinearisationError> {
        let (operator, operand) = match expr.kind() {
            ProcessKind::Merge(..) => {
                return Ok(Composition::Parallel(
                    split_merge(&expr.copy())
                        .iter()
                        .map(|operand| self.split(&operand.protect()))
                        .collect::<Result<_, _>>()?,
                ));
            }
            ProcessKind::Allow(multisets, operand) => (Operator::Allow(allow_set(&multisets)), operand),
            ProcessKind::Block(names, operand) => (Operator::Block(name_set(&names)), operand),
            ProcessKind::Hide(names, operand) => (Operator::Hide(name_set(&names)), operand),
            ProcessKind::Rename(renamings, operand) => (Operator::Rename(rename_map(&renamings)?), operand),
            ProcessKind::Comm(communications, operand) => {
                (Operator::Comm(communication_set(&communications)?), operand)
            }
            ProcessKind::Process(identifier, arguments) => {
                let call = Call {
                    process: identifier.protect(),
                    arguments: arguments.to_vec(),
                };
                return self.call(call);
            }
            _ => return Ok(Composition::Leaf(self.leaf(expr))),
        };
        Ok(Composition::Operator(
            operator,
            Box::new(self.split(&operand.protect())?),
        ))
    }

    fn call(&mut self, call: Call) -> Result<Composition<Call>, LinearisationError> {
        let body = self.table.body(&call.process).clone();
        if !is_parallel_layer(&body.copy()) {
            let call = self.termination.make_explicit(self.table, self.names, call);
            return Ok(Composition::Leaf(call));
        }
        if self.expanding.contains(&call.process) {
            // The parallel layer recurses, which would make the state vector unbounded.
            return Err(LinearisationError::NotParallelPcrl {
                construct: "recursion through ||",
                expression: call.expression().to_string(),
            });
        }

        let sigma = Substitution::new(call.process.parameters().iter().zip(call.arguments.iter().cloned()));
        let body = substitute(&body, &sigma, self.names);
        self.expanding.push(call.process);
        let result = self.split(&body);
        self.expanding.pop();
        result
    }

    /// A call to a fresh equation, over the variables in scope that `expr` uses, whose body is
    /// `expr`.
    fn leaf(&mut self, expr: &ProcessExpression) -> Call {
        let parameters = occurring_in(expr, self.scope);
        let process = self
            .termination
            .add(self.table, self.names, "Init", &parameters, expr.clone());
        let call = Call {
            process,
            arguments: parameters.into_iter().map(DataExpression::from).collect(),
        };
        self.termination.make_explicit(self.table, self.names, call)
    }
}
