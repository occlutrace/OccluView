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

/// A full synthetic session: load, orbit, select, sculpt stroke with
/// densification, burst load, clear. Mirrors the producer/consumer order of
/// `render_pending_frame` (one path per frame) and the offscreen render path.
#[test]
fn scripted_session_keeps_each_consumer_exact() {
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
    // The fallback path runs and catches up its own cursors.
    state.consume_offscreen_scene();
    state.consume_offscreen_overlay();
    steps += 1;
    assert_all_clean(&state, "after initial sync");

    // Selection change stales only the overlays; live consumes first.
    state.selection_changed();
    steps += 1;
    assert!(!state.live_scene_stale() && !state.offscreen_scene_stale());
    state.consume_redraw();
    state.consume_live_overlay();
    assert!(!state.live_overlay_stale());
    assert!(state.offscreen_overlay_stale());
    state.consume_offscreen_overlay();
    steps += 1;
    assert_all_clean(&state, "after selection sync");

    // Mid-stroke densification: scenes stale, overlay spared.
    state.sculpt_topology_changed();
    steps += 1;
    assert!(state.live_scene_stale() && state.offscreen_scene_stale());
    assert!(!state.live_overlay_stale() && !state.offscreen_overlay_stale());
    state.consume_redraw();
    state.consume_live_scene();
    state.consume_offscreen_scene();
    steps += 1;
    assert_all_clean(&state, "after sculpt resync");

    // Burst load: final scene marks everything, intermediate frames suppress.
    state.scene_geometry_changed();
    state.suppress_redraw();
    steps += 1;
    assert!(!state.redraw_pending());
    assert!(state.live_scene_stale() && state.offscreen_scene_stale());
    state.scene_geometry_changed();
    steps += 1;
    assert!(state.redraw_pending());
    state.consume_redraw();
    state.consume_live_scene();
    state.consume_live_overlay();
    state.consume_offscreen_scene();
    state.consume_offscreen_overlay();
    steps += 1;
    assert_all_clean(&state, "after burst load");

    // Clear resets every cursor.
    state.scene_geometry_changed();
    state.reset();
    steps += 1;
    assert_all_clean(&state, "after clear");

    assert_eq!(steps, 12, "the scripted session covers twelve cause steps");
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
