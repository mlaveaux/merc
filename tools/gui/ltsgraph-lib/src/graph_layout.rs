use std::sync::Arc;

use glam::Vec3;
use log::debug;
use merc_lts::LTS;
use rand::Rng;

use merc_lts::LabelledTransitionSystem;
use merc_unsafety::Edge;
use merc_unsafety::index_edge;

pub struct GraphLayout {
    // Store the underlying LTS to get the edges.
    pub lts: Arc<LabelledTransitionSystem<String>>,

    // For every state store layout information.
    pub layout_states: Vec<StateLayout>,
}

#[derive(Clone, Default)]
pub struct StateLayout {
    pub position: Vec3,
    pub force: Vec3,
}

impl GraphLayout {
    /// Construct a new layout for the given LTS.
    pub fn new(lts: Arc<LabelledTransitionSystem<String>>) -> GraphLayout {
        // Keep track of state layout information.
        let mut states_simulation = vec![StateLayout::default(); lts.num_of_states()];

        // Place the states at a random position within some bound based on the number of states.
        let mut rng = rand::rng();
        let bound = (lts.num_of_states() as f32).sqrt().ceil();

        debug!("Placing states within bound {bound}");
        for layout_state in &mut states_simulation {
            layout_state.position.x = rng.random_range(-bound..bound);
            layout_state.position.y = rng.random_range(-bound..bound);
        }

        GraphLayout {
            lts,
            layout_states: states_simulation,
        }
    }

    /// Update the layout one step using spring forces for transitions and repulsion between states.
    ///
    /// Returns true iff the layout is stable.
    pub fn update(&mut self, handle_length: f32, repulsion_strength: f32, delta: f32) -> bool {
        for state_index in self.lts.iter_states() {
            // Ignore the last state since it cannot repulse with any other state.
            if state_index < self.layout_states.len() {
                // Use split_at_mut to get two mutable slices at every split point.
                let (left_layout, right_layout) = self.layout_states.split_at_mut(state_index.value() + 1);
                let state_layout = left_layout.last_mut().unwrap();

                // Accumulate repulsion forces between vertices.
                for other_state_layout in right_layout {
                    let force = compute_repulsion_force(
                        &state_layout.position,
                        &other_state_layout.position,
                        repulsion_strength,
                    );

                    state_layout.force += force;
                    other_state_layout.force -= force;
                }
            }

            // Accumulate forces over all connected edges.
            for transition in self.lts.outgoing_transitions(state_index) {
                // Index an edge in the graph.
                match index_edge(&mut self.layout_states, state_index.value(), transition.to.value()) {
                    Edge::Selfloop(_) => {
                        // Handle self loop, but we apply no forces in this case.
                    }
                    Edge::Regular(from_layout, to_layout) => {
                        let force = compute_spring_force(&from_layout.position, &to_layout.position, handle_length);

                        from_layout.force += force;
                        to_layout.force -= force;
                    }
                }
            }
        }

        // Keep track of the total displacement of the system, to determine stablity
        let mut displacement = 0.0;

        for state_layout in &mut self.layout_states {
            // Integrate the forces.
            state_layout.position += state_layout.force * delta;
            displacement += (state_layout.force * delta).length_squared();

            // Reset the force.
            state_layout.force = Vec3::default();

            // A safety check for when the layout exploded.
            assert!(
                state_layout.position.is_finite(),
                "Invalid position {} obtained",
                state_layout.position
            );
        }

        (displacement / self.layout_states.len() as f32) < 0.01
    }
}

/// Compute a sping force between two points with a desired rest length.
fn compute_spring_force(p1: &Vec3, p2: &Vec3, rest_length: f32) -> Vec3 {
    let dist = p1.distance(*p2);

    if dist < 0.1 {
        // Give it some offset force.
        Vec3::new(0.0, 0.2, 0.0)
    } else {
        // This is already multiplied by -1.0, i.e. (p2 - p1) == (p1 - p2) * -1.0
        (*p2 - *p1) / dist * f32::log2(dist / rest_length)
    }
}

/// Computes a repulsion force between two points with a given strength.
fn compute_repulsion_force(p1: &Vec3, p2: &Vec3, repulsion_strength: f32) -> Vec3 {
    let dist = p1.distance_squared(*p2);

    if dist < 1.0 {
        // Give it some offset force.
        Vec3::new(0.0, 0.0, 0.0)
    } else {
        (*p1 - *p2) * repulsion_strength / dist
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use merc_lts::read_aut;

    use super::GraphLayout;

    #[test]
    fn test_graph_layout() {
        let file = include_str!("../../../../examples/lts/abp.aut");
        let lts = Arc::new(read_aut(file.as_bytes(), vec![]).unwrap());

        let mut layout = GraphLayout::new(lts);

        // Perform a number of updates
        layout.update(5.0, 1.0, 0.01);
        layout.update(5.0, 1.0, 0.01);
        layout.update(5.0, 1.0, 0.01);
        layout.update(5.0, 1.0, 0.01);
        layout.update(5.0, 1.0, 0.01);
    }
}
