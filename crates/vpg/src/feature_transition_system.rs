//! Authors: Maurice Laveaux and Sjef van Loo

use std::collections::HashMap;
use std::fmt;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;

use log::debug;
use merc_lts::LabelIndex;
use merc_lts::StateIndex;
use merc_lts::Transition;
use oxidd::BooleanFunction;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::read_aut;
use merc_syntax::DataExpr;
use merc_syntax::MultiAction;
use merc_utilities::MercError;

/// Reads a .aut file as feature transition system by using the associated feature diagram.
///
/// # Details
///
/// The action labels of a feature transition system are annotated with a special `BDD` struct that is defined as `struct BDD = node(var, true, false) | tt | ff`.
pub fn read_fts(
    manager_ref: &BDDManagerRef,
    reader: impl Read,
    features: HashMap<String, BDDFunction>,
) -> Result<FeatureTransitionSystem, MercError> {
    // Read the underlying LTS, where the labels are in plain text
    let aut = read_aut(reader, Vec::new())?;

    // Parse the labels as data expressions
    let mut feature_labels = Vec::new();
    for label in aut.labels() {
        let action = MultiAction::parse(label)?;

        debug!("Parsed action: {}", action);
        if action.actions.len() > 1 {
            return Err(MercError::from(format!(
                "Cannot read feature transition system: action \"{}\" has multiple actions",
                label
            )));
        }

        if let Some(action) = action.actions.first() {
            if let Some(arg) = action.args.first() {
                feature_labels.push(data_expr_to_bdd(manager_ref, &features, arg)?);
            } else {
                feature_labels.push(manager_ref.with_manager_shared(|manager| BDDFunction::t(manager)));
            }
        } else {
            // This is the tau action, is always enabled.
            feature_labels.push(manager_ref.with_manager_shared(|manager| BDDFunction::t(manager)));
        }
    }

    Ok(FeatureTransitionSystem::new(aut, feature_labels, features))
}

/// Converts the given data expression into a BDD function.
///
/// The input should be a data expression of the shape: expr = node(var, expr, expr) | tt | ff.
fn data_expr_to_bdd(
    manager_ref: &BDDManagerRef,
    variables: &HashMap<String, BDDFunction>,
    expr: &DataExpr,
) -> Result<BDDFunction, MercError> {
    match expr {
        DataExpr::Application { function, arguments } => {
            match function.as_ref() {
                // A node must be of the shape 'node(var, true_branch, false_branch)'
                DataExpr::Id(name) => {
                    if name == "node" {
                        let variable = format!("{}", arguments[0]);
                        let then_branch = data_expr_to_bdd(manager_ref, variables, &arguments[1])?;
                        let else_branch = data_expr_to_bdd(manager_ref, variables, &arguments[2])?;
                        Ok(variables
                            .get(&variable)
                            .ok_or(format!("Variable \"{}\" not found in feature diagram", variable))?
                            .ite(&then_branch, &else_branch)?)
                    } else {
                        unimplemented!("Conversion of data expression to BDD not implemented for this function");
                    }
                }
                _ => unimplemented!("Conversion of data expression to BDD not implemented for this function"),
            }
        }
        DataExpr::Id(name) => {
            // Deal with the base cases.
            match name.as_str() {
                "tt" => Ok(manager_ref.with_manager_shared(|manager| BDDFunction::t(manager))),
                "ff" => Ok(manager_ref.with_manager_shared(|manager| BDDFunction::f(manager))),
                _ => unimplemented!("Cannot convert data expression \"{expr}\" to BDD"),
            }
        }
        _ => unimplemented!("Cannot convert data expression \"{expr}\" to BDD"),
    }
}

pub struct FeatureDiagram {
    /// The mapping from variable names to their BDD variable.
    features: HashMap<String, BDDFunction>,

    /// Stores the set of products as a BDD function.
    configuration: BDDFunction,
}

impl FeatureDiagram {
    /// Reads feature diagram from the input.
    ///
    /// # Details
    ///
    /// The first line is a list of variable names, separated by commas. The
    /// second line is the initial configuration, represented as a data
    /// expression. This function will initialize the BDD manager with the
    /// variables read from the first line, and assumes that the manager has no
    /// variables yet defined.
    pub fn from_reader(manager_ref: &BDDManagerRef, input: impl Read) -> Result<Self, MercError> {
        manager_ref.with_manager_exclusive(|manager| {
            debug_assert_eq!(
                manager.num_vars(),
                0,
                "A BDD manager can only hold the variables for a single feature diagram"
            )
        });

        let input = BufReader::new(input);
        let mut line_iter = input.lines();
        let first_line = line_iter.next().ok_or("Expected variable names line")??;

        let variable_names: Vec<String> = first_line.split(',').map(|s| s.trim().to_string()).collect();
        let variables = manager_ref.with_manager_exclusive(|manager| -> Result<Vec<BDDFunction>, MercError> {
            Ok(manager
                .add_named_vars(variable_names.iter())
                .map_err(|e| format!("{}", e))?
                .map(|i| BDDFunction::var(manager, i))
                .collect::<Result<Vec<_>, _>>()?)
        })?;

        let variables = HashMap::from_iter(variable_names.into_iter().zip(variables));

        let second_line = line_iter.next().ok_or("Expected initial configuration line")??;
        let initial_configuration = data_expr_to_bdd(manager_ref, &variables, &DataExpr::parse(&second_line)?)?;

        Ok(Self {
            features: variables,
            configuration: initial_configuration,
        })
    }

    /// Returns the configuration of the feature diagram.
    pub fn configuration(&self) -> &BDDFunction {
        &self.configuration
    }

    /// Returns the features used in the feature diagram.
    pub fn features(&self) -> &HashMap<String, BDDFunction> {
        &self.features
    }
}

impl fmt::Debug for FeatureDiagram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "variables = {:?}", self.features.keys())
    }
}

/// A feature transition system, i.e., a labelled transition system
/// where each label is associated with a feature expression.
pub struct FeatureTransitionSystem {
    /// The underlying labelled transition system.
    lts: LabelledTransitionSystem<String>,

    /// The feature expression associated with each label.
    feature_labels: Vec<BDDFunction>,

    /// The features associated with this feature transition system.
    features: HashMap<String, BDDFunction>,
}

impl FeatureTransitionSystem {
    /// Creates a new feature transition system.
    pub fn new(
        lts: LabelledTransitionSystem<String>,
        feature_labels: Vec<BDDFunction>,
        features: HashMap<String, BDDFunction>,
    ) -> Self {
        Self {
            lts,
            feature_labels,
            features,
        }
    }

    /// Returns the feature label BDD for the given label index.
    pub fn feature_label(&self, label_index: LabelIndex) -> &BDDFunction {
        &self.feature_labels[label_index]
    }

    /// Returns the features used in the feature diagram.
    pub fn features(&self) -> &HashMap<String, BDDFunction> {
        &self.features
    }
}

impl LTS for FeatureTransitionSystem {
    type Label = String;

    fn merge_disjoint<L>(self, _other: &L) -> (LabelledTransitionSystem<String>, StateIndex) {
        unimplemented!("Merging feature transition systems is not yet implemented")
    }

    delegate::delegate! {
        to self.lts {
            fn initial_state_index(&self) -> StateIndex;
            fn num_of_states(&self) -> usize;
            fn num_of_labels(&self) -> usize;
            fn num_of_transitions(&self) -> usize;
            fn is_hidden_label(&self, label_index: LabelIndex) -> bool;
            fn labels(&self) -> &[String];
            fn outgoing_transitions(&self, state_index: StateIndex) -> impl Iterator<Item = Transition>;
            fn iter_states(&self) -> impl Iterator<Item = StateIndex> + use<>;
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_macros::merc_test;

    use super::*;

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not support miri (specifically the crossbeam-epoch dependency)
    fn test_read_minepump_fts() {
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

        let feature_diagram = FeatureDiagram::from_reader(
            &manager_ref,
            include_bytes!("../../../examples/vpg/minepump_fts.fd") as &[u8],
        )
        .unwrap();

        let _result = read_fts(
            &manager_ref,
            include_bytes!("../../../examples/vpg/minepump_fts.aut") as &[u8],
            feature_diagram.features,
        )
        .unwrap();
    }
}
