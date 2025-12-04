#![allow(non_snake_case)]
/// Authors: Menno Bartels and Maurice Laveaux
/// To keep consistent with the theory we allow non-snake case names.
use std::iter;

use itertools::Itertools;

use log::info;
use mcrl2::AtermString;
use mcrl2::ControlFlowGraphVertex;
use mcrl2::DataVariable;
use mcrl2::Pbes;
use mcrl2::PbesStategraph;
use mcrl2::ControlFlowGraph;
use mcrl2::SrfPbes;
use mcrl2::StategraphEquation;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::clone_iterator::CloneIterator;
use crate::permutation::permutation_group;
use crate::permutation::permutation_group_size;
use crate::permutation::Permutation;

/// Implements symmetry detection for PBESs.
pub struct SymmetryAlgorithm {
    state_graph: PbesStategraph, // Needs to be kept alive while the control flow graphs are used.

    parameters: Vec<DataVariable>, // The parameters of the unified SRF PBES.

    all_control_flow_parameters: Vec<usize>, // Keeps track of all parameters identified as control flow parameters.
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

impl SymmetryAlgorithm {
    /// Does the required preprocessing to analyse symmetries in the given PBES.
    pub fn new(pbes: &Pbes) -> Result<Self, MercError> {
        // Apply various preproecessing necessary for symmetry detection
        let mut srf = SrfPbes::from(pbes)?;
        srf.unify_parameters(false, false)?;

        info!("==== SRF PBES ====");
        info!("{}", srf.to_pbes());

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

        Ok(Self {
            state_graph,
            all_control_flow_parameters,
            parameters,
        })
    }

    /// Runs the symmetry detection algorithm.
    pub fn run(&self) {
        let cliques = self.cliques();

        for clique in &cliques {
            info!("Found clique: {:?}", clique);
        }

        let _progress = TimeProgress::new(|_: ()| {}, 1);

        for clique in &cliques {
            let (number_of_permutations, candidates) = self.clique_candidates(clique);
            info!("Number of candidate permutations: {}", number_of_permutations);

            for candidate in candidates {
                info!("Testing candidate permutation: {:?}", candidate);
            }
        }
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
    fn clique_candidates(&self, I: &Vec<usize>) -> (usize, impl Iterator<Item = Permutation>) {
        // Determine the parameter indices involved in the clique
        let parameter_indices: Vec<usize> = I
            .iter()
            .map(|&i| {
                let cfg = &self.state_graph.control_flow_graphs()[i];
                variable_index(cfg)
            })
            .collect();

        // Groups the parameters by their sort.
        let same_sort_parameters = {
            let mut result: Vec<Vec<DataVariable>> = Vec::new();
            for param in &self.parameters {
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
        let mut all_data_groups: Box<dyn CloneIterator<Item = Permutation>> = Box::new(iter::empty());
        for group in same_sort_parameters {
            info!("Group of same sort parameters: {:?}", group);

            // Determine the indices of these parameters.
            let parameter_indices: Vec<usize> = group
                .iter()
                .map(|param| self.parameters.iter().position(|p| p.name() == param.name()).unwrap())
                .collect();

            number_of_permutations *= permutation_group_size(parameter_indices.len());

            // Compute the product of the current data group with the already concatenated ones.
            all_data_groups = Box::new(
                all_data_groups
                    .cartesian_product(permutation_group(parameter_indices.clone()))
                    .map(|(a, b)| a.concat(&b)),
            ) as Box<dyn CloneIterator<Item = Permutation>>;
        }

        (
            number_of_permutations,
            permutation_group(parameter_indices)
                .cartesian_product(all_data_groups)
                .map(|(a, b)| a.concat(&b))
                .filter(|permutation| self.complies(permutation, I)),
        )
    }

    /// Returns true iff the two control flow graphs are compatible.
    fn compatible(
        &self,
        c: &ControlFlowGraph,
        c_prime: &ControlFlowGraph,
    ) -> Result<(), MercError> {
        // First check whether the vertex sets are compatible.
        if let Err(x) = self.vertex_sets_compatible(c, c_prime) {
            return Err(format!("Incompatible vertex sets.\n {x}").into());
        }

        for s in c.vertices() {

            for s_c_prime in c_prime.vertices() {
                // Check whether there is a matching value in c' for every value in c.
                if s.value() == s_c_prime.value() && s.name() == s_c_prime.name() {
                    // There exist t such that s_c and s'_c' match according to the definitions in the paper.
                    for s_prime in c.vertices() {
                        for s_c_prime_prime in c_prime.vertices() {
                            // Y(v) in c and Y(v) in c_prime.
                            if s_prime.value() == s_c_prime_prime.value()
                                && s_prime.name() == s_c_prime_prime.name() {

                                    let v_c = s.outgoing_edges().iter().find(|(vertex, _)| *vertex == s_prime.get());
                                    let v_c_prime = s_c_prime.outgoing_edges().iter().find(|(vertex, _)| *vertex == s_c_prime_prime.get());

                                    if v_c.is_none() != v_c_prime.is_none() {
                                        return Err("Could not match outgoing edges.".into());
                                    }

                                    if let Some((_, edges)) = v_c {
                                        if let Some((_, edges_prime)) = v_c_prime {
                                            if edges.len() != edges_prime.len() {
                                                return Err(format!("Found different number of outgoing edges ({} != {}).", edges.len(), edges_prime.len()).into());
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
        }

        Ok(())
    }

    /// Checks whether two control flow graphs have compatible vertex sets, meaning that the PVI and values of the
    /// vertices match.
    fn vertex_sets_compatible(
        &self,
        c: &ControlFlowGraph,
        c_prime: &ControlFlowGraph,
    ) -> Result<(), MercError> {
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
                .any(|vertex_prime| vertex.name() == vertex_prime.name() && vertex.value() == vertex_prime.value()) {
                return Err(format!(
                    "Vertex {:?} has no matching vertex in the c' CFG.",
                    vertex,
                ).into())
            }
        }

        for vertex_prime in c_prime.vertices() {
            if !c
                .vertices()
                .iter()
                .any(|vertex| vertex.name() == vertex_prime.name() && vertex.value() == vertex_prime.value()) {
                return Err(format!(
                    "Vertex {:?} has no matching vertex in the c CFG.",
                    vertex_prime,
                ).into())
            }
        }

        Ok(())
    }

    /// Returns true iff all vertices in I comply with the detail::permutation pi.
    fn complies(&self, pi: &Permutation, I: &Vec<usize>) -> bool {
        I.iter().all(|c| self.complies_cfg(pi, &self.state_graph.control_flow_graphs()[*c]))
    }

    /// Takes a detail::permutation and a control flow parameter and returns true or
    /// false depending on whether the detail::permutation complies with the control
    /// flow parameter.
    fn complies_cfg(&self, pi: &Permutation, c: &ControlFlowGraph) -> bool {
        let c_prime = self.state_graph.control_flow_graphs().iter().find(|cfg| {
            variable_index(cfg) == variable_index(c)
        }).expect("There should be a matching control flow graph.");

        for s in c.vertices() {
            for s_prime in c_prime.vertices() {
                if s.value() == s_prime.value() && s.name() == s_prime.name() {
                    // s == s'
                    for (to, _) in s.outgoing_edges() {
                        for (to_prime, _) in s_prime.outgoing_edges() {
                            let to = ControlFlowGraphVertex::new(*to);
                            let to_prime = ControlFlowGraphVertex::new(*to_prime);
                            if to.value() == to_prime.value() && to.name() == to_prime.name() {

                            }
                        }
                    }
                }
            }
        }

        true
    }


    /// Computes the sizes(c, s, s')
    /// 
    /// TODO: used is used_for and used_in in the theory (and should be split eventually)
    fn sizes(&self, _c: &ControlFlowGraph, s: &mcrl2::ControlFlowGraphVertex, s_prime: &mcrl2::ControlFlowGraphVertex) -> Vec<(usize, usize)> {        
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
    fn find_equation_by_name(&self, name: &AtermString) -> Option<&StategraphEquation> {

        // TODO: Fix naive implementation
        for equation in self.state_graph.equations() {
            if equation.variable().name() == *name {
                return Some(equation);
            }
        }

        None
    }
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

            SymmetryAlgorithm::new(&pbes).unwrap().run();
        }
    }
}
