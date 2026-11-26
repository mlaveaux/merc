#![allow(non_snake_case)]
/// To keep consistent with the theory we allow non-snake case names.
use std::cell::Cell;
use std::collections::HashSet;
use std::iter;

use itertools::Itertools;
use log::debug;
use log::info;
use log::trace;

use mcrl2::ATerm;
use mcrl2::ATermInt;
use mcrl2::ATermString;
use mcrl2::DataExpression;
use mcrl2::DataVariable;
use mcrl2::Mcrl2ControlFlowGraph;
use mcrl2::Mcrl2ControlFlowGraphVertex;
use mcrl2::Pbes;
use mcrl2::PbesExpression;
use mcrl2::PbesStategraph;
use mcrl2::SrfPbes;
use mcrl2::StategraphEquation;
use mcrl2::Symbol;
use mcrl2::pbes_expression_pvi;
use mcrl2::reorder_propositional_variables;
use mcrl2::substitute_data_expressions;
use mcrl2::substitute_variables;
use mcrl2::variable_occurrences_data_expression;
use merc_io::LargeFormatter;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::clone_iterator::CloneIterator;
use crate::explore_common::UNIFY_IGNORE_CE_EQUATIONS;
use crate::explore_common::UNIFY_RESET_PARAMETERS;
use crate::permutation::Permutation;
use crate::permutation::permutation_group;
use crate::permutation::permutation_group_size;

/// Implements symmetry detection for PBESs.
pub struct SymmetryAlgorithm {
    /// Needs to be kept alive while the control flow graphs are used.
    state_graph: PbesStategraph,

    /// The parameters of the unified SRF PBES.
    parameters: Vec<DataVariable>,

    /// The indices of all control flow parameters in the PBES.
    all_control_flow_parameters: Vec<usize>,

    /// The SRF PBES after unifying parameters.
    srf: SrfPbes,

    /// Keep track of some progress messages.
    num_of_checked_candidates: Cell<usize>,
    progress: TimeProgress<usize>,
}

impl SymmetryAlgorithm {
    /// Does the required preprocessing to analyse symmetries in the given PBES.
    pub fn new(pbes: &Pbes, print_srf: bool) -> Result<Self, MercError> {
        let (srf, parameters, state_graph) = preprocess_symmetry(pbes, print_srf)?;

        let all_control_flow_parameters = state_graph
            .control_flow_graphs()
            .iter()
            .map(variable_index)
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

    /// Returns the SRF PBES after unifying parameters.
    pub fn srf_pbes(&self) -> &SrfPbes {
        &self.srf
    }

    /// Returns the state graph of the PBES.
    pub fn state_graph(&self) -> &PbesStategraph {
        &self.state_graph
    }

    /// Returns compliant permutations.
    ///
    /// See `Self::clique_candidates` for the parameters.
    pub fn candidates(
        &self,
        partition_data_sorts: bool,
        partition_data_updates: bool,
    ) -> impl Iterator<Item = Permutation> + '_ {
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
            let (number_of_permutations, candidates) =
                self.clique_candidates(clique.clone(), partition_data_sorts, partition_data_updates);
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

            // If the number overflows we probably don't really care.
            number_of_candidates = number_of_candidates.saturating_mul(number_of_permutations);
        }

        info!(
            "Maximum number of symmetry candidates: {}",
            LargeFormatter(number_of_candidates)
        );

        combined_candidates.map(|(alpha, beta)| alpha.concat(&beta))
    }

    /// Checks whether the given permutation is valid, meaning that control flow parameters are mapped to control flow parameters.
    pub fn is_valid_permutation(&self, pi: &Permutation) -> Result<(), MercError> {
        // Check that all control flow parameters are mapped to control flow parameters.
        for index in pi.domain() {
            let mapped_index = pi.value(index);
            if self.all_control_flow_parameters.contains(&index)
                != self.all_control_flow_parameters.contains(&mapped_index)
            {
                return Err(format!(
                    "A parameter at index {} is mapped to parameter at index {}, but they are not both control flow parameters.",
                    index, mapped_index
                ).into());
            }

            if index >= self.parameters.len() || mapped_index >= self.parameters.len() {
                return Err(format!(
                    "A parameter at index {} is mapped to parameter at index {}, but the PBES only has {} parameters.",
                    index,
                    mapped_index,
                    self.parameters.len()
                )
                .into());
            }
        }

        Ok(())
    }

    /// Performs the syntactic check defined as symcheck in the paper.
    pub fn check_symmetry(&self, pi: &Permutation) -> bool {
        for equation in self.srf.equations() {
            for summand in equation.summands() {
                let mut matched = false;
                for other_equation in self.srf.equations() {
                    for other_summand in other_equation.summands() {
                        if equation.variable().name() == other_equation.variable().name()
                            && apply_permutation(&summand.condition(), &self.parameters, pi)
                                == other_summand.condition()
                            && apply_permutation(&summand.variable(), &self.parameters, pi) == other_summand.variable()
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
    pub fn cliques(&self) -> Vec<Vec<usize>> {
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
                    info!("Incompatible CFGs at indices {} and {}: \n\t - {}", i, j, reason);
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
        partition_data_updates: bool,
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

        let data_parameter_partition = if partition_data_sorts {
            // Groups the data parameters by their sort.
            partition(
                self.parameters.iter().enumerate().filter_map(|(index, param)| {
                    if self.all_control_flow_parameters.contains(&index) {
                        // Skip control flow parameters.
                        None
                    } else {
                        Some(param)
                    }
                }),
                |lhs, rhs| lhs.sort() == rhs.sort(),
            )
        } else {
            let groups: Vec<&DataVariable> = self
                .parameters
                .iter()
                .enumerate()
                .filter_map(|(index, param)| {
                    if self.all_control_flow_parameters.contains(&index) {
                        // Skip control flow parameters.
                        None
                    } else {
                        Some(param)
                    }
                })
                .collect();

            if groups.is_empty() {
                // No data parameters.
                Vec::new()
            } else {
                // All data parameters in a single group.
                vec![groups]
            }
        };

        let data_parameter_partition = if partition_data_updates {
            let mut parameter_updates = vec![HashSet::new(); self.parameters.len()];

            // Figure out all the PVIs in which the parameter is updated.
            for equation in self.srf.equations() {
                for summand in equation.summands() {
                    for pvi in pbes_expression_pvi(&summand.variable().copy()) {
                        for (index, param) in pvi.arguments().protect().cast::<DataExpression>().iter().enumerate() {
                            parameter_updates[index].insert(replace_variables_by_omega(&param));
                        }
                    }
                }
            }

            for (index, param) in self.parameters.iter().enumerate() {
                debug!(
                    "Parameter {} is updated with expressions: {}",
                    param.name(),
                    parameter_updates[index].iter().map(|expr| expr.to_string()).join(", ")
                );
            }

            let mut update_partition = Vec::new();
            for group in data_parameter_partition {
                update_partition.extend(partition(group.iter().cloned(), |lhs, rhs| {
                    parameter_updates[self
                        .parameters
                        .iter()
                        .position(|p| p.name() == lhs.name())
                        .expect("The parameter is declared")]
                        == parameter_updates[self
                            .parameters
                            .iter()
                            .position(|p| p.name() == rhs.name())
                            .expect("The parameter is declared")]
                }));
            }

            update_partition
        } else {
            // Do nothing
            data_parameter_partition
        };

        // For progress messages keep track of the number of permutations we need to check.
        let mut number_of_permutations = 1usize;

        let mut all_data_groups: Box<dyn CloneIterator<Item = Permutation>> =
            Box::new(iter::once(Permutation::from_mapping(Vec::new()))); // Default value is overwritten in first iteration.
        for group in data_parameter_partition {
            // Determine the indices of these parameters.
            let parameter_indices: Vec<usize> = group
                .iter()
                .map(|param| {
                    self.parameters
                        .iter()
                        .position(|p| p.name() == param.name())
                        .expect("The parameter is declared")
                })
                .collect();

            info!("Data parameters group: {:?}, indices: {:?}", group, parameter_indices);

            // Compute the product of the current data group with the already concatenated ones.
            let number_of_parameters = parameter_indices.len();
            if number_of_permutations == 1 {
                all_data_groups =
                    Box::new(permutation_group(parameter_indices)) as Box<dyn CloneIterator<Item = Permutation>>;
            } else {
                all_data_groups = Box::new(
                    all_data_groups
                        .cartesian_product(permutation_group(parameter_indices))
                        .map(|(a, b)| a.concat(&b)),
                ) as Box<dyn CloneIterator<Item = Permutation>>;
            }

            number_of_permutations =
                number_of_permutations.saturating_mul(permutation_group_size(number_of_parameters));
        }

        number_of_permutations =
            number_of_permutations.saturating_mul(permutation_group_size(control_flow_parameter_indices.len()));

        (
            number_of_permutations,
            Box::new(
                permutation_group(control_flow_parameter_indices)
                    .cartesian_product(all_data_groups)
                    .filter(move |(a, b)| {
                        let pi = a.clone().concat(b);

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
    fn compatible(&self, c: &Mcrl2ControlFlowGraph, c1: &Mcrl2ControlFlowGraph) -> Result<(), MercError> {
        // First check whether the vertex sets are compatible.
        if let Err(x) = self.vertex_sets_compatible(c, c1) {
            return Err(format!("Incompatible vertex sets.\n {x}").into());
        }

        // Note that this algorithm is slightly different than the pseudocode, because the graphs in the implementation are
        // over different (compatible) vertex sets.
        for s_c in c.vertices() {
            let mut s_matched = false;
            // There exist s_c' such that s and s_c' match according to the definitions in the paper.
            for s_c1 in c1.vertices() {
                // X(v) in c and X(v) in c1.
                if s_c.value() == s_c1.value() && s_c.name() == s_c1.name() {
                    self.find_compatible(c, c1, s_c, s_c1)?;
                    s_matched = true;
                }
            }

            if !s_matched {
                return Err(format!("No matching vertex found in c' for vertex {:?}.", s_c).into());
            }
        }

        Ok(())
    }

    // Find pairs of vertices (s_c, s'_c) and (s_c', s'_c') and checks whether they are compatible according to the definition in the paper.
    fn find_compatible(
        &self,
        c: &Mcrl2ControlFlowGraph,
        c1: &Mcrl2ControlFlowGraph,
        s_c: &Mcrl2ControlFlowGraphVertex,
        s_c1: &Mcrl2ControlFlowGraphVertex,
    ) -> Result<(), MercError> {
        // There exist s' such that s'_c and s'_c' match according to the definitions in the paper.
        let mut matched = false;
        for s1_c in c.vertices() {
            for s1_c1 in c1.vertices() {
                // Y(v) in c and Y(v) in c_prime.
                if s1_c.value() == s1_c1.value() && s1_c.name() == s1_c1.name() {
                    matched = true;
                    trace!(
                        "Checking edges between vertices {:?} and {:?} in c, and {:?} and {:?} in c'.",
                        s_c, s_c1, s1_c, s1_c1
                    );

                    if let Err(value) = self.check_compatible(s_c, s1_c, s_c1, s1_c1) {
                        return Err(format!(
                            "Incompatible edges between vertices {:?} and {:?}: \n\t - {}",
                            s_c, s1_c, value
                        )
                        .into());
                    }
                }
            }
        }

        if !matched {
            return Err("Could not find matching edges.".into());
        }

        Ok(())
    }

    /// Checks whether edges (s_c, s'_c) and (s_c', s'_c') are compatible according to the definition in the paper.
    fn check_compatible(
        &self,
        s_c: &Mcrl2ControlFlowGraphVertex,
        s1_c: &Mcrl2ControlFlowGraphVertex,
        s_c1: &Mcrl2ControlFlowGraphVertex,
        s1_c1: &Mcrl2ControlFlowGraphVertex,
    ) -> Result<(), MercError> {
        let edges_c = s_c.outgoing_edges().iter().find(|(vertex, _)| *vertex == s1_c.get());
        let edges_c1 = s_c1.outgoing_edges().iter().find(|(vertex, _)| *vertex == s1_c1.get());

        if edges_c.is_none() != edges_c1.is_none() {
            return Err("Could not match outgoing edges.".into());
        }

        if let Some((_, edges)) = edges_c {
            debug_assert!(edges_c1.is_some(), "Both v_c and v_c' should be Some or None.");
            if let Some((_, edges_prime)) = edges_c1 {
                if edges.len() != edges_prime.len() {
                    return Err(format!(
                        "Found different number of outgoing edges ({} != {}).",
                        edges.len(),
                        edges_prime.len()
                    )
                    .into());
                }

                if self.sizes(s_c, s1_c) != self.sizes(s_c1, s1_c1) {
                    return Err("Different sizes of outgoing edges.".into());
                }
            }
        }

        Ok(())
    }

    /// Checks whether two control flow graphs have compatible vertex sets, meaning that the PVI and values of the
    /// vertices match. Returns Ok when the check succeeds, and an Err with a reason otherwise.
    fn vertex_sets_compatible(
        &self,
        c: &Mcrl2ControlFlowGraph,
        c_prime: &Mcrl2ControlFlowGraph,
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
    fn complies(&self, pi: &Permutation, I: &[usize]) -> bool {
        I.iter()
            .all(|c| self.complies_cfg(pi, &self.state_graph.control_flow_graphs()[*c]))
    }

    /// Takes a detail::permutation and a control flow parameter and returns true or
    /// false depending on whether the detail::permutation complies with the control
    /// flow parameter.
    fn complies_cfg(&self, pi: &Permutation, c: &Mcrl2ControlFlowGraph) -> bool {
        let c1 = self
            .state_graph
            .control_flow_graphs()
            .iter()
            .find(|cfg| variable_index(cfg) == pi.value(variable_index(c)))
            .expect("There should be a matching control flow graph.");

        for s_c in c.vertices() {
            for s_c1 in c1.vertices() {
                if s_c.value() == s_c1.value() && s_c.name() == s_c1.name() {
                    // s == s'
                    for (to_c, labels) in s_c.outgoing_edges() {
                        for (to_c1, labels_prime) in s_c1.outgoing_edges() {
                            // `find_by_ptr` re-derives what `new()` already computed for the
                            // outgoing edges, which this loop does not need.
                            let to = c.find_by_ptr(*to_c);
                            let to_prime = c1.find_by_ptr(*to_c1);

                            if to.value() == to_prime.value() && to.name() == to_prime.name() {
                                let equation = self.find_equation_by_name(&s_c.name()).expect("Equation should exist");

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
        labels: &[usize],
        labels_prime: &[usize],
    ) -> bool {
        let mut remaining_j = labels_prime.to_vec();

        for i in labels {
            let variable = &equation.predicate_variables()[*i];

            let result = remaining_j.iter().find(|&&j| {
                let variable_prime = &equation.predicate_variables()[j];

                self.equal_under_permutation(pi, variable.changed(), variable_prime.changed())
                    .is_ok()
                    && self
                        .equal_under_permutation(pi, variable.used(), variable_prime.used())
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
    fn equal_under_permutation(&self, pi: &Permutation, left: &[usize], right: &[usize]) -> Result<(), MercError> {
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
    /// `variable.used()` here combines what the theory calls `used_for` and `used_in`.
    fn sizes(
        &self,
        s: &mcrl2::Mcrl2ControlFlowGraphVertex,
        s1: &mcrl2::Mcrl2ControlFlowGraphVertex,
    ) -> Vec<(usize, usize)> {
        if let Some((_, edges)) = s.outgoing_edges().iter().find(|(vertex, _)| *vertex == s1.get()) {
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
            panic!("No outgoing edges found from {:?} to {:?}.", s, s1);
        }
    }

    /// Returns the equation with the given name.
    ///
    /// Linear scan over all equations.
    fn find_equation_by_name(&self, name: &ATermString) -> Option<&StategraphEquation> {
        self.state_graph
            .equations()
            .iter()
            .find(|&equation| equation.variable().name() == *name)
    }
}

/// Applies the necessary preprocessing steps to use in the symmetry algorithm.
fn preprocess_symmetry(
    pbes: &Pbes,
    print_srf: bool,
) -> Result<(SrfPbes, Vec<DataVariable>, PbesStategraph), MercError> {
    let mut srf = SrfPbes::from(pbes)?;
    srf.unify_parameters(UNIFY_IGNORE_CE_EQUATIONS, UNIFY_RESET_PARAMETERS)?;
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
    info!("Unified parameters: {}", parameters.iter().format(", "));
    let state_graph = {
        let mut pbes = srf.to_pbes();
        pbes.normalize();
        debug_assert!(pbes.is_well_typed(), "PBES should be well-typed after normalization.");
        PbesStategraph::run(&pbes)?
    };

    Ok((srf, parameters, state_graph))
}

/// Partition a vector into a number of sets based on a predicate.
fn partition<T, I, P>(elements: I, predicate: P) -> Vec<Vec<T>>
where
    I: Iterator<Item = T>,
    P: Fn(&T, &T) -> bool,
    T: Clone,
{
    let mut result: Vec<Vec<T>> = Vec::new();

    for element in elements {
        // See if the element can be added to an existing group, by taking the first element of
        // each group as representative.
        if let Some(group) = result.iter_mut().find(|g: &&mut Vec<_>| {
            if let Some(first) = g.first() {
                predicate(first, &element)
            } else {
                false
            }
        }) {
            // Add to existing group
            group.push(element.clone());
        } else {
            // Create new group
            result.push(vec![element.clone()]);
        }
    }

    result
}

/// Replaces all variables in the expression by omega.
///
/// NOTE: the `omega` placeholder is built directly as an `OpId(omega, SortId(@NoValue), 0)`
/// aterm, which is tightly coupled to the internal mCRL2 data-expression
/// representation. If that representation changes this construction must be
/// updated in lockstep.
fn replace_variables_by_omega(expression: &DataExpression) -> DataExpression {
    let variables = variable_occurrences_data_expression(&expression.copy());

    // Generate an omega variable.
    let omega = DataExpression::from(ATerm::with_args(
        &Symbol::new("OpId", 3),
        &[
            // Identifier
            ATerm::constant(&Symbol::new("omega", 0)),
            // Sort
            ATerm::with_args(
                &Symbol::new("SortId", 1),
                &[ATerm::constant(&Symbol::new("@NoValue", 0))],
            ),
            // Index
            ATermInt::with_value(0).into(),
        ],
    ));

    let sigma = variables
        .iter()
        .map(|var| (var.clone().into(), omega.clone()))
        .collect::<Vec<(DataExpression, DataExpression)>>();

    substitute_variables(&expression.copy(), sigma)
}

/// A constant representing an undefined vertex.
const UNDEFINED_VERTEX: usize = usize::MAX;

/// Returns the index of the variable that the control flow graph represents.
pub fn variable_index(cfg: &Mcrl2ControlFlowGraph) -> usize {
    // Find the first defined index
    let defined_index = cfg
        .vertices()
        .iter()
        .find(|v| v.index() != UNDEFINED_VERTEX)
        .expect("Control flow graph should have defined variable index.")
        .index();

    // Check that all the vertices have the same variable assigned for consistency
    cfg.vertices().iter().for_each(|v| {
        if v.index() != UNDEFINED_VERTEX && v.index() != defined_index {
            panic!("Inconsistent variable index {} in control flow graph.", v.index());
        }
    });

    defined_index
}

/// Applies the given permutation to the given expression.
///
/// # Details
///
/// - Replaces data variables according to the permutation.
/// - Replaces propositional variables according to the permutation.
fn apply_permutation(expression: &PbesExpression, parameters: &[DataVariable], pi: &Permutation) -> PbesExpression {
    let sigma: Vec<(DataExpression, DataExpression)> = (0..parameters.len())
        .map(|i| {
            let var = &parameters[i];
            let permuted_var = &parameters[pi.value(i)];

            (var.clone().into(), permuted_var.clone().into())
        })
        .collect();

    let result = substitute_data_expressions(expression, sigma);

    let pi = (0..parameters.len()).map(|i| pi.value(i)).collect::<Vec<usize>>();
    // `pi` comes from a Permutation over the parameter positions, so it is a
    // permutation of 0..parameters.len() by construction.
    reorder_propositional_variables(&result, &pi).expect("pi is a permutation of the parameters")
}
