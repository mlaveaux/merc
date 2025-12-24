use log::debug;
use log::info;
use log::trace;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_collections::IndexedSet;
use merc_io::TimeProgress;
use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_syntax::ActFrm;
use merc_syntax::ActFrmBinaryOp;
use merc_syntax::Action;
use merc_syntax::FixedPointOperator;
use merc_syntax::ModalityOperator;
use merc_syntax::MultiAction;
use merc_syntax::RegFrm;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmOp;
use merc_utilities::MercError;

use crate::FeatureTransitionSystem;
use crate::ModalEquationSystem;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VertexIndex;
use crate::compute_reachable;
use crate::make_vpg_total;

/// Translates a feature transition system into a variability parity game.
pub fn translate(
    manager_ref: &BDDManagerRef,
    fts: &FeatureTransitionSystem,
    configuration: BDDFunction,
    formula: &StateFrm,
) -> Result<VariabilityParityGame, MercError> {
    // Parses all labels into MultiAction once
    let parsed_labels: Result<Vec<MultiAction>, MercError> =
        fts.labels().iter().map(|label| MultiAction::parse(label)).collect();

    // Simplify the labels by stripping BDD information
    let simplified_labels: Vec<MultiAction> = parsed_labels?
        .iter()
        .map(strip_feature_configuration_from_multi_action)
        .collect();

    let equation_system = ModalEquationSystem::new(formula);
    debug!("{}", equation_system);
    let mut algorithm = Translation::new(
        fts,
        &simplified_labels,
        &equation_system,
        manager_ref.with_manager_shared(|manager| BDDFunction::t(manager)),
    );

    algorithm.translate(fts.initial_state_index(), 0)?;

    // Convert the feature diagram (with names) to a VPG
    let variables: Vec<BDDFunction> = fts.features().values().cloned().collect();

    let result = VariabilityParityGame::from_edges(
        manager_ref,
        VertexIndex::new(0),
        algorithm.vertices.iter().map(|(p, _)| p).cloned().collect(),
        algorithm.vertices.into_iter().map(|(_, pr)| pr).collect(),
        configuration,
        variables,
        || algorithm.edges.iter().cloned(),
    );

    // Check that all vertices are reachable from the initial vertex. After
    // totality it could be that the true or false nodes are not reachable.
    if cfg!(debug_assertions) {
        let (_, reachable_vertices) = compute_reachable(&result);
        debug_assert!(
            reachable_vertices.iter().all(|v| v.is_some()),
            "Not all vertices are reachable from the initial vertex"
        );
    }

    // Ensure that the result is a total VPG.
    let total_result = if !result.is_total(manager_ref)? {
        make_vpg_total(manager_ref, &result)?
    } else {
        result
    };

    Ok(total_result)
}

/// Is used to distinguish between StateFrm and Equation vertices in the vertex map.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Formula<'a> {
    StateFrm(&'a StateFrm),
    Equation(usize),
}

/// Local struct to keep track of the translation state
///
/// Implements the translation from (s, Ψ) pairs to VPG vertices and edges.
/// However, to avoid the complication of merging sub-results we immediately
/// store the vertices and edges into mutable vectors. Furthermore, to avoid
/// stack overflows we use a breadth-first search approach with a queue. This
/// means that during queuing we immediately assign a fresh index to each (s, Ψ)
/// pair (if it does not yet exist) and then queue it to assign its actual
/// values later on.
struct Translation<'a> {
    vertex_map: IndexedSet<(StateIndex, Formula<'a>)>,
    vertices: Vec<(Player, Priority)>,
    edges: Vec<(VertexIndex, BDDFunction, VertexIndex)>,

    // Used for the breadth first search.
    queue: Vec<(StateIndex, Formula<'a>, VertexIndex)>,

    /// The parsed labels of the FTS.
    parsed_labels: &'a Vec<MultiAction>,

    /// The feature transition system being translated.
    fts: &'a FeatureTransitionSystem,

    /// A reference to the modal equation system being translated.
    equation_system: &'a ModalEquationSystem,

    /// The BDD representing the "true" feature configuration.
    true_bdd: BDDFunction,

    /// Use to print progress information.
    progress: TimeProgress<usize>,
}

impl<'a> Translation<'a> {
    /// Creates a new translation instance.
    fn new(
        fts: &'a FeatureTransitionSystem,
        parsed_labels: &'a Vec<MultiAction>,
        equation_system: &'a ModalEquationSystem,
        true_bdd: BDDFunction,
    ) -> Self {
        let progress: TimeProgress<usize> = TimeProgress::new(
            |num_of_vertices: usize| {
                info!("Translated {} vertices...", num_of_vertices);
            },
            1,
        );

        Self {
            vertex_map: IndexedSet::new(),
            vertices: Vec::new(),
            edges: Vec::new(),
            queue: Vec::new(),
            fts,
            parsed_labels,
            equation_system,
            true_bdd,
            progress,
        }
    }

    /// Perform the actual translation.
    fn translate(&mut self, initial_state: StateIndex, initial_equation_index: usize) -> Result<(), MercError> {
        // We store (state, formula, N) into the queue, where N is the vertex number assigned to this pair. This means
        // that during the traversal we can assume this N to exist.
        self.queue = vec![(
            initial_state,
            Formula::Equation(initial_equation_index),
            VertexIndex::new(0),
        )];
        self.vertices.push((Player::Odd, Priority::new(0))); // Placeholder for the initial vertex

        while let Some((s, formula, vertex_index)) = self.queue.pop() {
            debug!("Translating vertex {}: (s={}, formula={:?})", vertex_index, s, formula);
            self.progress.print(self.vertices.len());
            match formula {
                Formula::StateFrm(f) => {
                    self.translate_vertex(s, f, vertex_index);
                }
                Formula::Equation(i) => {
                    self.translate_equation(s, i, vertex_index);
                }
            }
        }

        Ok(())
    }

    /// Translate a single vertex (s, Ψ) into the variability parity game vertex and its outgoing edges.
    ///
    /// The `fts` and `parsed_labels` are used to find the outgoing transitions matching the modalities in the formula.
    ///
    /// These are stored in the provided `vertices` and `edges` vectors.
    /// The `vertex_map` is used to keep track of already translated vertices.
    ///
    /// This function is recursively called for subformulas.
    pub fn translate_vertex(&mut self, s: StateIndex, formula: &'a StateFrm, vertex_index: VertexIndex) {
        match formula {
            StateFrm::True => {
                // (s, true) → odd, 0
                self.vertices[vertex_index] = (Player::Odd, Priority::new(0));
            }
            StateFrm::False => {
                // (s, false) → even, 0
                self.vertices[vertex_index] = (Player::Even, Priority::new(0));
            }
            StateFrm::Binary { op, lhs, rhs } => {
                match op {
                    StateFrmOp::Conjunction => {
                        // (s, Ψ_1 ∧ Ψ_2) →_P odd, (s, Ψ_1) and (s, Ψ_2), 0
                        self.vertices[vertex_index] = (Player::Odd, Priority::new(0));
                        let s_psi_1 = self.queue_vertex(s, Formula::StateFrm(lhs));
                        let s_psi_2 = self.queue_vertex(s, Formula::StateFrm(rhs));

                        self.edges.push((vertex_index, self.true_bdd.clone(), s_psi_1));
                        self.edges.push((vertex_index, self.true_bdd.clone(), s_psi_2));
                    }
                    StateFrmOp::Disjunction => {
                        // (s, Ψ_1 ∨ Ψ_2) →_P even, (s, Ψ_1) and (s, Ψ_2), 0
                        self.vertices[vertex_index] = (Player::Even, Priority::new(0));
                        let s_psi_1 = self.queue_vertex(s, Formula::StateFrm(lhs));
                        let s_psi_2 = self.queue_vertex(s, Formula::StateFrm(rhs));

                        self.edges.push((vertex_index, self.true_bdd.clone(), s_psi_1));
                        self.edges.push((vertex_index, self.true_bdd.clone(), s_psi_2));
                    }
                    _ => {
                        unimplemented!("Cannot translate binary operator in {}", formula);
                    }
                }
            }
            StateFrm::Id(identifier, _args) => {
                let (i, _equation) = self
                    .equation_system
                    .find_equation_by_identifier(identifier)
                    .expect("Variable must correspond to an equation");

                self.vertices[vertex_index] = (Player::Odd, Priority::new(0)); // The priority and owner do not matter here
                let equation_vertex = self.queue_vertex(s, Formula::Equation(i));
                self.edges.push((vertex_index, self.true_bdd.clone(), equation_vertex));
            }
            StateFrm::Modality {
                operator,
                formula,
                expr,
            } => {
                match operator {
                    ModalityOperator::Box => {
                        // (s, [a] Ψ) → odd, (s', Ψ) for all s' with s -a-> s', 0
                        self.vertices[vertex_index] = (Player::Odd, Priority::new(0));

                        for transition in self.fts.outgoing_transitions(s) {
                            let action = &self.parsed_labels[*transition.label];

                            trace!("Matching action {} against formula {}", action, formula);

                            if match_regular_formula(formula, action) {
                                let s_prime_psi = self.queue_vertex(transition.to, Formula::StateFrm(expr));

                                self.edges.push((
                                    vertex_index,
                                    self.fts.feature_label(transition.label).clone(),
                                    s_prime_psi,
                                ));
                            }
                        }
                    }
                    ModalityOperator::Diamond => {
                        // (s, <a> Ψ) → even, (s', Ψ) for all s' with s -a-> s', 0
                        self.vertices[vertex_index] = (Player::Even, Priority::new(0));

                        for transition in self.fts.outgoing_transitions(s) {
                            let action = &self.parsed_labels[*transition.label];

                            if match_regular_formula(formula, action) {
                                let s_prime_psi = self.queue_vertex(transition.to, Formula::StateFrm(expr));

                                self.edges.push((
                                    vertex_index,
                                    self.fts.feature_label(transition.label).clone(),
                                    s_prime_psi,
                                ));
                            }
                        }
                    }
                }
            }
            _ => {
                unimplemented!("Cannot translate formula {}", formula);
            }
        }
    }

    /// Applies the translation to the given (s, equation) vertex.
    fn translate_equation(&mut self, s: StateIndex, equation_index: usize, vertex_index: VertexIndex) {
        let equation = self.equation_system.equation(equation_index);
        match equation.operator() {
            FixedPointOperator::Least => {
                // (s, μ X. Ψ) →_P odd, (s, Ψ[x := μ X. Ψ]), 2 * floor(AD(Ψ)/2) + 1. In Rust division is already floor.
                self.vertices[vertex_index] = (
                    Player::Odd,
                    Priority::new(2 * (self.equation_system.alternation_depth(equation_index) / 2) + 1),
                );
                let s_psi = self.queue_vertex(s, Formula::StateFrm(equation.body()));
                self.edges.push((vertex_index, self.true_bdd.clone(), s_psi));
            }
            FixedPointOperator::Greatest => {
                // (s, ν X. Ψ) →_P even, (s, Ψ[x := ν X. Ψ]), 2 * (AD(Ψ)/2). In Rust division is already floor.
                self.vertices[vertex_index] = (
                    Player::Even,
                    Priority::new(2 * (self.equation_system.alternation_depth(equation_index) / 2)),
                );
                let s_psi = self.queue_vertex(s, Formula::StateFrm(equation.body()));
                self.edges.push((vertex_index, self.true_bdd.clone(), s_psi));
            }
        }
    }

    /// Queues a new pair to be translated, returning its vertex index.
    fn queue_vertex(&mut self, s: StateIndex, formula: Formula<'a>) -> VertexIndex {
        let (index, inserted) = self.vertex_map.insert((s, formula.clone()));
        let vertex_index = VertexIndex::new(*index);

        if inserted {
            // New vertex, assign placeholder values
            self.vertices.resize(*vertex_index + 1, (Player::Odd, Priority::new(0)));
            self.queue.push((s, formula, vertex_index));
        }

        vertex_index
    }
}

/// Removes the BDD information from the multi-action, i.e., only keeps the action labels.
fn strip_feature_configuration_from_multi_action(multi_action: &MultiAction) -> MultiAction {
    MultiAction {
        actions: multi_action
            .actions
            .iter()
            .map(|action| Action {
                id: action.id.clone(),
                args: Vec::new(),
            })
            .collect(),
    }
}

/// Returns true iff the given action matches the regular formula.
fn match_regular_formula(formula: &RegFrm, action: &MultiAction) -> bool {
    match formula {
        RegFrm::Action(action_formula) => match_action_formula(action_formula, action),
        RegFrm::Choice { lhs, rhs } => match_regular_formula(lhs, action) || match_regular_formula(rhs, action),
        _ => {
            unimplemented!("Cannot translate regular formula {}", formula);
        }
    }
}

/// Returns true iff the given action matches the action formula.
fn match_action_formula(formula: &ActFrm, action: &MultiAction) -> bool {
    match formula {
        ActFrm::True => true,
        ActFrm::False => false,
        ActFrm::MultAct(expected_action) => expected_action == action,
        ActFrm::Binary { op, lhs, rhs } => match op {
            ActFrmBinaryOp::Union => match_action_formula(lhs, action) || match_action_formula(rhs, action),
            ActFrmBinaryOp::Intersect => match_action_formula(lhs, action) && match_action_formula(rhs, action),
            _ => {
                unimplemented!("Cannot translate binary operator {}", formula);
            }
        },
        ActFrm::Negation(expr) => !match_action_formula(expr, action),
        _ => {
            unimplemented!("Cannot translate action formula {}", formula);
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_macros::merc_test;
    use merc_syntax::UntypedStateFrmSpec;

    use crate::FeatureDiagram;
    use crate::read_fts;

    use super::*;

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_running_example() {
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

        let fd = FeatureDiagram::from_reader(
            &manager_ref,
            include_bytes!("../../../examples/vpg/running_example_fts.fd") as &[u8],
        )
        .unwrap();
        let fts = read_fts(
            &manager_ref,
            include_bytes!("../../../examples/vpg/running_example_fts.aut") as &[u8],
            fd.features().clone(),
        )
        .unwrap();

        let formula = UntypedStateFrmSpec::parse(include_str!("../../../examples/vpg/running_example.mcf")).unwrap();

        let _vpg = translate(&manager_ref, &fts, fd.configuration().clone(), &formula.formula).unwrap();
    }
}
