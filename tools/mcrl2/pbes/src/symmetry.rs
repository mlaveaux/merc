#![allow(non_snake_case)]
/// Authors: Menno Bartels and Maurice Laveaux
/// To keep consistent with the theory we allow non-snake case names.
use std::cell::Cell;
use std::iter;

use itertools::Itertools;

use log::debug;
use log::info;
use mcrl2::ATermString;
use mcrl2::ControlFlowGraph;
use mcrl2::ControlFlowGraphVertex;
use mcrl2::DataExpression;
use mcrl2::DataVariable;
use mcrl2::Pbes;
use mcrl2::PbesExpression;
use mcrl2::PbesStategraph;
use mcrl2::SrfPbes;
use mcrl2::StategraphEquation;
use mcrl2::replace_propositional_variables;
use mcrl2::replace_variables;
use merc_io::TimeProgress;
use merc_utilities::LargeFormatter;
use merc_utilities::MercError;

use crate::clone_iterator::CloneIterator;
use crate::permutation::Permutation;
use crate::permutation::permutation_group;
use crate::permutation::permutation_group_size;

/// Implements symmetry detection for PBESs.
pub struct SymmetryAlgorithm {
    state_graph: PbesStategraph, // Needs to be kept alive while the control flow graphs are used.

    parameters: Vec<DataVariable>, // The parameters of the unified SRF PBES.

    all_control_flow_parameters: Vec<usize>, // Keeps track of all parameters identified as control flow parameters.

    srf: SrfPbes, // The SRF PBES after unifying parameters.

    /// Keep track of some progress messages.
    num_of_checked_candidates: Cell<usize>,
    progress: TimeProgress<usize>,
}

impl SymmetryAlgorithm {
    /// Does the required preprocessing to analyse symmetries in the given PBES.
    pub fn new(pbes: &Pbes, print_srf: bool) -> Result<Self, MercError> {
        // Apply various preproecessing necessary for symmetry detection
        let mut srf = SrfPbes::from(pbes)?;
        srf.unify_parameters(false, false)?;

        if print_srf {
            info!("==== SRF PBES ====");
            info!("{}", srf.to_pbes());
        }

        let parameters = if let Some(equation) = srf.equations().first() {
            equation.variable().parameters().to_vec()
        } else {
            // There are no equations, so no parameters.
            Vec::new()
        };

        info!(
            "Unified parameters: {:?}",
            parameters.iter().map(|p| (p.name(), p.sort())).format(", ")
        );

        let state_graph = PbesStategraph::run(&srf.to_pbes())?;
        let all_control_flow_parameters = state_graph
            .control_flow_graphs()
            .iter()
            .map(|cfg| variable_index(cfg))
            .collect::<Vec<_>>();

        let progress = TimeProgress::new(
            |count: usize| {
                info!("Checked {count} candidates...");
            },
            1,
        );

        Ok(Self {
            state_graph,
            all_control_flow_parameters,
            parameters,
            srf,
            progress,
            num_of_checked_candidates: Cell::new(0),
        })
    }

    /// Runs the symmetry detection algorithm.
    pub fn find_symmetries(&self, partition_data_sorts: bool) {
        let cliques = self.cliques();

        for clique in &cliques {
            info!(
                "Found clique: {:?}",
                clique.iter().format_with(", ", |i, f| f(&format_args!(
                    "cfg {} (var {})",
                    i, self.all_control_flow_parameters[*i]
                )))
            );
        }

        let mut combined_candidates =
            Box::new(iter::empty()) as Box<dyn CloneIterator<Item = (Permutation, Permutation)>>;
        let mut number_of_candidates = 1usize;

        for clique in &cliques {
            let (number_of_permutations, candidates) = self.clique_candidates(clique.clone(), partition_data_sorts);
            info!(
                "Maximum number of permutations for clique {:?}: {}",
                clique,
                LargeFormatter(number_of_permutations)
            );

            if number_of_candidates == 1 {
                combined_candidates = Box::new(candidates) as Box<dyn CloneIterator<Item = (Permutation, Permutation)>>;
            } else {
                combined_candidates = Box::new(
                    combined_candidates
                        .cartesian_product(candidates)
                        .filter(|((_, lhs_beta), (_, rhs_beta))| lhs_beta == rhs_beta)
                        .map(|((lhs_alpha, beta), (rhs_alpha, _))| (lhs_alpha.concat(&rhs_alpha), beta)),
                ) as Box<dyn CloneIterator<Item = (Permutation, Permutation)>>;
            }

            number_of_candidates *= number_of_permutations;
        }

        info!(
            "Maximum number of symmetry candidates: {}",
            LargeFormatter(number_of_candidates)
        );

        for (alpha, beta) in combined_candidates {
            let permutation = alpha.concat(&beta);
            info!("Found candidate: {}", permutation);

            if self.check_symmetry(&permutation) {
                info!("Found symmetry: {}", permutation);
            }
        }
    }

    /// Performs the syntactic check defined as symcheck in the paper.
    pub fn check_symmetry(&self, pi: &Permutation) -> bool {
        for equation in self.srf.equations() {
            for summand in equation.summands() {
                let mut matched = false;
                for other_equation in self.srf.equations() {
                    for other_summand in other_equation.summands() {
                        if equation.variable().name() == other_equation.variable().name()
                            && apply_permutation(&summand.condition(), &self.parameters, &pi)
                                == other_summand.condition()
                            && apply_permutation(&summand.variable(), &self.parameters, &pi) == other_summand.variable()
                        {
                            matched = true;
                            break;
                        }
                    }

                    if matched {
                        break;
                    }
                }

                if !matched {
                    debug!(
                        "No matching summand found for {summand:?} in equation {:?}.",
                        equation.variable().name()
                    );
                    return false;
                }
            }
        }

        true
    }

    /// Determine the cliques in the given control flow graphs.
    fn cliques(&self) -> Vec<Vec<usize>> {
        let mut cal_I = Vec::new();

        for (i, cfg) in self.state_graph.control_flow_graphs().iter().enumerate() {
            if cal_I.iter().any(|clique: &Vec<usize>| clique.contains(&i)) {
                // Skip every graph that already belongs to a clique.
                continue;
            }

            // For every other control flow graph check if it is compatible, and start a new clique
            let mut clique = vec![i];
            for j in (i + 1)..self.state_graph.control_flow_graphs().len() {
                if let Err(reason) = self.compatible(cfg, &self.state_graph.control_flow_graphs()[j]) {
                    info!("Incompatible CFGs at indices {} and {}: {}", i, j, reason);
                } else {
                    clique.push(j);
                }
            }

            if clique.len() > 1 {
                cal_I.push(clique);
            }
        }

        cal_I
    }

    /// Computes the set of candidates we can derive from a single clique
    fn clique_candidates(
        &self,
        I: Vec<usize>,
        partition_data_sorts: bool,
    ) -> (usize, Box<dyn CloneIterator<Item = (Permutation, Permutation)> + '_>) {
        // Determine the parameter indices involved in the clique
        let control_flow_parameter_indices: Vec<usize> = I
            .iter()
            .map(|&i| {
                let cfg = &self.state_graph.control_flow_graphs()[i];
                variable_index(cfg)
            })
            .collect();

        info!("Parameter indices in clique: {:?}", control_flow_parameter_indices);

        // Groups the data parameters by their sort.
        let (mut number_of_permutations, all_data_groups) = if partition_data_sorts {
            let same_sort_parameters = {
                let mut result: Vec<Vec<DataVariable>> = Vec::new();

                for (index, param) in self.parameters.iter().enumerate() {
                    if self.all_control_flow_parameters.contains(&index) {
                        // Skip control flow parameters.
                        continue;
                    }

                    let sort = param.sort();
                    if let Some(group) = result.iter_mut().find(|g: &&mut Vec<_>| {
                        if let Some(first) = g.first() {
                            first.sort() == sort
                        } else {
                            false
                        }
                    }) {
                        group.push(param.clone());
                    } else {
                        result.push(vec![param.clone()]);
                    }
                }
                result
            };

            let mut number_of_permutations = 1usize;
            let mut all_data_groups: Box<dyn CloneIterator<Item = Permutation>> = Box::new(iter::empty()); // Default value is overwritten in first iteration.
            for group in same_sort_parameters {
                // Determine the indices of these parameters.
                let parameter_indices: Vec<usize> = group
                    .iter()
                    .map(|param| self.parameters.iter().position(|p| p.name() == param.name()).unwrap())
                    .collect();

                info!(
                    "Same sort data parameters: {:?}, indices: {:?}",
                    group, parameter_indices
                );

                // Compute the product of the current data group with the already concatenated ones.
                if number_of_permutations == 1 {
                    all_data_groups = Box::new(permutation_group(parameter_indices.clone()))
                        as Box<dyn CloneIterator<Item = Permutation>>;
                } else {
                    all_data_groups = Box::new(
                        all_data_groups
                            .cartesian_product(permutation_group(parameter_indices.clone()))
                            .map(|(a, b)| a.concat(&b)),
                    ) as Box<dyn CloneIterator<Item = Permutation>>;
                }

                number_of_permutations *= permutation_group_size(parameter_indices.len());
            }

            (number_of_permutations, all_data_groups)
        } else {
            // All data parameters in a single group.
            let parameter_indices: Vec<usize> = (0..self.parameters.len())
                .filter(|i| !self.all_control_flow_parameters.contains(i))
                .collect();

            info!("All data parameter indices: {:?}", parameter_indices);

            let number_of_permutations = permutation_group_size(parameter_indices.len());
            let all_data_groups =
                Box::new(permutation_group(parameter_indices.clone())) as Box<dyn CloneIterator<Item = Permutation>>;

            (number_of_permutations, all_data_groups)
        };

        number_of_permutations *= permutation_group_size(control_flow_parameter_indices.len());

        (
            number_of_permutations,
            Box::new(
                permutation_group(control_flow_parameter_indices)
                    .cartesian_product(all_data_groups)
                    .filter(move |(a, b)| {
                        let pi = a.clone().concat(&b);

                        // Print progress messages.
                        self.num_of_checked_candidates
                            .set(self.num_of_checked_candidates.get() + 1);
                        self.progress.print(self.num_of_checked_candidates.get());

                        if !self.complies(&pi, &I) {
                            debug!("Non compliant permutation {}.", pi);
                            return false;
                        }

                        true
                    }),
            ) as Box<dyn CloneIterator<Item = (Permutation, Permutation)>>,
        )
    }

    /// Returns true iff the two control flow graphs are compatible.
    fn compatible(&self, c: &ControlFlowGraph, c_prime: &ControlFlowGraph) -> Result<(), MercError> {
        // First check whether the vertex sets are compatible.
        if let Err(x) = self.vertex_sets_compatible(c, c_prime) {
            return Err(format!("Incompatible vertex sets.\n {x}").into());
        }

        for s in c.vertices() {
            let mut s_matched = false;
            for s_c_prime in c_prime.vertices() {
                // Check whether there is a matching value in c' for every value in c.
                if s.value() == s_c_prime.value() && s.name() == s_c_prime.name() {
                    s_matched = true;

                    // There exist t such that s_c and s'_c' match according to the definitions in the paper.
                    for s_prime in c.vertices() {
                        for s_c_prime_prime in c_prime.vertices() {
                            // Y(v) in c and Y(v) in c_prime.
                            if s_prime.value() == s_c_prime_prime.value() && s_prime.name() == s_c_prime_prime.name() {
                                let v_c = s.outgoing_edges().iter().find(|(vertex, _)| *vertex == s_prime.get());
                                let v_c_prime = s_c_prime
                                    .outgoing_edges()
                                    .iter()
                                    .find(|(vertex, _)| *vertex == s_c_prime_prime.get());

                                if v_c.is_none() != v_c_prime.is_none() {
                                    return Err("Could not match outgoing edges.".into());
                                }

                                if let Some((_, edges)) = v_c {
                                    if let Some((_, edges_prime)) = v_c_prime {
                                        if edges.len() != edges_prime.len() {
                                            return Err(format!(
                                                "Found different number of outgoing edges ({} != {}).",
                                                edges.len(),
                                                edges_prime.len()
                                            )
                                            .into());
                                        }
                                    }
                                }

                                if self.sizes(c, s, s_prime) != self.sizes(c_prime, s_c_prime, s_c_prime_prime) {
                                    return Err("Different sizes of outgoing edges.".into());
                                }
                            }
                        }
                    }
                }
            }

            if !s_matched {
                return Err(format!("No matching vertex found in c' for vertex {:?}.", s).into());
            }
        }

        Ok(())
    }

    /// Checks whether two control flow graphs have compatible vertex sets, meaning that the PVI and values of the
    /// vertices match.
    fn vertex_sets_compatible(&self, c: &ControlFlowGraph, c_prime: &ControlFlowGraph) -> Result<(), MercError> {
        if c.vertices().len() != c_prime.vertices().len() {
            return Err(format!(
                "Different number of vertices ({} != {}).",
                c.vertices().len(),
                c_prime.vertices().len()
            )
            .into());
        }

        for vertex in c.vertices() {
            if !c_prime
                .vertices()
                .iter()
                .any(|vertex_prime| vertex.name() == vertex_prime.name() && vertex.value() == vertex_prime.value())
            {
                return Err(format!("Vertex {:?} has no matching vertex in the c' CFG.", vertex,).into());
            }
        }

        for vertex_prime in c_prime.vertices() {
            if !c
                .vertices()
                .iter()
                .any(|vertex| vertex.name() == vertex_prime.name() && vertex.value() == vertex_prime.value())
            {
                return Err(format!("Vertex {:?} has no matching vertex in the c CFG.", vertex_prime,).into());
            }
        }

        Ok(())
    }

    /// Returns true iff all vertices in I comply with the detail::permutation pi.
    fn complies(&self, pi: &Permutation, I: &Vec<usize>) -> bool {
        I.iter()
            .all(|c| self.complies_cfg(pi, &self.state_graph.control_flow_graphs()[*c]))
    }

    /// Takes a detail::permutation and a control flow parameter and returns true or
    /// false depending on whether the detail::permutation complies with the control
    /// flow parameter.
    fn complies_cfg(&self, pi: &Permutation, c: &ControlFlowGraph) -> bool {
        let c_prime = self
            .state_graph
            .control_flow_graphs()
            .iter()
            .find(|cfg| variable_index(cfg) == pi.value(variable_index(c)))
            .expect("There should be a matching control flow graph.");

        for s in c.vertices() {
            for s_prime in c_prime.vertices() {
                if s.value() == s_prime.value() && s.name() == s_prime.name() {
                    // s == s'
                    for (to, labels) in s.outgoing_edges() {
                        for (to_prime, labels_prime) in s_prime.outgoing_edges() {
                            // TODO: This is not optimal since we are not interested in the outgoing edges, which new() computes.
                            let to = ControlFlowGraphVertex::new(*to);
                            let to_prime = ControlFlowGraphVertex::new(*to_prime);

                            if to.value() == to_prime.value() && to.name() == to_prime.name() {
                                let equation = self.find_equation_by_name(&s.name()).expect("Equation should exist");

                                // Checks whether these edges can match
                                if !self.matching_summand(equation, pi, labels, labels_prime) {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
        }

        true
    }

    /// Checks whether there is a matching summand in the equation for the given labels under the permutation pi.
    fn matching_summand(
        &self,
        equation: &StategraphEquation,
        pi: &Permutation,
        labels: &Vec<usize>,
        labels_prime: &Vec<usize>,
    ) -> bool {
        let mut remaining_j = labels_prime.clone();

        for i in labels {
            let variable = &equation.predicate_variables()[*i];

            let result = remaining_j.iter().find(|&&j| {
                let variable_prime = &equation.predicate_variables()[j];

                self.equal_under_permutation(pi, &variable.changed(), &variable_prime.changed())
                    .is_ok()
                    && self
                        .equal_under_permutation(pi, &variable.used(), &variable_prime.used())
                        .is_ok()
            });

            if let Some(x) = result {
                // Remove x from remaining_j
                let index = remaining_j
                    .iter()
                    .position(|r| r == x)
                    .expect("Element should exist since it was found before.");
                remaining_j.remove(index);
            } else {
                return false;
            }
        }

        true
    }

    /// Checks whether the data parameters of two sets are equal under the given permutation.
    fn equal_under_permutation(
        &self,
        pi: &Permutation,
        left: &Vec<usize>,
        right: &Vec<usize>,
    ) -> Result<(), MercError> {
        if left.len() != right.len() {
            return Err(format!(
                "Cannot be equal: left has size {}, right has size {}",
                left.len(),
                right.len()
            )
            .into());
        }

        // Only need to check one way since sizes are equal (and the vectors have no duplicates).
        for l in left {
            if self.all_control_flow_parameters.contains(l) {
                // Skip control flow parameters.
                continue;
            }

            let l_permuted = pi.value(*l);
            if !right.contains(&l_permuted) {
                return Err(format!("Element {} (permuted to {}) not found in right set.", l, l_permuted).into());
            }
        }

        Ok(())
    }

    /// Computes the sizes(c, s, s')
    ///
    /// TODO: used is used_for and used_in in the theory (and should be split eventually)
    fn sizes(
        &self,
        _c: &ControlFlowGraph,
        s: &mcrl2::ControlFlowGraphVertex,
        s_prime: &mcrl2::ControlFlowGraphVertex,
    ) -> Vec<(usize, usize)> {
        if let Some((_, edges)) = s.outgoing_edges().iter().find(|(vertex, _)| *vertex == s_prime.get()) {
            let mut result = Vec::new();

            let equation = self.find_equation_by_name(&s.name()).expect("Equation should exist");
            for label in edges {
                let variable = &equation.predicate_variables()[*label];
                result.push((variable.changed().len(), variable.used().len()));
            }

            // Remove duplicates
            result.sort();
            result.dedup();
            result
        } else {
            Vec::new()
        }
    }

    /// Returns the equation with the given name.
    fn find_equation_by_name(&self, name: &ATermString) -> Option<&StategraphEquation> {
        // TODO: Fix naive implementation
        for equation in self.state_graph.equations() {
            if equation.variable().name() == *name {
                return Some(equation);
            }
        }

        None
    }
}

/// Returns the index of the variable that the control flow graph considers
fn variable_index(cfg: &ControlFlowGraph) -> usize {
    // Check that all the vertices have the same variable assigned for consistency
    cfg.vertices().iter().for_each(|v| {
        if v.index()
            != cfg
                .vertices()
                .first()
                .expect("There is at least one vertex in a CFG")
                .index()
        {
            panic!("Inconsistent variable indices in control flow graph.");
        }
    });

    for v in cfg.vertices() {
        // Simply return the index of the variable
        return v.index();
    }

    panic!("No variable found in control flow graph.");
}

/// Applies the given permutation to the given expression.
///
/// # Details
///
/// - Replaces data variables according to the permutation.
/// - Replaces propositional variables according to the permutation.
fn apply_permutation(expression: &PbesExpression, parameters: &Vec<DataVariable>, pi: &Permutation) -> PbesExpression {
    let sigma: Vec<(DataExpression, DataExpression)> = (0..parameters.len())
        .map(|i| {
            let var = &parameters[i];
            let permuted_var = &parameters[pi.value(i)];

            (var.clone().into(), permuted_var.clone().into())
        })
        .collect();

    let result = replace_variables(expression, sigma);

    let pi = (0..parameters.len()).map(|i| pi.value(i)).collect::<Vec<usize>>();
    replace_propositional_variables(&result, &pi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symmetry_examples() {
        for example in &[
            include_str!("../../../../examples/pbes/a.text.pbes"),
            include_str!("../../../../examples/pbes/b.text.pbes"),
            include_str!("../../../../examples/pbes/c.text.pbes"),
        ] {
            let pbes = Pbes::from_text(example).unwrap();

            SymmetryAlgorithm::new(&pbes, false).unwrap().find_symmetries(true);
        }
    }
}
