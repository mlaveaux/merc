use log::info;
use log::trace;
use merc_syntax::ActFrm;
use merc_syntax::ActFrmBinaryOp;
use merc_syntax::Action;
use merc_syntax::StateVarDecl;
use merc_syntax::visit;
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

    // Translate the initial vertex
    translate_vertex(
        &mut vertex_map,
        &mut vertices,
        &mut edges,
        fts.initial_state_index(),
        fts,
        &simplified_labels,
        formula,
    )?;

    // Convert the feature diagram (with names) to a VPG
    let variables: Vec<BDDFunction> = fts.variables().iter().map(|(_, var)| var.clone()).collect();

    let num_of_vertices = vertices.len();
    Ok(VariabilityParityGame::from_edges(
        manager_ref,
        VertexIndex::new(0),
        vertices.iter().map(|(p, _)| p).cloned().collect(),
        vertices.into_iter().map(|(_, pr)| pr).collect(),
        Some(num_of_vertices),
        fts.configuration().clone(),
        variables,
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
                _ => {
                    unimplemented!("Cannot translate binary operator in {}", formula);
                }
            }
        }
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            match operator {
                FixedPointOperator::Least => {
                    // (s, μ X. Ψ) →_P odd, (s, Ψ[x := μ X. Ψ]), 2 * floor(AD(Ψ)/2) + 1
                    vertices.push((Player::Odd, Priority::new(2 * alternation_depth(formula) / 2 + 1)));

                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
                        parsed_labels,
                        &visit(*body.clone(), |subformula| {
                            match &subformula {
                                StateFrm::Id(name, arguments) => {
                                    assert!(
                                        arguments.is_empty(),
                                        "State formula variables with arguments are not supported in VPG translation"
                                    );
                                    if variable.identifier == *name {
                                        return Ok(Some(formula.clone()));
                                    }
                                }
                                StateFrm::FixedPoint {
                                    operator: _,
                                    variable: inner_variable,
                                    body: _,
                                } => {
                                    // Prevent capturing inner fixed-point variables with the same name
                                    if variable.identifier == *inner_variable.identifier {
                                        return Ok(Some(subformula.clone()));
                                    }
                                }
                                _ => {
                                    // Do nothing
                                }
                            }

                            Ok(None)
                        })?,
                    )?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi));
                }
                FixedPointOperator::Greatest => {
                    // (s, ν X. Ψ) →_P even, (s, Ψ[X := ν X. Ψ]), 2 * floor(AD(Ψ)/2)
                    vertices.push((Player::Even, Priority::new(2 * alternation_depth(formula) / 2)));

                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
                        parsed_labels,
                        &visit(*body.clone(), |subformula| {
                            match &subformula {
                                StateFrm::Id(name, arguments) => {
                                    assert!(
                                        arguments.is_empty(),
                                        "State formula variables with arguments are not supported in VPG translation"
                                    );
                                    if variable.identifier == *name {
                                        return Ok(Some(formula.clone()));
                                    }
                                }
                                StateFrm::FixedPoint {
                                    operator: _,
                                    variable: inner_variable,
                                    body: _,
                                } => {
                                    // Prevent capturing inner fixed-point variables with the same name
                                    if variable.identifier == *inner_variable.identifier {
                                        return Ok(Some(subformula.clone()));
                                    }
                                }
                                _ => {
                                    // Do nothing
                                }
                            }

                            Ok(None)
                        })?,
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

                        if match_regular_formula(formula, &action) {
                            let s_prime_psi =
                                translate_vertex(vertex_map, vertices, edges, transition.to, fts, parsed_labels, expr)?;

                            edges.push((
                                vertex_index,
                                fts.configuration().and(fts.feature_label(transition.label))?,
                                s_prime_psi,
                            ));
                        }
                    }
                }
                ModalityOperator::Diamond => {
                    // (s, <a> Ψ) → even, (s', Ψ) for all s' with s -a-> s', 0
                    vertices.push((Player::Even, Priority::new(0)));

                    for transition in fts.outgoing_transitions(s) {
                        let action = &parsed_labels[*transition.label];

                        if match_regular_formula(formula, &action) {
                            let s_prime_psi =
                                translate_vertex(vertex_map, vertices, edges, transition.to, fts, parsed_labels, expr)?;

                            edges.push((
                                vertex_index,
                                fts.configuration().and(fts.feature_label(transition.label))?,
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

    debug_assert!(
        vertex_index <= vertices.len() - 1,
        "New vertex must have been added for {formula}"
    );
    Ok(vertex_index)
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

/// Returns the alternation depth of the given state formula.
fn alternation_depth(formula: &StateFrm) -> usize {
    match formula {
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
            ..
        } => alternation_depth_rec(body, *operator, &variable),
        _ => {
            unimplemented!("Cannot determine alternation depth of formula {}", formula)
        }
    }
}

/// Returns the alternation depth of the given state formula.
fn alternation_depth_rec(formula: &StateFrm, op: FixedPointOperator, name: &StateVarDecl) -> usize {
    match formula {
        StateFrm::Id(id, _) => {
            if id == &name.identifier {
                1
            } else {
                0
            }
        }
        StateFrm::FixedPoint { operator, body, .. } => {
            let depth = alternation_depth_rec(body, *operator, name);
            if depth > 0 {
                (if *operator != op { 1 } else { 0 }) + depth
            } else {
                0
            }
        }
        StateFrm::Binary { lhs, rhs, .. } => {
            alternation_depth_rec(lhs, op, name).max(alternation_depth_rec(rhs, op, name))
        }
        StateFrm::Modality { expr, .. } => alternation_depth_rec(expr, op, name),
        _ => {
            unimplemented!("Cannot determine alternation depth of formula {}", formula)
        }
    }
}

#[cfg(test)]
mod tests {

    use merc_syntax::UntypedStateFrmSpec;

    use crate::FeatureDiagram;
    use crate::read_fts;

    use super::*;

    #[test]
    fn test_alternation_depth() {
        assert_eq!(
            alternation_depth(&UntypedStateFrmSpec::parse("nu X. X").unwrap().formula),
            1
        );
        assert_eq!(
            alternation_depth(&UntypedStateFrmSpec::parse("nu X. nu Z. X").unwrap().formula),
            1
        );
        assert_eq!(
            alternation_depth(&UntypedStateFrmSpec::parse("nu X. mu Z. X && Z").unwrap().formula),
            2
        );
    }

    #[test]
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
