use std::collections::HashMap;

use merc_aterm::Symb;
use merc_aterm::Term;
use merc_data::BasicSort;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::SortExpression;
use merc_syntax::Sort;
use merc_typecheck::NumberEncoding;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_and;

use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::data::equal;
use crate::passes::gnf::substitute_action;
use crate::passes::regular::RegularProcess;
use merc_lps::ActionSummand;
use merc_lps::DeadlockSummand;
use merc_lps::LinearProcess;
use merc_process::ProcessIdentifier;

/// A linear process with its initial state and the global variables its initial state uses, the
/// unit that parallel composition combines.
#[derive(Clone, Debug)]
pub(crate) struct Component {
    pub(crate) process: LinearProcess,
    pub(crate) initial_state: Vec<DataExpression>,
    pub(crate) global_variables: Vec<DataVariable>,
}

/// Values of sorts that are used for parameters whose value does not matter, which keeps the state
/// space smaller than leaving them unchanged; mirrors mCRL2's `--no-globvars`. Also provides the
/// values of the control parameter.
pub(crate) struct Representatives<'a> {
    constructors: &'a [DataFunctionSymbol],
    encoding: NumberEncoding,
}

impl<'a> Representatives<'a> {
    pub(crate) fn new(constructors: &'a [DataFunctionSymbol], encoding: NumberEncoding) -> Self {
        Representatives { constructors, encoding }
    }

    /// The control state `index` as a `Pos`.
    pub(crate) fn state(&self, index: usize) -> DataExpression {
        self.encoding.literal(index as u64, Sort::Pos)
    }

    /// A closed value of `sort`, when one is readily available: a number, `false`, or a constant
    /// constructor.
    pub(crate) fn of(&self, sort: &SortExpression) -> Option<DataExpression> {
        if sort.get_head_symbol().name() == "SortId" {
            match sort.arg(0).get_head_symbol().name() {
                "Bool" => return Some(DataExpression::false_()),
                "Pos" => return Some(self.encoding.literal(1, Sort::Pos)),
                "Nat" => return Some(self.encoding.literal(0, Sort::Nat)),
                "Int" => return Some(self.encoding.literal(0, Sort::Int)),
                "Real" => return Some(self.encoding.literal(0, Sort::Real)),
                _ => {}
            }
        }
        self.constructors
            .iter()
            .find(|constructor| constructor.sort() == sort.copy())
            .map(|constructor| constructor.clone().into())
    }
}

/// The encoding pass: turns equations in regular form into one linear process
/// whose parameters are the union of their parameters, plus a control parameter that records which
/// equation is active when there is more than one.
///
/// `processes` must contain every equation reachable from `processes[0]`, the root, which starts
/// with `initial_arguments`.
pub(crate) fn encode(
    processes: Vec<RegularProcess>,
    initial_arguments: Vec<DataExpression>,
    names: &mut NameGenerator,
    representatives: &Representatives<'_>,
) -> Component {
    // Parameters with the same name and sort share a slot; a parameter whose name is taken by one
    // of a different sort is renamed.
    let mut parameters: Vec<DataVariable> = Vec::new();
    let processes: Vec<(RegularProcess, Vec<DataVariable>)> = processes
        .into_iter()
        .map(|mut process| {
            let mut own = process.id.parameters().to_vec();
            let mut renaming = Substitution::default();
            for parameter in &mut own {
                if parameters.contains(parameter) {
                    continue;
                }
                if parameters.iter().any(|existing| existing.name() == parameter.name()) {
                    let fresh = names.variable(parameter);
                    renaming.insert(parameter.clone(), DataExpression::from(fresh.clone()));
                    *parameter = fresh;
                }
                parameters.push(parameter.clone());
            }
            if !renaming.is_empty() {
                for summand in &mut process.summands {
                    let (sums, inner) = renaming.bind(&summand.sums, names);
                    summand.sums = sums;
                    summand.condition = inner.apply(&summand.condition, names);
                    if let Some(action) = &mut summand.action {
                        *action = substitute_action(action, &inner, names);
                    }
                    if let Some(next) = &mut summand.next {
                        next.arguments = inner.apply_all(&next.arguments, names);
                    }
                }
            }
            (process, own)
        })
        .collect();

    let terminating = processes
        .iter()
        .flat_map(|(process, _)| &process.summands)
        .any(|summand| summand.action.is_some() && summand.next.is_none());
    let control = (processes.len() > 1 || terminating)
        .then(|| DataVariable::with_sort(names.fresh("s").as_str(), pos_sort().copy()));
    let index: HashMap<ProcessIdentifier, usize> = processes
        .iter()
        .enumerate()
        .map(|(index, (process, _))| (process.id.clone(), index + 1))
        .collect();
    // A summand that terminates moves to this state, which has no summands.
    let stopped = processes.len() + 1;

    let mut avoid = Substitution::default();
    for parameter in parameters.iter().chain(&control) {
        avoid.avoid(parameter.name());
    }

    let parameters_of: HashMap<ProcessIdentifier, Vec<DataVariable>> = processes
        .iter()
        .map(|(process, own)| (process.id.clone(), own.clone()))
        .collect();
    let root = processes[0].0.id.clone();

    let mut action_summands = Vec::new();
    let mut deadlock_summands = Vec::new();
    for (process, _) in processes {
        for summand in process.summands {
            // Summation variables must not shadow the parameters of the other equations.
            let (sums, sigma) = avoid.bind(&summand.sums, names);
            let condition = sigma.apply(&summand.condition, names);
            let condition = match &control {
                Some(control) => optimized_and(
                    equal(
                        DataExpression::from(control.clone()),
                        representatives.state(index[&process.id]),
                    ),
                    condition,
                ),
                None => condition,
            };

            let Some(action) = summand.action else {
                deadlock_summands.push(DeadlockSummand {
                    summation_variables: sums,
                    condition,
                });
                continue;
            };
            let multi_action = substitute_action(&action, &sigma, names);

            let mut next_state = Vec::with_capacity(parameters.len() + 1);
            match &summand.next {
                Some(next) => {
                    let arguments = sigma.apply_all(&next.arguments, names);
                    let target = &parameters_of[&next.process];
                    if control.is_some() {
                        next_state.push(representatives.state(index[&next.process]));
                    }
                    for parameter in &parameters {
                        let value = match target.iter().position(|variable| variable == parameter) {
                            Some(position) => arguments[position].clone(),
                            None => representatives
                                .of(&parameter.sort().protect())
                                .unwrap_or_else(|| DataExpression::from(parameter.clone())),
                        };
                        next_state.push(value);
                    }
                }
                None => {
                    next_state.push(representatives.state(stopped));
                    next_state.extend(parameters.iter().map(|variable| DataExpression::from(variable.clone())));
                }
            }

            action_summands.push(ActionSummand {
                summation_variables: sums,
                condition,
                multi_action,
                next_state,
            });
        }
    }

    // Parameters the root does not have get a representative, or a global don't-care variable.
    let mut global_variables = Vec::new();
    let mut initial_state = Vec::with_capacity(parameters.len() + 1);
    if control.is_some() {
        initial_state.push(representatives.state(index[&root]));
    }
    let root_parameters = &parameters_of[&root];
    for parameter in &parameters {
        let value = match root_parameters.iter().position(|variable| variable == parameter) {
            Some(position) => initial_arguments[position].clone(),
            None => representatives.of(&parameter.sort().protect()).unwrap_or_else(|| {
                let dont_care = DataVariable::with_sort(names.fresh("dc").as_str(), parameter.sort());
                global_variables.push(dont_care.clone());
                DataExpression::from(dont_care.clone())
            }),
        };
        initial_state.push(value);
    }

    let mut all_parameters: Vec<DataVariable> = control.into_iter().collect();
    all_parameters.extend(parameters);
    Component {
        process: LinearProcess {
            parameters: all_parameters,
            action_summands,
            deadlock_summands,
        },
        initial_state,
        global_variables,
    }
}

fn pos_sort() -> SortExpression {
    BasicSort::new("Pos").into()
}
