//! Regression test for a state-management bug: `SkiaRenderer` caches the `LabelledTransitionSystem`
//! it was built for and indexes `Viewer::state_view()` using *its own* state/label indices. In
//! `tools/gui/ltsgraph/src/main.rs`, `state.viewer` (and `state.graph_layout`) are replaced with a
//! freshly loaded LTS's data *before* `state.reload_lts` is set and observed by the render thread,
//! which is what actually rebuilds the `Renderer`'s `SkiaRenderer`/`FemtovgRenderer` for the new LTS
//! (see `Renderer::reload`). Because `state.viewer` and the renderer's cached LTS live behind two
//! independent `Mutex`es updated from different threads, there is a real window in which the render
//! thread calls `renderer.render(new_viewer, ...)` while the renderer itself is still wrapping the
//! *previous* LTS.
//!
//! This is harmless when the newly loaded LTS is the same size or larger, but when the user loads a
//! *smaller* LTS after a bigger one, the (still-old, bigger) renderer iterates its own state indices
//! and indexes straight into the new, smaller `Viewer::state_view()` slice, which panics.
//!
//! This test reproduces the underlying indexing bug directly (no threads required): construct a
//! `SkiaRenderer` for a 4-state LTS, then ask it to render a `Viewer` for an unrelated 2-state LTS,
//! exactly the combination the render thread can observe transiently during a reload to a smaller
//! model.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use merc_lts::read_aut;
use merc_ltsgraph_lib::SkiaRenderer;
use merc_ltsgraph_lib::Viewer;
use tiny_skia::Pixmap;
use tiny_skia::PixmapMut;

#[test]
fn render_does_not_panic_when_viewer_lts_is_smaller_than_renderer_lts() {
    // The renderer was (still) built for a 4-state ring LTS...
    let big = "des (0,4,4)\n(0,\"a\",1)\n(1,\"a\",2)\n(2,\"a\",3)\n(3,\"a\",0)\n";
    let big_lts = Arc::new(read_aut(big.as_bytes()).unwrap());

    // ...but the viewer it is handed already belongs to a freshly (re)loaded, smaller 2-state LTS,
    // as happens transiently in ltsgraph's render thread between `state.viewer` being replaced and
    // `Renderer::reload` catching up to the new LTS.
    let small = "des (0,1,2)\n(0,\"a\",1)\n";
    let small_lts = Arc::new(read_aut(small.as_bytes()).unwrap());

    let mut renderer = SkiaRenderer::new(big_lts);
    let viewer = Viewer::new(small_lts);

    let mut pixmap = Pixmap::new(64, 64).unwrap();
    let mut pixmap_mut = PixmapMut::from_bytes(pixmap.data_mut(), 64, 64).unwrap();

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        renderer.render(&mut pixmap_mut, &viewer, true, 5.0, 0.0, 0.0, 64, 64, 1.0, 12.0);
    }));

    assert!(
        result.is_ok(),
        "SkiaRenderer::render panicked (index out of bounds) when handed a Viewer backed by an \
         LTS smaller than the renderer's own cached LTS -- this is exactly the transient state \
         ltsgraph's render thread can observe while reloading to a smaller model"
    );
}
