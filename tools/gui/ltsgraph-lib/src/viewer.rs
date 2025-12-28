use std::sync::Arc;

use glam::Mat3;
use glam::Vec3;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use merc_lts::StateIndex;

use crate::graph_layout::GraphLayout;

pub struct Viewer {
    /// The underlying LTS being displayed
    lts: Arc<LabelledTransitionSystem<String>>,

    /// Stores a local copy of the state positions
    view_states: Vec<StateView>,
}

#[derive(Clone, Default)]
pub struct StateView {
    pub position: Vec3,
    pub outgoing: Vec<TransitionView>,
}

#[derive(Clone, Default)]
pub struct TransitionView {
    /// The offset of the handle w.r.t. the 'from' state
    pub handle_offset: Vec3,
}

impl Viewer {
    /// Creates a new viewer for the given LTS
    pub fn new(lts: Arc<LabelledTransitionSystem<String>>) -> Viewer {
        // Initialize the view information for the states
        let mut view_states = vec![StateView::default(); lts.num_of_states()];

        // Add the transition view information
        for (state_index, state_view) in view_states.iter_mut().enumerate() {
            let state_index = StateIndex::new(state_index);

            state_view.outgoing = vec![TransitionView::default(); lts.outgoing_transitions(state_index).count()];

            // Compute the offsets for self-loops, put them at equal distance around the state
            let num_selfloops = lts
                .outgoing_transitions(state_index)
                .filter(|transition| transition.to == state_index)
                .count();

            // Keep track of the current self loop index
            let mut index_selfloop = 0;

            // Keep track of the current transition index
            let mut index_transition = 0;

            for (transition_index, transition) in lts.outgoing_transitions(state_index).enumerate() {
                let transition_view = &mut state_view.outgoing[transition_index];

                if state_index == transition.to {
                    // This is a self loop so compute a rotation around the state for its handle
                    let rotation_mat = Mat3::from_euler(
                        glam::EulerRot::XYZ,
                        0.0,
                        0.0,
                        (index_selfloop as f32 / num_selfloops as f32) * 2.0 * std::f32::consts::PI,
                    );
                    transition_view.handle_offset = rotation_mat.mul_vec3(Vec3::new(0.0, -40.0, 0.0));

                    index_selfloop += 1;
                } else {
                    // Determine whether any of the outgoing edges from the reached state point back
                    let has_backtransition = lts
                        .outgoing_transitions(transition.to)
                        .filter(|transition| transition.to == state_index)
                        .count()
                        > 0;

                    // Compute the number of transitions going to the same state
                    let num_transitions = lts
                        .outgoing_transitions(state_index)
                        .filter(|transition| transition.to == state_index)
                        .count();

                    if has_backtransition {
                        // Offset the outgoing transitions towards that state to the right
                        transition_view.handle_offset =
                            Vec3::new(0.0, index_transition as f32 / num_transitions as f32, 0.0);
                    } else {
                        // Balance transitions around the midpoint
                    }

                    index_transition += 1;
                }
            }
        }

        Viewer { lts, view_states }
    }

    /// Update the state of the viewer with the given graph layout
    pub fn update(&mut self, layout: &GraphLayout) {
        for (index, layout_state) in self.view_states.iter_mut().enumerate() {
            layout_state.position = layout.layout_states[index].position;
        }
    }

    /// Returns the center of the graph
    pub fn center(&self) -> Vec3 {
        self.view_states.iter().map(|x| x.position).sum::<Vec3>() / self.view_states.len() as f32
    }

    /// Gets a reference to the state views for testing and rendering
    pub fn state_view(&self) -> &[StateView] {
        &self.view_states
    }

    /// Gets a reference to the LTS that is being displayed
    pub fn lts(&self) -> &LabelledTransitionSystem<String> {
        &self.lts
    }
}
