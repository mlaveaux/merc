//! Authors: Maurice Laveaux and Sjef van Loo

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;

use oxidd::BooleanFunction;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_lts::{LTS, LabelledTransitionSystem, read_aut};
use merc_syntax::{DataExpr, MultiAction};
use merc_utilities::MercError;
use oxidd::util::OutOfMemory;

/// Reads a .aut file as feature transition system by using the associated feature diagram.
/// 
/// # Details
/// 
/// The action labels of a feature transition sytstem are annotated with a special `BDD` struct that is defined as `struct BDD = node(var, true, false) | tt | ff`.
pub fn read_fts(manager_ref: &BDDManagerRef, reader: impl Read, feature_diagram: &FeatureDiagram) -> Result<FeatureTransitionSystem, MercError> {
    // Read the underlying LTS, where the labels are in plain text
    let aut = read_aut(reader, Vec::new())?;

    // Parse the labels as data expressions
    let mut feature_labels = Vec::new();
    for label in aut.labels() {
        let action = MultiAction::parse(&label)?;

        println!("Parsed action: {}", action);
        feature_labels.push(action)
    }

    Ok(FeatureTransitionSystem::new(manager_ref, aut, feature_labels))
}

/// Converts the given data expression into a BDD function.
fn data_expr_to_bdd(manager_ref: &BDDManagerRef, expr: &DataExpr) -> Result<BDDFunction, OutOfMemory> {
    match expr {
        DataExpr::Application { function, arguments } => {
            match function.as_ref() {
                // A node must be of the shape 'node(var, true_branch, false_branch)'
                DataExpr::Id(name) => {
                    if name == "node" {
                        let _true_branch = data_expr_to_bdd(manager_ref, &arguments[0])?;
                        let _false_branch = data_expr_to_bdd(manager_ref, &arguments[1])?;
                        unimplemented!();
                        // manager_ref.with_manager_shared(|manager| {
                        //     BDDFunction::ite(manager., &true_branch, &false_branch)
                        // })
                    } else {
                        unimplemented!("Conversion of data expression to BDD not implemented for this function");
                    }        
                }
                _ => unimplemented!("Conversion of data expression to BDD not implemented for this function"),
            }
        }
        _ => unimplemented!("Conversion of data expression to BDD not implemented for this expression"),
    }
}

pub struct FeatureDiagram {
    /// The variable names
    variables: Vec<BDDFunction>,

    initial_configuration: BDDFunction,
}

impl FeatureDiagram {
    
    /// Reads feature diagram from the input.
    /// 
    /// # Details
    /// 
    /// The first line is a list of variable names, separated by commas.
    /// The second line is the initial configuration, represented as a data expression.
    pub fn from_reader(manager_ref: &BDDManagerRef, input: impl Read) -> Result<Self, MercError> {
        let input = BufReader::new(input);
        let mut line_iter = input.lines();
        let first_line = line_iter.next().ok_or("Expected variable names line")??;

        let variable_names = first_line
            .split(',')
            .map(|s| s.trim().to_string());

        let variables = manager_ref.with_manager_exclusive(|manager| {
            manager.add_named_vars(variable_names)
                .expect("The input should not have duplicated variable names") // TODO: This should be returned as an error, but that can only have OutOfMemory.                 
                .map(|i| BDDFunction::var(manager, i))
                .collect::<Result<Vec<_>, _>>()
        })?;

        let second_line = line_iter.next().ok_or("Expected initial configuration line")??;
        let initial_configuration = data_expr_to_bdd(manager_ref, &DataExpr::parse(&second_line)?)?;

        Ok(Self {
            variables,
            initial_configuration
        })
    }
}

/// A feature transition system, i.e., a labelled transition system
/// where each label is associated with a feature expression.
pub struct FeatureTransitionSystem {
    
    /// The underlying labelled transition system.
    lts: LabelledTransitionSystem,

    /// The feature expression associated with each label.
    feature_label: Vec<MultiAction>,
}

impl FeatureTransitionSystem {
    /// Creates a new feature transition system.
    pub fn new(manager: &BDDManagerRef, lts: LabelledTransitionSystem, feature_label: Vec<MultiAction>) -> Self {
        Self { lts, feature_label }
    }
}


#[cfg(test)]
mod tests {
    use merc_macros::merc_test;

    use super::*;

    #[merc_test]
    fn test_read_minepump_fts() {
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

        let feature_diagram = FeatureDiagram::from_reader(&manager_ref, include_bytes!("../../../examples/vpg/minepump_fts.fd") as &[u8]).unwrap();

        let _result = read_fts(&manager_ref, include_bytes!("../../../examples/vpg/minepump_fts.aut") as &[u8], &feature_diagram).unwrap();
    }
}