//! Scene lifecycle transitions.
//!
//! These entry points own the order in which document content, tool sessions,
//! camera framing, and render mirrors change. Document-only and render-only
//! bookkeeping lives with `DocumentState` / `RenderState`; this module keeps
//! the cross-owner ordering explicit.

use super::super::{Scene, SceneContext};

impl SceneContext<'_> {
    pub(in crate::app) fn set_scene(&mut self, scene: Scene, reset_camera: bool) {
        self.document.begin_scene_install();
        // The drag is ended here, and which form is decided by what happens to
        // the layer, not by where the code sits. `set_scene` is reached by two
        // different transitions:
        //
        // - Replace (and the scene-destroying paths): the layer and the pose
        //   both go. Recording would push a history step describing the
        //   outgoing scene, and the guard would not refuse it (it matches layer
        //   ids), so the first Ctrl+Z would show the edit undone and the second
        //   would put it back and rewind the pose. `forget_replaced_scene_state`
        //   already drops the gesture for this path before the scene is
        //   installed.
        // - Append: the layer survives into the combined scene, so the pose
        //   sitting on it is just as real there. Dropping it would leave a scan
        //   in a pose that no history step describes and no save prompt names.
        //   It must be committed.
        //
        // `abandon_align_drag` is the commit form and is the right default here:
        // on the Replace path the drag is already `None` (dropped by
        // `forget_replaced_scene_state`), so it is a no-op, and on the Append
        // path it records the move. Calling `discard` here instead would let an
        // append carry a moved pose forward with nothing recording it.
        self.abandon_align_drag();
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.document.sync_edit_mode_to_scene(&scene);
        // A structural scene swap (load, delete, another mesh edit, undo/redo)
        // reverts the geometry the persistent Sculpt session was prepared over,
        // without necessarily changing topology_id (a sculpt commit preserves
        // it), so drop the session here and re-prepare on the next stroke.
        self.tools.sculpt.invalidate_session();
        self.document.install_scene_content(scene);
        self.enrol_align_arrivals();
        self.clear_live_viewport();
        self.render.drop_prepared_scene();
        // Preserving a view requires an existing camera. The first mesh in a
        // new pane must be framed even when automatic framing is disabled.
        if reset_camera || self.render.camera.is_none() {
            self.reset_camera_to_home();
        }
        self.render.mark_scene_geometry_changed();
        self.document.clear_mesh_selection_drag();
        self.render.discard_rendered_frame();
        // Whatever the align tool was showing described the geometry that just
        // got replaced. Every structural path (undo, redo, repair, close holes,
        // crop, cut, separate, a bridge split commit, a cancelled mesh-edit
        // session) passes through here. Left up, the map would keep describing
        // the former surface, take the scan's tint, and the panel would report
        // a percentage for a surface that no longer exists.
        self.forget_align_fit(
            &self
                .ui
                .locale
                .tr(crate::i18n::message_id!("align-status-scan-changed")),
        );
        // Structural scene change: world anchors may now dangle over deleted or
        // replaced geometry, so measurements are cleared (the tool stays armed
        // while something remains to measure). Material-only updates keep them
        // (world space is unchanged).
        self.tools.measure.clear_measurements();
        if !self.has_measurable_layer() {
            self.tools.measure.disarm();
        }
        if self.can_render_cut_view() {
            // A planted disc holds a world-space plane. Scanner vendors place
            // models at wildly different origins, so a plane kept across a
            // scene replace usually leaves the new case entirely on the
            // clipped-away side, drawing as a faint ghost — which reads as "the
            // file loaded wrong". Re-arm instead: the tool stays on, the stale
            // placement does not. Bridge split does the same just above.
            if self.tools.cut_view.is_active() {
                self.tools.cut_view.enable();
            }
            self.tools.cut_view.mark_dirty();
        } else {
            self.tools.cut_view.disable();
        }
    }

    pub(in crate::app) fn update_scene_materials(&mut self, scene: Scene) {
        self.document.replace_scene_handle(scene);
        self.mark_scene_materials_changed();
    }

    /// The bookkeeping a material change needs, for a caller that already owns
    /// the live scene and mutated it in place.
    pub(in crate::app) fn mark_scene_materials_changed(&mut self) {
        if let Some(scene) = self.document.scene.clone() {
            self.document.sync_edit_mode_to_scene(&scene);
        }
        self.render.mark_scene_geometry_changed();
        self.document.clear_mesh_selection_drag();
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
        } else {
            self.tools.cut_view.disable();
        }
    }

    pub(in crate::app) fn clear_scene(&mut self) {
        // Overlay cleanup may edit the scene; detach this handle first.
        let scene = self.document.take_scene_for_clear();
        // The last layer can disappear while Align Meshes is armed. Revoke its
        // pose, overlay, mask, and worker generation before a new scene may
        // reuse one of the old layer ids.
        self.reset_align_state_for_scene_clear();
        drop(scene);
        // A clear has no replacement scene to validate against. Revoke the
        // persistent Sculpt worker before dropping the scene so a background
        // completion cannot outlive this generation and be mistaken for the
        // next file's layer.
        self.tools.sculpt.invalidate_session();
        self.document.clear_scene_content();
        self.clear_live_viewport();
        self.render.drop_prepared_scene();
        self.document.clear_scene_paths();
        self.render.clear_camera_and_frame();
        self.document.finish_scene_clear();
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.tools.cut_view.disable();
        self.tools.measure.disarm();
        self.render.clear_section_cache();
    }
}
