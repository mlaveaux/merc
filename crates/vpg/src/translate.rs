use log::info;
use log::trace;
use merc_syntax::ActFrm;
use merc_syntax::ActFrmBinaryOp;
use merc_syntax::Action;
use oxidd::BooleanFunction;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_syntax::FixedPointOperator;
use merc_syntax::ModalityOperator;
use merc_syntax::MultiAction;
use merc_syntax::RegFrm;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmOp;
use merc_utilities::IndexedSet;
use merc_utilities::MercError;

use crate::FeatureTransitionSystem;
use crate::ModalEquationSystem;
use crate::PG;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VertexIndex;

/// Translates a feature transition system into a variability parity game.
pub fn translate(
    manager_ref: &BDDManagerRef,
    fts: &FeatureTransitionSystem,
    formula: &StateFrm,
) -> Result<VariabilityParityGame, MercError> {
    // Parses all labels into MultiAction once
    let parsed_labels: Result<Vec<MultiAction>, MercError> =
        fts.labels().iter().map(|label| MultiAction::parse(label)).collect();

    // Simplify the labels by stripping BDD information
    let simplified_labels: Vec<MultiAction> = parsed_labels?
        .iter()
        .map(|ma| strip_feature_configuration_from_multi_action(ma))
        .collect();

    for label in &simplified_labels {
        info!("label: {}", label);
    }

    let equation_system = ModalEquationSystem::new(formula);
    info!("{}", equation_system);
    let mut algorithm = Translation::new(fts, &simplified_labels, &equation_system, true);

    algorithm.translate_equation(fts.initial_state_index(), 0)?;

    // Convert the feature diagram (with names) to a VPG
    let variables: Vec<BDDFunction> = fts.variables().iter().map(|(_, var)| var.clone()).collect();

    let result = VariabilityParityGame::from_edges(
        manager_ref,
        VertexIndex::new(0),
        algorithm.vertices.iter().map(|(p, _)| p).cloned().collect(),
        algorithm.vertices.into_iter().map(|(_, pr)| pr).collect(),
        fts.configuration().clone(),
        variables,
        || algorithm.edges.iter().cloned(),
    );

    // Check that the result is a total VPG.
    if cfg!(debug_assertions) {
        for v in result.iter_vertices() {
            debug_assert!(
                result.outgoing_edges(v).next().is_some(),
                "VPG is not total: vertex {} has no outgoing edges",
                v
            );
        }
    }

    Ok(result)
}

/// Is used to distinguish between StateFrm and Equation vertices in the vertex map.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Formula<'a> {
    StateFrm(&'a StateFrm),
    Equation(usize),
}

// Local struct to keep track of the translation state
struct Translation<'a> {
    vertex_map: IndexedSet<(StateIndex, Formula<'a>)>,
    vertices: Vec<(Player, Priority)>,
    edges: Vec<(VertexIndex, BDDFunction, VertexIndex)>,

    /// Set to true to ensure that the resulting VPG edge relation is total
    make_total: bool,

    /// The parsed labels of the FTS.
    parsed_labels: &'a Vec<MultiAction>,

    /// The feature transition system being translated.
    fts: &'a FeatureTransitionSystem,

    /// A reference to the modal equation system being translated.
    equation_system: &'a ModalEquationSystem,
}

impl<'a> Translation<'a> {
    fn new(
        fts: &'a FeatureTransitionSystem,
        parsed_labels: &'a Vec<MultiAction>,
        equation_system: &'a ModalEquationSystem,
        make_total: bool,
    ) -> Self {
        Self {
            vertex_map: IndexedSet::new(),
            vertices: Vec::new(),
            edges: Vec::new(),
            fts,
            parsed_labels,
            equation_system,
            make_total,
        }
    }

    /// Translate a single vertex (s, Ψ) into the variability parity game vertex and its outgoing edges.
    ///
    /// The `fts` and `parsed_labels` are used to find the outgoing transitions matching the modalities in the formula.
    ///
    /// These are stored in the provided `vertices` and `edges` vectors.
    /// The `vertex_map` is used to keep track of already translated vertices.
    ///
    /// This function is recursively called for subformulas.
    pub fn translate_vertex(&mut self, s: StateIndex, formula: &'a StateFrm) -> Result<VertexIndex, MercError> {
        let (index, inserted) = self.vertex_map.insert((s, Formula::StateFrm(formula)));
        let vertex_index = VertexIndex::new(*index);

        if !inserted {
            // Returns the existing vertex.
            return Ok(vertex_index);
        }

        // New vertex should be created, and this mapping is dense
        debug_assert_eq!(
            vertex_index,
            self.vertices.len(),
            "Vertex indices should be assigned sequentially"
        );

        match formula {
            StateFrm::True => {
                // (s, true) → odd, 0
                self.vertices.push((Player::Odd, Priority::new(0)));

                if self.make_total {
                    // Self-loop
                    self.edges
                        .push((vertex_index, self.fts.configuration().clone(), vertex_index));
                }
            }
            StateFrm::False => {
                // (s, false) → even, 0
                self.vertices.push((Player::Even, Priority::new(0)));

                if self.make_total {
                    // Self-loop
                    self.edges
                        .push((vertex_index, self.fts.configuration().clone(), vertex_index));
                }
            }
            StateFrm::Binary { op, lhs, rhs } => {
                match op {
                    StateFrmOp::Conjunction => {
                        // (s, Ψ_1 ∧ Ψ_2) →_P odd, (s, Ψ_1) and (s, Ψ_2), 0
                        self.vertices.push((Player::Odd, Priority::new(0)));
                        let s_psi_1 = self.translate_vertex(s, lhs)?;
                        let s_psi_2 = self.translate_vertex(s, rhs)?;

                        self.edges
                            .push((vertex_index, self.fts.configuration().clone(), s_psi_1));
                        self.edges
                            .push((vertex_index, self.fts.configuration().clone(), s_psi_2));
                    }
                    StateFrmOp::Disjunction => {
                        // (s, Ψ_1 ∨ Ψ_2) →_P even, (s, Ψ_1) and (s, Ψ_2), 0
                        self.vertices.push((Player::Even, Priority::new(0)));
                        let s_psi_1 = self.translate_vertex(s, lhs)?;
                        let s_psi_2 = self.translate_vertex(s, rhs)?;

                        self.edges
                            .push((vertex_index, self.fts.configuration().clone(), s_psi_1));
                        self.edges
                            .push((vertex_index, self.fts.configuration().clone(), s_psi_2));
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

                self.vertices.push((Player::Odd, Priority::new(0))); // The priority and owner do not matter here
                let equation_vertex = self.translate_equation(s, i);
                self.edges
                    .push((vertex_index, self.fts.configuration().clone(), equation_vertex?));
            }
            StateFrm::Modality {
                operator,
                formula,
                expr,
            } => {
                match operator {
                    ModalityOperator::Box => {
                        // (s, [a] Ψ) → odd, (s', Ψ) for all s' with s -a-> s', 0
                        self.vertices.push((Player::Odd, Priority::new(0)));

                        let mut matched = false;
                        for transition in self.fts.outgoing_transitions(s) {
                            let action = &self.parsed_labels[*transition.label];

                            trace!("Matching action {} against formula {}", action, formula);

                            if match_regular_formula(formula, &action) {
                                matched = true;
                                let s_prime_psi = self.translate_vertex(transition.to, expr)?;

                                self.edges.push((
                                    vertex_index,
                                    self.fts.configuration().and(self.fts.feature_label(transition.label))?,
                                    s_prime_psi,
                                ));
                            }
                        }

                        if !matched && self.make_total {
                            // No matching transitions, add a self-loop to ensure totality
                            self.edges
                                .push((vertex_index, self.fts.configuration().clone(), vertex_index));
                        }
                    }
                    ModalityOperator::Diamond => {
                        // (s, <a> Ψ) → even, (s', Ψ) for all s' with s -a-> s', 0
                        self.vertices.push((Player::Even, Priority::new(0)));

                        let mut matched = false;
                        for transition in self.fts.outgoing_transitions(s) {
                            let action = &self.parsed_labels[*transition.label];

                            if match_regular_formula(formula, &action) {
                                matched = true;
                                let s_prime_psi = self.translate_vertex(transition.to, expr)?;

                                self.edges.push((
                                    vertex_index,
                                    self.fts.configuration().and(self.fts.feature_label(transition.label))?,
                                    s_prime_psi,
                                ));
                            }
                        }

                        if !matched && self.make_total {
                            // No matching transitions, add a self-loop to ensure totality
                            self.edges
                                .push((vertex_index, self.fts.configuration().clone(), vertex_index));
                        }
                    }
                }
            }
            _ => {
                unimplemented!("Cannot translate formula {}", formula);
            }
        }

        debug_assert!(
            vertex_index <= self.vertices.len() - 1,
            "New vertex must have been added for {formula}"
        );
        Ok(vertex_index)
    }

    /// Applies the translation to the given (s, equation) vertex.
    fn translate_equation(&mut self, s: StateIndex, equation_index: usize) -> Result<VertexIndex, MercError> {
        let (index, inserted) = self.vertex_map.insert((s, Formula::Equation(equation_index)));
        let vertex_index = VertexIndex::new(*index);

        if !inserted {
            // Returns the existing vertex.
            return Ok(vertex_index);
        }

        let equation = self.equation_system.equation(equation_index);
        match equation.operator() {
            FixedPointOperator::Least => {
                // (s, μ X. Ψ) →_P odd, (s, Ψ[x := μ X. Ψ]), 2 * floor(AD(Ψ)/2) + 1. In Rust division is already floor.
                self.vertices.push((
                    Player::Odd,
                    Priority::new(2 * (self.equation_system.alternation_depth(equation_index) / 2) + 1),
                ));
                let s_psi = self.translate_vertex(s, equation.body())?;
                self.edges.push((vertex_index, self.fts.configuration().clone(), s_psi));
            }
            FixedPointOperator::Greatest => {
                // (s, ν X. Ψ) →_P even, (s, Ψ[x := ν X. Ψ]), 2 * (AD(Ψ)/2). In Rust division is already floor.
                self.vertices.push((
                    Player::Even,
                    Priority::new(2 * (self.equation_system.alternation_depth(equation_index) / 2)),
                ));
                let s_psi = self.translate_vertex(s, equation.body())?;
                self.edges.push((vertex_index, self.fts.configuration().clone(), s_psi));
            }
        }

        debug_assert!(
            vertex_index <= self.vertices.len() - 1,
            "New vertex must have been added for equation {equation_index}"
        );
        Ok(vertex_index)
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
            include_bytes!("../../../examples/vpg/running_example.fd") as &[u8],
        )
        .unwrap();
        let fts = read_fts(
            &manager_ref,
            include_bytes!("../../../examples/vpg/running_example_fts.aut") as &[u8],
            fd,
        )
        .unwrap();

        let formula = UntypedStateFrmSpec::parse(include_str!("../../../examples/vpg/running_example.mcf")).unwrap();

        let _vpg = translate(&manager_ref, &fts, &formula.formula).unwrap();
    }
}
