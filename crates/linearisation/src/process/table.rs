use std::collections::HashMap;

use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_process::ProcessEquation;
use merc_process::ProcessExpression;
use merc_process::ProcessIdentifier;

use crate::data::NameGenerator;

/// A call `P(e₁, …, eₙ)`, with one argument per parameter of `P`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Call {
    pub(crate) process: ProcessIdentifier,
    pub(crate) arguments: Vec<DataExpression>,
}

impl Call {
    /// The call `P(d₁, …, dₙ)` of `P` with its own parameters.
    pub(crate) fn identity(process: ProcessIdentifier) -> Call {
        let arguments = process.parameters().iter().map(DataExpression::from).collect();
        Call { process, arguments }
    }

    pub(crate) fn expression(&self) -> ProcessExpression {
        ProcessExpression::process(&self.process, &self.arguments)
    }
}

/// The equations of a specification, and those the passes add.
///
/// Invariant: `order` contains every key of `bodies` exactly once, in order of addition.
pub(crate) struct ProcessTable {
    bodies: HashMap<ProcessIdentifier, ProcessExpression>,
    order: Vec<ProcessIdentifier>,
}

impl ProcessTable {
    pub(crate) fn new<'a, I: IntoIterator<Item = &'a ProcessEquation>>(equations: I) -> Self {
        let mut table = ProcessTable {
            bodies: HashMap::new(),
            order: Vec::new(),
        };
        for equation in equations {
            table.insert(equation.identifier.clone(), equation.body.clone());
        }
        table
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = &ProcessIdentifier> {
        self.order.iter()
    }

    pub(crate) fn body(&self, process: &ProcessIdentifier) -> &ProcessExpression {
        &self.bodies[process]
    }

    /// Adds `P(parameters) = body` for a fresh name `P` based on `hint`.
    pub(crate) fn add(
        &mut self,
        names: &mut NameGenerator,
        hint: &str,
        parameters: &[DataVariable],
        body: ProcessExpression,
    ) -> ProcessIdentifier {
        let identifier = ProcessIdentifier::new(&names.fresh(hint), parameters);
        self.insert(identifier.clone(), body);
        identifier
    }

    fn insert(&mut self, identifier: ProcessIdentifier, body: ProcessExpression) {
        let previous = self.bodies.insert(identifier.clone(), body);
        debug_assert!(previous.is_none(), "the equation of {identifier} is defined twice");
        self.order.push(identifier);
    }
}
