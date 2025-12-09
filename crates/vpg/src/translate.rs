
use log::trace;
use merc_syntax::ActFrm;
use merc_syntax::ActFrmBinaryOp;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::BooleanFunction;

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
    let mut vertex_map: IndexedSet<(StateIndex, StateFrm)> = IndexedSet::new();
    let mut vertices: Vec<(Player, Priority)> = Vec::new();
    let mut edges: Vec<(VertexIndex, BDDFunction, VertexIndex)> = Vec::new();

    // Parses all labels into MultiAction once
    let parsed_labels: Result<Vec<MultiAction>, MercError> = fts.labels().iter().map(|label| MultiAction::parse(label)).collect();

    // Translate the initial vertex
    translate_vertex(
        &mut vertex_map,
        &mut vertices,
        &mut edges,
        fts.initial_state_index(),
        fts,
        &parsed_labels?,
        formula,
    )?;

    let num_of_vertices = vertices.len();
    Ok(VariabilityParityGame::from_edges(
        manager_ref,
        VertexIndex::new(0),
        vertices.iter().map(|(p, _)| p).cloned().collect(),
        vertices.into_iter().map(|(_, pr)| pr).collect(),
        Some(num_of_vertices),
        fts.configuration().clone(),
        || edges.iter().cloned(),
    ))
}

/// Translate a single vertex (s, Ψ) into the variability parity game vertex and its outgoing edges.
///
/// These are stored in the provided `vertices` and `edges` vectors.
pub fn translate_vertex<'a>(
    vertex_map: &mut IndexedSet<(StateIndex, StateFrm)>,
    vertices: &mut Vec<(Player, Priority)>,
    edges: &mut Vec<(VertexIndex, BDDFunction, VertexIndex)>,
    s: StateIndex,
    fts: &FeatureTransitionSystem,
    parsed_labels: &Vec<MultiAction>,
    formula: &StateFrm,
) -> Result<VertexIndex, MercError> {
    let (index, inserted) = vertex_map.insert((s, formula.clone()));
    let vertex_index = VertexIndex::new(*index);

    if !inserted {
        // Returns the existing vertex.
        return Ok(vertex_index);
    }

    // New vertex should be created, and this mapping is dense
    debug_assert_eq!(
        vertex_index,
        vertices.len(),
        "Vertex indices should be assigned sequentially"
    );

    trace!("Translating vertex ({}, {})", s, formula);

    match formula {
        StateFrm::True => {
            // (s, true) → odd, 0
            vertices.push((Player::Odd, Priority::new(0)));
        }
        StateFrm::False => {
            // (s, false) → even, 0
            vertices.push((Player::Even, Priority::new(0)));
        }
        StateFrm::Binary { op, lhs, rhs } => {
            match op {
                StateFrmOp::Conjunction => {
                    // (s, Ψ_1 ∧ Ψ_2) →_P odd, (s, Ψ_1) and (s, Ψ_2), 0
                    vertices.push((Player::Odd, Priority::new(0)));
                    let s_psi_1 = translate_vertex(vertex_map, vertices, edges, s, fts, parsed_labels, lhs)?;
                    let s_psi_2 = translate_vertex(vertex_map, vertices, edges, s, fts, parsed_labels, rhs)?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi_1));
                    edges.push((vertex_index, fts.configuration().clone(), s_psi_2));
                }
                StateFrmOp::Disjunction => {
                    // (s, Ψ_1 ∨ Ψ_2) →_P even, (s, Ψ_1) and (s, Ψ_2), 0
                    vertices.push((Player::Even, Priority::new(0)));
                    let s_psi_1 = translate_vertex(vertex_map, vertices, edges, s, fts, parsed_labels, lhs)?;
                    let s_psi_2 = translate_vertex(vertex_map, vertices, edges, s, fts, parsed_labels, rhs)?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi_1));
                    edges.push((vertex_index, fts.configuration().clone(), s_psi_2));
                }
                _ => return Err(format!("Cannot translate binary operator in {}", formula).into()),
            }
        }
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            match operator {
                FixedPointOperator::Least => {
                    // (s, μ X. Ψ) →_P odd, (s, Ψ[x := μ X. Ψ]), 1
                    vertices.push((Player::Odd, Priority::new(1)));

                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
                        parsed_labels,
                        &substitute(*body.clone(), &|subformula| {
                            match &subformula {
                                StateFrm::Id(name, arguments) => {
                                    assert!(
                                        arguments.is_empty(),
                                        "State formula variables with arguments are not supported in VPG translation"
                                    );
                                    if variable.identifier == *name {
                                        return Some(formula.clone());
                                    }
                                }
                                _ => {
                                    // Do nothing
                                }
                            }

                            None
                        }),
                    )?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi));
                }
                FixedPointOperator::Greatest => {
                    // (s, ν X. Ψ) →_P even, (s, Ψ[X := ν X. Ψ]), 2
                    vertices.push((Player::Even, Priority::new(2)));

                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
                        parsed_labels,
                        &substitute(*body.clone(), &|subformula| {
                            match &subformula {
                                StateFrm::Id(name, arguments) => {
                                    assert!(
                                        arguments.is_empty(),
                                        "State formula variables with arguments are not supported in VPG translation"
                                    );
                                    if variable.identifier == *name {
                                        return Some(formula.clone());
                                    }
                                }
                                _ => {
                                    // Do nothing
                                }
                            }

                            None
                        }),
                    )?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi));
                }
            }
        }
        StateFrm::Modality {
            operator,
            formula,
            expr,
        } => {
            match operator {
                ModalityOperator::Box => {
                    // (s, [a] Ψ) → odd, (s', Ψ) for all s' with s -a-> s', 0
                    vertices.push((Player::Odd, Priority::new(0)));

                    for transition in fts.outgoing_transitions(s) {
                        let action = &parsed_labels[*transition.label];

                        trace!("Matching action {} against formula {}", action, formula);

                        if match_regular_formula(formula, &action)? {
                            let s_prime_psi = translate_vertex(vertex_map, vertices, edges, transition.to, fts, parsed_labels, expr)?;

                            edges.push((vertex_index, fts.configuration().and(fts.feature_label(transition.label))?, s_prime_psi));
                        }
                    }
                }
                ModalityOperator::Diamond => {
                    // (s, <a> Ψ) → even, (s', Ψ) for all s' with s -a-> s', 0
                    vertices.push((Player::Even, Priority::new(0)));

                    for transition in fts.outgoing_transitions(s) {
                        let action = &parsed_labels[*transition.label];

                        if match_regular_formula(formula, &action)? {
                            let s_prime_psi = translate_vertex(vertex_map, vertices, edges, transition.to, fts, parsed_labels, expr)?;

                            edges.push((vertex_index, fts.configuration().and(fts.feature_label(transition.label))?, s_prime_psi));
                        }
                    }
                }
            }
        }
        _ => return Err(format!("Cannot translate formula {}", formula).into()),
    }

    debug_assert!(
        vertex_index <= vertices.len() - 1,
        "New vertex must have been added for {formula}"
    );
    Ok(vertex_index)
}

/// Returns true iff the given action matches the regular formula.
fn match_regular_formula(formula: &RegFrm, action: &MultiAction) -> Result<bool, MercError> {
    match formula {
        RegFrm::Action(action_formula) => {
            match_action_formula(action_formula, action)            
        },
        RegFrm::Choice { lhs, rhs } => {
            Ok(match_regular_formula(lhs, action)? || match_regular_formula(rhs, action)?)
        }
        _ => Err(format!("Cannot translate regular formula {}", formula).into()),
    }
}

/// Returns true iff the given action matches the action formula.
fn match_action_formula(formula: &ActFrm, action: &MultiAction) -> Result<bool, MercError> {
    match formula {
        ActFrm::True => Ok(true),
        ActFrm::False => Ok(false),
        ActFrm::MultAct(expected_action) => {
            Ok(expected_action == action)
        },
        ActFrm::Binary { op, lhs, rhs } => {
            match op {
                ActFrmBinaryOp::Union => {
                    Ok(match_action_formula(lhs, action)? || match_action_formula(rhs, action)?)
                }
                _ => Err(format!("Cannot translate binary opertor {}", formula).into()),
            }
        }
        ActFrm::Negation(expr) => {
            Ok(!match_action_formula(expr, action)?)
        }
        _ => Err(format!("Cannot translate action formula {}", formula).into()),
    }
}