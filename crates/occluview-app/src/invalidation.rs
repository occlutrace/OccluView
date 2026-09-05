//! Typed render invalidation for the viewport.
//!
//! Four GPU consumers share three underlying generations: the next frame,
//! the prepared scenes (live and offscreen), and the selection overlay
//! (live and offscreen). Call sites name the semantic cause of a change;
//! each consumer tracks its own cursor, so one path consuming an update
//! cannot hide it from the other path.
//!
//! Costs are preserved from the previous boolean flags: a camera-only change
//! requests a repaint without touching uploaded geometry, a scene change
//! rebuilds both prepared scenes and the overlay, and a mid-stroke sculpt
//! topology change rebuilds the scenes while leaving the overlay alone.

/// Owner of redraw and cache-staleness state for the viewport.
///
/// Lives in the application state; the live and offscreen render paths query
/// and consume their own cursors where they previously read and cleared
/// shared boolean flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderInvalidation {
    frame: u64,
    scene: u64,
    overlay: u64,
    consumed_frame: u64,
    consumed_live_scene: u64,
    consumed_offscreen_scene: u64,
    consumed_live_overlay: u64,
    consumed_offscreen_overlay: u64,
}

impl RenderInvalidation {
    /// Clean state: nothing pending, no consumer stale.
    pub fn new() -> Self {
        Self::default()
    }

    /// The camera moved (orbit, pan, zoom, retarget, refit). Uploaded
    /// geometry is still valid; only the next frame must be painted.
    pub fn request_redraw(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Tool overlays, dialogs, or panels changed presentation state.
    /// Same cost as a camera move: repaint, no geometry work.
    pub fn overlay_tools_changed(&mut self) {
        self.request_redraw();
    }

    /// The selection changed. Both overlay consumers must rebuild; the
    /// prepared scenes are untouched.
    pub fn selection_changed(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.overlay = self.overlay.wrapping_add(1);
    }

    /// Scene structure, geometry, or layer materials changed (load, layer
    /// add/remove/reorder, mesh edit, sculpt commit, align, undo/redo).
    /// Every consumer is stale.
    pub fn scene_geometry_changed(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.scene = self.scene.wrapping_add(1);
        self.overlay = self.overlay.wrapping_add(1);
    }

    /// A mid-stroke sculpt densification replaced the layer's vertex array.
    /// The uploaded geometry is the wrong size, so both prepared scenes must
    /// be rebuilt — but the selection did not change, so the overlay stays.
    pub fn sculpt_topology_changed(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.scene = self.scene.wrapping_add(1);
    }

    /// The scene was cleared. Nothing is pending and no consumer is stale.
    /// Cursors catch up to current generations so a later change is still
    /// detected.
    pub fn reset(&mut self) {
        self.consumed_frame = self.frame;
        self.consumed_live_scene = self.scene;
        self.consumed_offscreen_scene = self.scene;
        self.consumed_live_overlay = self.overlay;
        self.consumed_offscreen_overlay = self.overlay;
    }

    /// Drop a pending frame without acting on it. Used while an open burst
    /// still has queued files: the half-framed scene must not publish, but
    /// the scene and overlay generations stay stale for the final load.
    pub fn suppress_redraw(&mut self) {
        self.consumed_frame = self.frame;
    }

    /// Whether a frame is pending.
    pub fn redraw_pending(&self) -> bool {
        self.consumed_frame != self.frame
    }

    /// Mark the pending frame as dispatched.
    pub fn consume_redraw(&mut self) {
        self.consumed_frame = self.frame;
    }

    /// Whether the live path must re-upload the scene.
    pub fn live_scene_stale(&self) -> bool {
        self.consumed_live_scene != self.scene
    }

    /// Mark the live scene as re-uploaded.
    pub fn consume_live_scene(&mut self) {
        self.consumed_live_scene = self.scene;
    }

    /// Whether the offscreen path must rebuild its prepared scene.
    pub fn offscreen_scene_stale(&self) -> bool {
        self.consumed_offscreen_scene != self.scene
    }

    /// Mark the offscreen scene as rebuilt.
    pub fn consume_offscreen_scene(&mut self) {
        self.consumed_offscreen_scene = self.scene;
    }

    /// Whether the live path must rebuild the selection overlay.
    pub fn live_overlay_stale(&self) -> bool {
        self.consumed_live_overlay != self.overlay
    }

    /// Mark the live overlay as rebuilt.
    pub fn consume_live_overlay(&mut self) {
        self.consumed_live_overlay = self.overlay;
    }

    /// Whether the offscreen path must rebuild the selection overlay.
    pub fn offscreen_overlay_stale(&self) -> bool {
        self.consumed_offscreen_overlay != self.overlay
    }

    /// Mark the offscreen overlay as rebuilt.
    pub fn consume_offscreen_overlay(&mut self) {
        self.consumed_offscreen_overlay = self.overlay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_clean() {
        let state = RenderInvalidation::new();
        assert!(!state.redraw_pending());
        assert!(!state.live_scene_stale());
        assert!(!state.offscreen_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(!state.offscreen_overlay_stale());
    }

    #[test]
    fn camera_change_requests_only_a_repaint() {
        let mut state = RenderInvalidation::new();
        state.request_redraw();
        assert!(state.redraw_pending());
        assert!(!state.live_scene_stale());
        assert!(!state.offscreen_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(!state.offscreen_overlay_stale());
        state.consume_redraw();
        assert!(!state.redraw_pending());
    }

    #[test]
    fn overlay_tool_change_costs_a_repaint_like_a_camera_move() {
        let mut state = RenderInvalidation::new();
        state.overlay_tools_changed();
        assert!(state.redraw_pending());
        assert!(!state.live_scene_stale());
        assert!(!state.offscreen_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(!state.offscreen_overlay_stale());
    }

    #[test]
    fn selection_change_stales_only_both_overlays() {
        let mut state = RenderInvalidation::new();
        state.selection_changed();
        assert!(state.redraw_pending());
        assert!(state.live_overlay_stale());
        assert!(state.offscreen_overlay_stale());
        assert!(!state.live_scene_stale());
        assert!(!state.offscreen_scene_stale());
    }

    #[test]
    fn scene_change_stales_every_consumer_independently() {
        let mut state = RenderInvalidation::new();
        state.scene_geometry_changed();
        assert!(state.redraw_pending());
        assert!(state.live_scene_stale());
        assert!(state.offscreen_scene_stale());
        assert!(state.live_overlay_stale());
        assert!(state.offscreen_overlay_stale());
        // One path consuming must not hide the update from the other.
        state.consume_live_scene();
        state.consume_live_overlay();
        assert!(!state.live_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(state.offscreen_scene_stale());
        assert!(state.offscreen_overlay_stale());
    }

    #[test]
    fn sculpt_topology_change_spares_the_selection_overlay() {
        let mut state = RenderInvalidation::new();
        state.sculpt_topology_changed();
        assert!(state.redraw_pending());
        assert!(state.live_scene_stale());
        assert!(state.offscreen_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(!state.offscreen_overlay_stale());
    }

    #[test]
    fn suppressed_burst_frame_keeps_scene_generations_stale() {
        let mut state = RenderInvalidation::new();
        state.scene_geometry_changed();
        state.suppress_redraw();
        assert!(!state.redraw_pending());
        assert!(state.live_scene_stale());
        assert!(state.offscreen_scene_stale());
        assert!(state.live_overlay_stale());
        assert!(state.offscreen_overlay_stale());
    }

    #[test]
    fn reset_clears_every_consumer() {
        let mut state = RenderInvalidation::new();
        state.scene_geometry_changed();
        state.reset();
        assert!(!state.redraw_pending());
        assert!(!state.live_scene_stale());
        assert!(!state.offscreen_scene_stale());
        assert!(!state.live_overlay_stale());
        assert!(!state.offscreen_overlay_stale());
        // Generations keep advancing after a reset.
        state.selection_changed();
        assert!(state.live_overlay_stale());
        assert!(state.offscreen_overlay_stale());
        assert!(!state.live_scene_stale());
    }
}
