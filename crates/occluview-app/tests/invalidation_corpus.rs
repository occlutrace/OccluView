//! Deterministic render-invalidation corpus.
//!
//! Schema: a scripted session of [`RenderInvalidation`] causes with the exact
//! consumer visibility expected after each step, covering interleaved live /
//! offscreen consumption, burst suppression, sculpt topology changes, and a
//! mid-session reset. All fixtures are synthetic; no meshes, GPU, or timing.
//!
//! Commands:
//!   cargo test -p occluview-app --test `invalidation_corpus` --locked
//!
//! There are no wall-clock assertions: correctness is exact generation
//! comparison, and the scripted step count is asserted deterministically.

use occluview_app::invalidation::RenderInvalidation;

fn assert_all_clean(state: &RenderInvalidation, step: &str) {
    assert!(!state.redraw_pending(), "{step}: no frame pending");
    assert!(!state.live_scene_stale(), "{step}: live scene fresh");
    assert!(
        !state.offscreen_scene_stale(),
        "{step}: offscreen scene fresh"
    );
    assert!(!state.live_overlay_stale(), "{step}: live overlay fresh");
    assert!(
        !state.offscreen_overlay_stale(),
        "{step}: offscreen overlay fresh"
    );
}

/// A full synthetic live session: load, orbit, select, sculpt stroke with
/// densification, burst load, clear. Only the live cursors advance; the
/// offscreen cursor stays behind from the load, proving one path cannot hide
/// an update from the other. Mirrors `render_pending_frame` dispatching to
/// the live path every frame while a live viewport is attached.
#[test]
fn live_session_keeps_each_consumer_exact() {
    let mut state = RenderInvalidation::new();
    let mut steps = 0;

    // Frame 1: structural load stales everything; the live path consumes.
    state.scene_geometry_changed();
    steps += 1;
    assert!(state.redraw_pending());
    assert!(state.live_scene_stale() && state.offscreen_scene_stale());
    assert!(state.live_overlay_stale() && state.offscreen_overlay_stale());
    state.consume_redraw();
    state.consume_live_scene();
    state.consume_live_overlay();
    steps += 1;

    // Frame 2: camera orbit repaints without touching uploaded geometry.
    // The live consumer is fresh; the offscreen cursor is still behind from
    // the load because that path has not run — and must not be hidden.
    state.request_redraw();
    steps += 1;
    assert!(state.redraw_pending());
    assert!(!state.live_scene_stale());
    assert!(state.offscreen_scene_stale());
    assert!(state.offscreen_overlay_stale());
    state.consume_redraw();
    steps += 1;
    assert!(!state.redraw_pending());
    assert!(!state.live_scene_stale() && !state.live_overlay_stale());

    // Selection change stales only the overlays; the live one consumes.
    state.selection_changed();
    steps += 1;
    assert!(!state.live_scene_stale());
    assert!(state.live_overlay_stale());
    state.consume_redraw();
    state.consume_live_overlay();
    steps += 1;
    assert!(!state.live_overlay_stale());

    // Mid-stroke densification: live scene stale, overlay spared.
    state.sculpt_topology_changed();
    steps += 1;
    assert!(state.live_scene_stale());
    assert!(!state.live_overlay_stale());
    state.consume_redraw();
    state.consume_live_scene();
    steps += 1;
    assert!(!state.live_scene_stale());

    // Burst load: intermediate frames suppress, the final load repaints.
    state.scene_geometry_changed();
    state.suppress_redraw();
    steps += 1;
    assert!(!state.redraw_pending());
    assert!(state.live_scene_stale());
    state.scene_geometry_changed();
    steps += 1;
    assert!(state.redraw_pending());
    state.consume_redraw();
    state.consume_live_scene();
    state.consume_live_overlay();
    steps += 1;
    assert!(!state.redraw_pending());
    assert!(!state.live_scene_stale() && !state.live_overlay_stale());

    // Clear resets every cursor, including the offscreen one left behind.
    state.scene_geometry_changed();
    state.reset();
    steps += 1;
    assert_all_clean(&state, "after clear");

    assert_eq!(steps, 12, "the live session covers twelve cause steps");
}

/// The same synthetic session on the offscreen fallback path: only the
/// offscreen cursors advance while the live cursor stays behind.
#[test]
fn offscreen_session_keeps_each_consumer_exact() {
    let mut state = RenderInvalidation::new();
    let mut steps = 0;

    state.scene_geometry_changed();
    steps += 1;
    state.consume_redraw();
    state.consume_offscreen_scene();
    state.consume_offscreen_overlay();
    steps += 1;
    assert!(!state.offscreen_scene_stale() && !state.offscreen_overlay_stale());
    assert!(state.live_scene_stale() && state.live_overlay_stale());

    state.request_redraw();
    steps += 1;
    state.consume_redraw();
    steps += 1;
    assert!(!state.offscreen_scene_stale());

    state.selection_changed();
    steps += 1;
    assert!(!state.offscreen_scene_stale());
    assert!(state.offscreen_overlay_stale());
    state.consume_redraw();
    state.consume_offscreen_overlay();
    steps += 1;
    assert!(!state.offscreen_overlay_stale());

    state.sculpt_topology_changed();
    steps += 1;
    assert!(state.offscreen_scene_stale());
    assert!(!state.offscreen_overlay_stale());
    state.consume_redraw();
    state.consume_offscreen_scene();
    steps += 1;

    state.scene_geometry_changed();
    state.suppress_redraw();
    steps += 1;
    assert!(!state.redraw_pending());
    assert!(state.offscreen_scene_stale());
    state.scene_geometry_changed();
    steps += 1;
    state.consume_redraw();
    state.consume_offscreen_scene();
    state.consume_offscreen_overlay();
    steps += 1;
    assert!(!state.offscreen_scene_stale() && !state.offscreen_overlay_stale());

    state.scene_geometry_changed();
    state.reset();
    steps += 1;
    assert_all_clean(&state, "after clear");

    assert_eq!(steps, 12, "the offscreen session covers twelve cause steps");
}

/// Live and offscreen cursors advance independently across many generations.
#[test]
fn interleaved_consumers_never_hide_updates_from_each_other() {
    let mut state = RenderInvalidation::new();
    for generation in 1..=64u32 {
        state.scene_geometry_changed();
        state.consume_redraw();
        if generation % 2 == 0 {
            state.consume_live_scene();
            state.consume_live_overlay();
            assert!(state.offscreen_scene_stale());
            assert!(state.offscreen_overlay_stale());
            state.consume_offscreen_scene();
            state.consume_offscreen_overlay();
        } else {
            state.consume_offscreen_scene();
            state.consume_offscreen_overlay();
            assert!(state.live_scene_stale());
            assert!(state.live_overlay_stale());
            state.consume_live_scene();
            state.consume_live_overlay();
        }
        assert_all_clean(&state, "generation {generation}");
    }
}
