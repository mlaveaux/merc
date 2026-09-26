use std::sync::Arc;

use femtovg::Canvas;
use femtovg::Color;
use femtovg::Paint;
use femtovg::Path;
use femtovg::Renderer;
use glam::Vec2;
use glam::Vec3Swizzles;

use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;

use crate::Viewer;

pub struct FemtovgRenderer {
    /// Reference to the LTS being rendered
    lts: Arc<LabelledTransitionSystem<String>>,
}

impl FemtovgRenderer {
    pub fn new(lts: Arc<LabelledTransitionSystem<String>>) -> Self {
        Self { lts }
    }

    /// Render the current state of the simulation into the canvas.
    #[allow(clippy::too_many_arguments)]
    pub fn render<T: Renderer>(
        &mut self,
        canvas: &mut Canvas<T>,
        viewer: &Viewer,
        draw_actions_labels: bool,
        state_radius: f32,
        view_x: f32,
        view_y: f32,
        screen_x: u32,
        screen_y: u32,
        zoom_level: f32,
        label_text_size: f32,
    ) -> Result<(), femtovg::ErrorKind> {
        canvas.set_size(screen_x, screen_y, 1.0);
        canvas.reset();

        // Clear the canvas with white color
        canvas.clear_rect(0, 0, screen_x, screen_y, Color::white());

        canvas.save();
        canvas.translate(view_x, view_y);
        canvas.scale(zoom_level, zoom_level);
        canvas.translate(screen_x as f32 / 2.0, screen_y as f32 / 2.0);

        // The color information for states
        let state_inner_paint = Paint::color(Color::white());
        let initial_state_paint = Paint::color(Color::rgb(100, 255, 100));

        let mut text_paint = Paint::color(Color::black());
        text_paint.set_font_size(label_text_size);
        text_paint.set_line_width(1.0);

        let mut state_outer = Paint::color(Color::black());
        state_outer.set_line_width(1.0);

        // The color information for edges
        let mut edge_paint = Paint::color(Color::black());
        edge_paint.set_line_width(1.0);

        let mut edge_path = Path::new();
        let mut arrow_path = Path::new();

        // Draw the edges and the arrows on them
        for state_index in self.lts.iter_states() {
            // `viewer` is expected to belong to this renderer's own `self.lts`, but this does not
            // panic if it momentarily does not (e.g. `viewer` was just swapped for a differently
            // sized LTS and this renderer has not been rebuilt via `reload` yet): skip states and
            // transitions the viewer does not have rather than indexing out of bounds.
            let Some(state_view) = viewer.state_view().get(state_index.value()) else {
                continue;
            };

            // For now we only draw 2D graphs properly
            debug_assert!(state_view.position.z.abs() < 0.01);

            for (transition_index, transition) in self.lts.outgoing_transitions(state_index).enumerate() {
                let Some(to_state_view) = viewer.state_view().get(transition.to.value()) else {
                    continue;
                };
                let Some(transition_view) = state_view.outgoing.get(transition_index) else {
                    continue;
                };

                let label_position = if transition.to != state_index {
                    // Draw the transition line
                    edge_path.move_to(state_view.position.x, state_view.position.y);
                    edge_path.line_to(to_state_view.position.x, to_state_view.position.y);

                    let direction = (state_view.position - to_state_view.position).normalize();
                    let angle = -direction.xy().angle_to(Vec2::new(0.0, -1.0)).to_degrees();

                    // Draw the arrowhead just outside the target state, pointing along the edge.
                    // This mirrors the skia backend's arrow so both renderers look identical.
                    let (sin, cos) = angle.to_radians().sin_cos();
                    let rotate = |p: Vec2| Vec2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
                    let tip = to_state_view.position.xy();
                    let arrow = [
                        rotate(Vec2::new(0.0, -state_radius - 0.5)),
                        rotate(Vec2::new(2.0, -state_radius - 5.5)),
                        rotate(Vec2::new(-2.0, -state_radius - 5.5)),
                    ];
                    arrow_path.move_to(tip.x + arrow[0].x, tip.y + arrow[0].y);
                    arrow_path.line_to(tip.x + arrow[1].x, tip.y + arrow[1].y);
                    arrow_path.line_to(tip.x + arrow[2].x, tip.y + arrow[2].y);
                    arrow_path.close();

                    // Draw the edge handle
                    let middle = (to_state_view.position + state_view.position) / 2.0;
                    let handle_x = middle.x + transition_view.handle_offset.x;
                    let handle_y = middle.y + transition_view.handle_offset.y;

                    edge_path.circle(handle_x, handle_y, 1.0);

                    middle
                } else {
                    // This is a self loop
                    let middle = (2.0 * state_view.position + transition_view.handle_offset) / 2.0;
                    let radius = transition_view.handle_offset.length() / 2.0;

                    edge_path.circle(middle.x, middle.y, radius);

                    // Draw the edge handle
                    let handle_x = state_view.position.x + transition_view.handle_offset.x;
                    let handle_y = state_view.position.y + transition_view.handle_offset.y;

                    edge_path.circle(handle_x, handle_y, 1.0);

                    state_view.position + transition_view.handle_offset
                };

                // Draw the text label
                if draw_actions_labels {
                    // Calculate the transformed position for text
                    canvas.fill_text(
                        label_position.x,
                        label_position.y,
                        &self.lts.labels()[transition.label],
                        &text_paint,
                    )?;
                }
            }
        }

        // Draw all edges first, then states on top.
        canvas.stroke_path(&edge_path, &edge_paint);
        canvas.fill_path(&arrow_path, &edge_paint);

        // Draw the states on top
        let mut state_path = Path::new();
        let mut initial_state_path = Path::new();
        for (index, state_view) in viewer.state_view().iter().enumerate() {
            if index != *self.lts.initial_state_index() {
                // Regular state
                state_path.circle(state_view.position.x, state_view.position.y, state_radius);
            } else {
                // Initial state
                initial_state_path.circle(state_view.position.x, state_view.position.y, state_radius);
            }
        }

        canvas.fill_path(&state_path, &state_inner_paint);
        canvas.fill_path(&initial_state_path, &initial_state_paint);
        canvas.stroke_path(&state_path, &state_outer);
        canvas.stroke_path(&initial_state_path, &state_outer);
        canvas.restore();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use femtovg::Canvas;
    use femtovg::renderer::Void;

    use merc_lts::read_aut;

    use crate::FemtovgRenderer;
    use crate::Viewer;

    #[test]
    fn test_femtovg_renderer() {
        let file = include_str!("../../../../examples/lts/abp.aut");
        let lts = Arc::new(read_aut(file.as_bytes()).unwrap());

        let mut canvas = Canvas::new(Void).unwrap();
        let viewer = Viewer::new(lts.clone());
        let mut renderer = FemtovgRenderer::new(lts);

        renderer
            .render(&mut canvas, &viewer, false, 10.0, 0.0, 0.0, 800, 600, 1.0, 12.0)
            .unwrap();
    }
}
