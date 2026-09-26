# Phase 4 — GUI (`tools/gui`)

Coverage: `tools/gui/ltsgraph` (the `merc-ltsgraph` binary: `main.rs`,
`renderer.rs`, `pauseable_thread.rs`, `error_dialog.rs`, `wgpu.rs`) and
`tools/gui/ltsgraph-lib` (`graph_layout.rs`, `viewer.rs`, `renderer_skia.rs`,
`renderer_femtovg.rs`, `text_cache.rs`). This is a separate cargo workspace
(`tools/gui`); all commands below are run from there. This code was
previously untouched by review ("Phase 4 ... not started" in the README).

No `unsafe` code exists anywhere in this scope (`grep -rn unsafe tools/gui`
turns up only the identifier `merc_unsafety`, the crate name of an
already-reviewed dependency used through its safe API); `unsafe-verify` does
not apply here.

## Verdict

Not sound. `tools/gui/ltsgraph/src/main.rs` reloads an LTS by replacing three
independent `Mutex`-guarded fields (`state.viewer`, `state.graph_layout`,
`state.lts`/`reload_lts`) one after another with no single lock covering the
transition, while a separate render thread and a separate layout thread read
those fields concurrently and combine values taken from different fields
without re-checking that they still agree in size. Loading a new LTS whose
state count differs from the previous one — a completely ordinary user action
(open another file from the toolbar while the previous graph is still
animating, which is the common case for any LTS that has not fully
stabilized) — can panic a background thread with an index-out-of-bounds
inside `Viewer::update`, `SkiaRenderer::render`, or `FemtovgRenderer::render`,
depending on which direction the size changed and which thread wins the race.
`PauseableThread`'s `Drop` swallows a worker panic (logs it) so this does not
tear down the whole GUI, but the graph being displayed is silently left in
whatever partially-updated state it was in, or the layout/render thread stops
functioning until the next explicit `resume()` finds it out of sync again.
Separately, `Viewer::new`'s parallel-edge fan-out has a real, deterministic
logic bug: parallel transitions between the same two states are only spread
apart when there is a transition back the other way; otherwise every parallel
edge collapses onto the exact same handle position and is indistinguishable
in the rendered graph.

## Findings

### 1. Non-atomic LTS reload lets background threads observe a `Viewer`/renderer mismatched in size with the `GraphLayout`/renderer's own LTS — CONFIRMED (two independent repros)

- **Location**: `tools/gui/ltsgraph/src/main.rs:346-361` (`load_lts`, the
  `state.viewer` / `state.graph_layout` / `state.lts` replacement sequence,
  three separate `Mutex::lock()` calls with no shared lock across them);
  consumed by `tools/gui/ltsgraph-lib/src/viewer.rs:102-106` (`Viewer::update`)
  and `tools/gui/ltsgraph-lib/src/renderer_skia.rs:114-121` /
  `tools/gui/ltsgraph-lib/src/renderer_femtovg.rs:71-78`
  (`SkiaRenderer::render` / `FemtovgRenderer::render`, both index
  `viewer.state_view()` using the renderer's own, separately-cached LTS's
  state/transition indices).
- **Scenario A (renderer stale, shrink)**: the layout/render worker threads
  run continuously while a graph has not stabilized (typical for anything
  non-trivial). If the user loads a *smaller* LTS while the previous, bigger
  one is still animating, `main.rs` replaces `state.viewer` with a `Viewer`
  for the new, smaller LTS before it sets `state.reload_lts = true` (which is
  what makes the render thread rebuild `SkiaRenderer`/`FemtovgRenderer` for
  the new LTS via `Renderer::reload`). In that window the render thread can
  see the new (smaller) viewer while its own `SkiaRenderer`/`FemtovgRenderer`
  is still wrapping the old (bigger) LTS, iterate the old LTS's state/edge
  indices, and index straight past the end of the new viewer's
  `state_view()` slice.
- **Scenario B (layout stale, grow)**: symmetric case in the other direction.
  `main.rs` replaces `state.viewer` before `state.graph_layout`. If the user
  loads a *bigger* LTS, the layout thread can take `state.graph_layout`
  before it is replaced (still the old, smaller layout) and then take
  `state.viewer` after it has been replaced (already the new, bigger
  viewer), and call `viewer.update(&old_smaller_layout)`, which indexes the
  old layout's (shorter) `layout_states` using the new viewer's (longer)
  index range.
- **Evidence**:
  - `cargo test -p merc_ltsgraph_lib --test renderer_stale_lts_after_reload`
    fails:
    ```
    thread '...' panicked at ltsgraph-lib/src/renderer_skia.rs:121:57:
    index out of bounds: the len is 2 but the index is 2
    ...
    SkiaRenderer::render panicked (index out of bounds) when handed a Viewer
    backed by an LTS smaller than the renderer's own cached LTS -- ...
    test result: FAILED. 0 passed; 1 failed
    ```
  - `cargo test -p merc_ltsgraph_lib --lib viewer::tests::update_does_not_panic_when_layout_lts_is_smaller_than_viewer_lts`
    fails:
    ```
    thread '...' panicked at ltsgraph-lib/src/viewer.rs:104:57:
    index out of bounds: the len is 2 but the index is 2
    ...
    Viewer::update panicked (index out of bounds) when handed a GraphLayout
    backed by an LTS smaller than the viewer's own LTS -- ...
    test result: FAILED. 1 passed; 2 failed
    ```
    (the `parallel_transitions_...` failure in that same run is finding #2,
    below; unrelated to this one).
  - Both tests reproduce the underlying indexing bug directly (constructing
    a mismatched pair by hand) rather than through the actual thread race, so
    they are deterministic and do not depend on timing; the race in
    `main.rs` is what makes the mismatched pair reachable in the running
    application, not part of what the tests exercise.
- **Why the tests would pass once fixed**: once the renderer is guaranteed to
  be reloaded (or the viewer/layout pair guaranteed to be swapped) atomically
  with respect to `state.viewer`/`state.graph_layout`/`state.lts`, no thread
  can observe a `Viewer` and a `GraphLayout`/cached-LTS-in-`Renderer` of
  different sizes together, and both `catch_unwind`s return `Ok`.
- **Direction of a fix**: replace the three independent `Mutex` fields with
  one `Mutex<ReloadState>` (or an `ArcSwap` of an atomically-swapped bundle)
  so a reload is visible to readers as a single atomic transition, or have
  `Renderer::reload` and the viewer/layout swap happen under one combined
  lock before `reload_lts` is observed.

### 2. Parallel transitions between the same two states overlap unless a back-transition also exists — CONFIRMED

- **Location**: `tools/gui/ltsgraph-lib/src/viewer.rs:72-94` (`Viewer::new`,
  the non-self-loop branch).
- **Scenario**: `num_parallel` and the per-target running `index` are
  computed unconditionally, and the surrounding comment states the running
  index exists to "fan out multiple edges that share the same source and
  target so their handles do not overlap" — but `transition_view.handle_offset`
  is only ever assigned inside `if has_backtransition`. Two (or more) parallel
  transitions from state A to state B, with no transition from B back to A,
  are therefore all left at the default `Vec3::ZERO` handle offset and are
  rendered exactly on top of each other, indistinguishable in the graph.
- **Evidence**: `cargo test -p merc_ltsgraph_lib --lib viewer::tests::parallel_transitions_without_back_transition_get_distinct_handle_offsets`
  fails:
  ```
  assertion `left != right` failed: two parallel transitions from state 0 to
  state 1 were given the same handle offset (Vec3(0.0, 0.0, 0.0)), so they
  will be drawn exactly on top of each other
    left: Vec3(0.0, 0.0, 0.0)
   right: Vec3(0.0, 0.0, 0.0)
  ```
  Input: `des (0,2,2)\n(0,"a",1)\n(0,"b",1)\n"` (two parallel transitions
  0->1, no 1->0 transition).
- **Why the test would pass once fixed**: once the fan-out offset is applied
  whenever `num_parallel > 1` (not only when `has_backtransition`), the two
  transitions get distinct offsets and the assertion holds.
- **Direction of a fix**: move the `transition_view.handle_offset = ...`
  assignment out of the `if has_backtransition` guard (or add an `||
  num_parallel > 1` condition), keeping the two conditions'
  distinct offset shapes (radial fan vs. left/right split) if that
  distinction is intentional for the visualization.

## Checked and found correct

- **Self-loop handle offsets** (`viewer.rs`, `num_selfloops`): cannot divide
  by zero — `num_selfloops` is only used inside the branch that is itself
  counted by it (`state_index == transition.to`), so it is always >= 1 there.
  An existing regression test
  (`test_handle_offsets_are_finite_with_back_transitions`) already covers a
  related, previously-fixed NaN bug and still passes.
- **`Viewer::center()` / `GraphLayout::update`'s stability ratio** divide by
  `num_of_states()`, which looks unguarded against zero, but
  `LabelledTransitionSystem::new` (`crates/lts/src/labelled_transition_system.rs:96-99`)
  always grows the state vector to at least `initial_state.value() + 1`, so
  an LTS with zero states cannot exist; not reachable.
  `GraphLayout::new`'s per-state `rng.random_range(-bound..bound)` loop
  (which would panic on an empty range if `bound == 0.0`) is likewise
  unreachable since it only runs when `num_of_states() >= 1`.
- **`compute_spring_force`/`compute_repulsion_force`** explosion risk from a
  large `delta` (timestep): the UI's own default global settings
  (`ui/application.slint`: `handle_length: 50.0`, `repulsion_strength: 5.0`,
  `timestep: 15.0`) are much larger than the values the existing unit test
  uses (`5.0, 1.0, 0.01`), which looked like a candidate for the
  `assert!(state_layout.position.is_finite())` safety check to trip (and,
  since `tools/gui/Cargo.toml` sets `panic = "abort"` for release builds,
  abort the whole process rather than show an error dialog). Probed directly
  with `GraphLayout::update(50.0, 5.0, 15.0)` for 200 iterations against
  `examples/lts/abp.aut`: it neither panics nor even fails to stabilize,
  because both force functions decay with distance (spring force is
  `log2(dist/rest_length)/dist`, repulsion is `strength/dist^2`), which
  self-limits the system even at a large timestep. No defect here; probe test
  not kept (it did not fail).
- **`PauseableThread`**: the `resume_generation` bookkeeping correctly avoids
  losing a `resume()` that races a self-pause; `stop()`/`Drop` correctly logs
  rather than re-panicking on a worker panic during drop (would otherwise
  abort under `panic = "abort"`). No defect found; existing tests
  (`test_pausablethread`, `test_pauseablethread_surfaces_errors`) already
  cover the surfaced-error path.
- **`repack_padded_rows`/`align_up`** (GPU readback row unpadding): correct
  for realistic canvas sizes; existing test
  (`test_repack_padded_rows_strips_padding`) already covers the non-aligned
  width case. (`align_up` can wrap for `width` near `u32::MAX`, but that is
  not reachable from any real window/canvas size and is not a GUI-specific
  defect worth reporting.)
- **`text_cache.rs`** glyph rendering: mask vs. color glyph handling already
  normalizes to RGBA8 (a comment references a previously-fixed panic here);
  no further defect found.
- **No `unsafe` code** anywhere in `tools/gui/ltsgraph` or
  `tools/gui/ltsgraph-lib`.

## Tests added

- `tools/gui/ltsgraph-lib/tests/renderer_stale_lts_after_reload.rs` —
  `render_does_not_panic_when_viewer_lts_is_smaller_than_renderer_lts`.
  Run: `cd tools/gui && cargo test -p merc_ltsgraph_lib --test renderer_stale_lts_after_reload`.
- `tools/gui/ltsgraph-lib/src/viewer.rs` (`#[cfg(test)] mod tests`) — two new
  tests:
  - `parallel_transitions_without_back_transition_get_distinct_handle_offsets`
  - `update_does_not_panic_when_layout_lts_is_smaller_than_viewer_lts`
  Run: `cd tools/gui && cargo test -p merc_ltsgraph_lib --lib viewer::`.

All three fail on the current tree (see evidence above) and are left in the
tree as regression tests for the implementor pass.
