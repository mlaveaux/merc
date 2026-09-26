use std::collections::HashMap;
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

            // Running index of parallel transitions per destination state, used to fan out
            // multiple edges that share the same source and target so their handles do not overlap.
            let mut index_per_target: HashMap<StateIndex, usize> = HashMap::new();

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
                        .any(|back| back.to == state_index);

                    // Number of parallel transitions from this state to the same destination.
                    // This always counts the current transition, so it is at least one.
                    let num_parallel = lts
                        .outgoing_transitions(state_index)
                        .filter(|other| other.to == transition.to)
                        .count();

                    let index = index_per_target.entry(transition.to).or_insert(0);

                    if has_backtransition {
                        // Offset the parallel outgoing transitions towards that state to the right
                        // so the back- and forward-transitions do not overlap.
                        transition_view.handle_offset = Vec3::new(0.0, *index as f32 / num_parallel as f32, 0.0);
                    }

                    *index += 1;
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

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;

    use merc_lts::read_aut;

    use crate::graph_layout::GraphLayout;
    use crate::viewer::Viewer;

    #[test]
    fn test_handle_offsets_are_finite_with_back_transitions() {
        // Two states with a transition and a matching back-transition, but no self-loops.
        // This previously divided by a zero self-loop count and produced NaN handle offsets.
        let aut = "des (0,2,2)\n(0,\"a\",1)\n(1,\"b\",0)\n";
        let lts = Arc::new(read_aut(aut.as_bytes()).unwrap());

        let viewer = Viewer::new(lts);

        for state_view in viewer.state_view() {
            for transition_view in &state_view.outgoing {
                assert!(
                    transition_view.handle_offset.is_finite(),
                    "Non-finite handle offset {} computed",
                    transition_view.handle_offset
                );
            }
        }
    }

    #[test]
    fn parallel_transitions_without_back_transition_get_distinct_handle_offsets() {
        // Two parallel transitions from state 0 to state 1 (via different labels), and no
        // transition from 1 back to 0. `num_parallel` is computed unconditionally as "the number
        // of parallel transitions from this state to the same destination" and the surrounding
        // comment says the per-target running index exists to "fan out multiple edges that share
        // the same source and target so their handles do not overlap" -- but the fan-out offset is
        // only ever assigned inside the `has_backtransition` branch, so parallel transitions with
        // no back-transition are left at the default zero offset and are rendered on top of each
        // other.
        let aut = "des (0,2,2)\n(0,\"a\",1)\n(0,\"b\",1)\n";
        let lts = Arc::new(read_aut(aut.as_bytes()).unwrap());

        let viewer = Viewer::new(lts);
        let outgoing = &viewer.state_view()[0].outgoing;
        assert_eq!(outgoing.len(), 2);

        assert_ne!(
            outgoing[0].handle_offset, outgoing[1].handle_offset,
            "two parallel transitions from state 0 to state 1 were given the same handle offset \
             ({:?}), so they will be drawn exactly on top of each other",
            outgoing[0].handle_offset
        );
    }

    #[test]
    fn update_does_not_panic_when_layout_lts_is_smaller_than_viewer_lts() {
        // `tools/gui/ltsgraph/src/main.rs` reloads an LTS by replacing `state.viewer` and
        // `state.graph_layout` (backed by the same new LTS) through two independent `Mutex`es, one
        // right after the other, while a separate thread runs `viewer.update(&graph_layout)`
        // whenever it can take both locks. That thread can observe `state.viewer` already updated
        // to the new LTS while `state.graph_layout` still holds the previous one's layout.
        //
        // This test reproduces the resulting mismatch directly: a `Viewer` for a bigger LTS is
        // updated from a `GraphLayout` belonging to a smaller, unrelated LTS -- exactly what
        // `Viewer::update` can be handed during that window when the newly loaded LTS has *more*
        // states than the one it replaced.
        let small = "des (0,1,2)\n(0,\"a\",1)\n";
        let small_lts = Arc::new(read_aut(small.as_bytes()).unwrap());

        let big = "des (0,4,4)\n(0,\"a\",1)\n(1,\"a\",2)\n(2,\"a\",3)\n(3,\"a\",0)\n";
        let big_lts = Arc::new(read_aut(big.as_bytes()).unwrap());

        let mut viewer = Viewer::new(big_lts);
        let stale_layout = GraphLayout::new(small_lts);

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| viewer.update(&stale_layout)));

        assert!(
            result.is_ok(),
            "Viewer::update panicked (index out of bounds) when handed a GraphLayout backed by an \
             LTS smaller than the viewer's own LTS -- this is exactly the transient state \
             ltsgraph's layout thread can observe while reloading to a bigger model"
        );
    }
}
