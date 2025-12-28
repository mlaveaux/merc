use std::sync::Arc;

use cosmic_text::Metrics;
use glam::Vec2;
use glam::Vec3Swizzles;
use merc_lts::LTS;
use merc_lts::LabelledTransitionSystem;
use tiny_skia::PixmapMut;
use tiny_skia::Shader;
use tiny_skia::Stroke;
use tiny_skia::Transform;

use crate::Viewer;
use crate::text_cache::TextCache;

/// Handles the rendering of LTS graphs using Skia
pub struct SkiaRenderer {
    /// The underlying LTS being displayed
    lts: Arc<LabelledTransitionSystem<String>>,

    /// A cache used to cache strings and font information
    text_cache: TextCache,

    /// A buffer for transition labels
    labels_cache: Vec<cosmic_text::Buffer>,
}

impl SkiaRenderer {
    /// Creates a new renderer for the given LTS
    pub fn new(lts: Arc<LabelledTransitionSystem<String>>) -> Self {
        let mut text_cache = TextCache::new();
        let mut labels_cache = vec![];

        for label in lts.labels() {
            // Create text elements for all labels that we are going to render
            let buffer = text_cache.create_buffer(label, Metrics::new(12.0, 12.0));

            // Put it in the label cache
            labels_cache.push(buffer);
        }

        SkiaRenderer {
            lts,
            text_cache,
            labels_cache,
        }
    }

    /// Renders the LTS graph to the provided pixmap
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        pixmap: &mut PixmapMut,
        viewer: &Viewer,
        draw_actions: bool,
        state_radius: f32,
        view_x: f32,
        view_y: f32,
        screen_x: u32,
        screen_y: u32,
        zoom_level: f32,
        label_text_size: f32,
    ) {
        pixmap.fill(tiny_skia::Color::WHITE);

        // Compute the view transform
        let view_transform = Transform::from_translate(view_x, view_y)
            .post_scale(zoom_level, zoom_level)
            .post_translate(screen_x as f32 / 2.0, screen_y as f32 / 2.0);

        // The color information for states
        let state_inner_paint = tiny_skia::Paint {
            shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(255, 255, 255, 255)),
            ..Default::default()
        };
        let initial_state_paint = tiny_skia::Paint {
            shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(100, 255, 100, 255)),
            ..Default::default()
        };
        let state_outer = tiny_skia::Paint {
            shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(0, 0, 0, 255)),
            ..Default::default()
        };

        // The color information for edges
        let edge_paint = tiny_skia::Paint::default();

        // The arrow to indicate the direction of the edge
        let arrow = {
            let mut builder = tiny_skia::PathBuilder::new();
            builder.line_to(2.0, -5.0);
            builder.line_to(-2.0, -5.0);
            builder.close();
            builder.finish().unwrap()
        };

        // A single circle that is used to render colored states
        let circle = {
            let mut builder = tiny_skia::PathBuilder::new();
            builder.push_circle(0.0, 0.0, state_radius);
            builder.finish().unwrap()
        };

        // Resize the labels if necessary
        for buffer in &mut self.labels_cache {
            self.text_cache
                .resize(buffer, Metrics::new(label_text_size, label_text_size));
        }

        // Draw the edges and the arrows on them
        let mut edge_builder = tiny_skia::PathBuilder::new();
        let mut arrow_builder = tiny_skia::PathBuilder::new();

        for state_index in self.lts.iter_states() {
            let state_view = &viewer.state_view()[state_index];

            // For now we only draw 2D graphs properly
            debug_assert!(state_view.position.z.abs() < 0.01);

            for (transition_index, transition) in self.lts.outgoing_transitions(state_index).enumerate() {
                let to_state_view = &viewer.state_view()[transition.to];
                let transition_view = &state_view.outgoing[transition_index];

                let label_position = if transition.to != state_index {
                    // Draw the transition
                    edge_builder.move_to(state_view.position.x, state_view.position.y);
                    edge_builder.line_to(to_state_view.position.x, to_state_view.position.y);

                    let direction = (state_view.position - to_state_view.position).normalize();
                    let angle = -direction.xy().angle_to(Vec2::new(0.0, -1.0)).to_degrees();

                    // Draw the arrow of the transition
                    if let Some(path) = arrow.clone().transform(
                        Transform::from_translate(0.0, -state_radius - 0.5)
                            .post_rotate(angle)
                            .post_translate(to_state_view.position.x, to_state_view.position.y),
                    ) {
                        arrow_builder.push_path(&path);
                    };

                    // Draw the edge handle
                    let middle = (to_state_view.position + state_view.position) / 2.0;
                    edge_builder.push_circle(
                        middle.x + transition_view.handle_offset.x,
                        middle.y + transition_view.handle_offset.y,
                        1.0,
                    );

                    middle
                } else {
                    // This is a self loop so draw a circle around the middle of the position and the handle
                    let middle = (2.0 * state_view.position + transition_view.handle_offset) / 2.0;
                    edge_builder.push_circle(middle.x, middle.y, transition_view.handle_offset.length() / 2.0);

                    // Draw the edge handle
                    edge_builder.push_circle(
                        state_view.position.x + transition_view.handle_offset.x,
                        state_view.position.y + transition_view.handle_offset.y,
                        1.0,
                    );
                    state_view.position + transition_view.handle_offset
                };

                // Draw the text label
                if draw_actions {
                    let buffer = &self.labels_cache[transition.label];
                    self.text_cache.draw(
                        buffer,
                        pixmap,
                        Transform::from_translate(label_position.x, label_position.y).post_concat(view_transform),
                    );
                }
            }
        }

        if let Some(path) = arrow_builder.finish() {
            pixmap.fill_path(&path, &edge_paint, tiny_skia::FillRule::Winding, view_transform, None);
        }

        // Draw the path for edges
        if let Some(path) = edge_builder.finish() {
            pixmap.stroke_path(&path, &edge_paint, &Stroke::default(), view_transform, None);
        }

        // Draw the states on top
        let mut state_path_builder = tiny_skia::PathBuilder::new();

        for (index, state_view) in viewer.state_view().iter().enumerate() {
            if index != *self.lts.initial_state_index() {
                state_path_builder.push_circle(state_view.position.x, state_view.position.y, state_radius);
            } else {
                // Draw the colored states individually
                let transform =
                    Transform::from_translate(state_view.position.x, state_view.position.y).post_concat(view_transform);

                pixmap.fill_path(
                    &circle,
                    &initial_state_paint,
                    tiny_skia::FillRule::Winding,
                    transform,
                    None,
                );

                pixmap.stroke_path(&circle, &state_outer, &Stroke::default(), transform, None);
            }
        }

        // Draw the states with an outline
        if let Some(path) = state_path_builder.finish() {
            pixmap.fill_path(
                &path,
                &state_inner_paint,
                tiny_skia::FillRule::Winding,
                view_transform,
                None,
            );

            pixmap.stroke_path(&path, &state_outer, &Stroke::default(), view_transform, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_lts::read_aut;
    use std::sync::Arc;
    use tiny_skia::Pixmap;
    use tiny_skia::PixmapMut;

    use super::*;
    use crate::viewer::Viewer;

    #[test]
    fn test_skia_renderer() {
        // Render a single frame from the alternating bit protocol with some settings
        let file = include_str!("../../../../examples/lts/abp.aut");
        let lts = Arc::new(read_aut(file.as_bytes(), vec![]).unwrap());

        // Create a viewer and renderer
        let viewer = Viewer::new(lts.clone());
        let mut renderer = SkiaRenderer::new(lts);

        let mut pixel_buffer = Pixmap::new(800, 600).unwrap();
        renderer.render(
            &mut PixmapMut::from_bytes(pixel_buffer.data_mut(), 800, 600).unwrap(),
            &viewer,
            true,
            5.0,
            0.0,
            0.0,
            800,
            600,
            1.0,
            14.0,
        );
    }
}
