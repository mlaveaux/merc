use merc_lts::LTS;
use merc_lts::StateIndex;
use merc_syntax::FixedPointOperator;
use merc_syntax::ModalityOperator;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmOp;
use merc_utilities::IndexedSet;
use merc_utilities::MercError;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

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

    // Translate the initial vertex
    translate_vertex(
        &mut vertex_map,
        &mut vertices,
        &mut edges,
        fts.initial_state_index(),
        fts,
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
                    // (s, Ψ_1 ∧ Ψ_2) → odd, (s, Ψ_1)|P and (s, Ψ_2)|P, 0
                    vertices.push((Player::Even, Priority::new(0)));
                    let s_psi_1 = translate_vertex(vertex_map, vertices, edges, s, fts, lhs)?;
                    let s_psi_2 = translate_vertex(vertex_map, vertices, edges, s, fts, rhs)?;

                    edges.push((vertex_index, fts.configuration().clone(), s_psi_1));
                    edges.push((vertex_index, fts.configuration().clone(), s_psi_2));
                }
                StateFrmOp::Disjunction => {
                    // (s, Ψ_1 ∨ Ψ_2) → even, (s, Ψ_1)|P and (s, Ψ_2)|P, 0
                    vertices.push((Player::Odd, Priority::new(0)));
                    let s_psi_1 = translate_vertex(vertex_map, vertices, edges, s, fts, lhs)?;
                    let s_psi_2 = translate_vertex(vertex_map, vertices, edges, s, fts, rhs)?;

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
                    // (s, μ X. Ψ) → odd, (s, Ψ), 1
                    vertices.push((Player::Odd, Priority::new(1)));

                    // (s, ν X. Ψ) → even, (s, Ψ), 2
                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
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
                    vertices.push((Player::Even, Priority::new(2)));

                    // (s, ν X. Ψ) → even, (s, Ψ), 2
                    let s_psi = translate_vertex(
                        vertex_map,
                        vertices,
                        edges,
                        s,
                        fts,
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
                        let s_prime_psi = translate_vertex(vertex_map, vertices, edges, transition.to, fts, expr)?;

                        edges.push((vertex_index, fts.configuration().clone(), s_prime_psi)); // TODO: Set proper configuration
                    }
                }
                ModalityOperator::Diamond => {
                    // (s, <a> Ψ) → even, (s', Ψ) for all s' with s -a-> s', 0
                    vertices.push((Player::Even, Priority::new(0)));

                    for transition in fts.outgoing_transitions(s) {
                        let s_prime_psi = translate_vertex(vertex_map, vertices, edges, transition.to, fts, expr)?;

                        edges.push((vertex_index, fts.configuration().clone(), s_prime_psi));
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



/// Substitute state formula variables in a formula using the provided substitution function.
fn substitute(formula: StateFrm, substitution: &impl Fn(&StateFrm) -> Option<StateFrm>) -> StateFrm {
    if let Some(formula) = substitution(&formula) {
        // A substitution was made, return the new formula.
        return formula;
    }

    match formula {
        StateFrm::Binary { op, lhs, rhs } => {
            let new_lhs = substitute(*lhs, substitution);
            let new_rhs = substitute(*rhs, substitution);
            StateFrm::Binary {
                op,
                lhs: Box::new(new_lhs),
                rhs: Box::new(new_rhs),
            }
        }
        StateFrm::FixedPoint {
            operator,
            variable,
            body,
        } => {
            let new_body = substitute(*body, substitution);
            StateFrm::FixedPoint {
                operator,
                variable,
                body: Box::new(new_body),
            }
        }
        StateFrm::Bound { bound, variables, body } => {
            let new_body = substitute(*body, substitution);
            StateFrm::Bound {
                bound,
                variables,
                body: Box::new(new_body),
            }
        }
        StateFrm::Modality {
            operator,
            formula,
            expr,
        } => {
            let expr = substitute(*expr, substitution);
            StateFrm::Modality {
                operator,
                formula,
                expr: Box::new(expr),
            }
        }
        StateFrm::Quantifier {
            quantifier,
            variables,
            body,
        } => {
            let new_body = substitute(*body, substitution);
            StateFrm::Quantifier {
                quantifier,
                variables,
                body: Box::new(new_body),
            }
        }
        StateFrm::DataValExprRightMult(expr, data_val) => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::DataValExprRightMult(Box::new(new_expr), data_val)
        }
        StateFrm::DataValExprLeftMult(data_val, expr) => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::DataValExprLeftMult(data_val, Box::new(new_expr))
        }
        StateFrm::Unary { op, expr } => {
            let new_expr = substitute(*expr, substitution);
            StateFrm::Unary {
                op,
                expr: Box::new(new_expr),
            }
        }
        StateFrm::Id(_, _)
        | StateFrm::True
        | StateFrm::False
        | StateFrm::Delay(_)
        | StateFrm::Yaled(_)
        | StateFrm::DataValExpr(_) => formula,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_substitution_state_formulas() {}
}
