//! Viewport glue for the interactive sculpt brushes (exocad Freeforming on
//! scans): owns the primary-button gesture while a brush is armed, feeds one
//! kernel stroke per input frame, streams the touched vertices straight into
//! the prepared GPU vertex buffer for live feedback, and commits the finished
//! drag through the same layer-edit path (undo snapshot, unsaved marking)
//! every other mesh operation uses. Pure state/math lives in
//! [`crate::sculpt_tool`]; the geometry kernel is `occlu_mesh_edit`.

use super::{
    egui, mesh_editor_overlay, pick_scene_hit, EditModeCommand, OccluViewApp, PreparedSceneTopology,
};
use crate::sculpt_tool::{frame_strength, mean_uniform_scale, SculptBrushKind, SculptStroke};
use occluview_core::{
    mesh_edit_buffers_from_mesh, mesh_from_edit_buffers_like, BrushSession, BrushStroke,
    ScenePickHit,
};

impl OccluViewApp {
    /// Arm/disarm one sculpt brush from the Mesh Editor window. Arming takes
    /// the primary gesture away from the selection tools (lasso, object,
    /// marquee), mirroring how those tools disarm each other.
    pub(super) fn toggle_sculpt_brush(&mut self, kind: SculptBrushKind, ctx: &egui::Context) {
        self.abort_sculpt_stroke();
        self.sculpt.toggle(kind);
        if self.sculpt.armed.is_some() {
            let _ = self.edit_mode.set_lasso_armed(false);
            let _ = self.edit_mode.set_object_mode(false);
            self.mesh_selection_drag = None;
        }
        self.status_message = Some(match self.sculpt.armed {
            Some(kind) => format!(
                "Sculpt {}: drag on the surface, release to apply",
                kind.label()
            ),
            None => "Sculpt off".to_string(),
        });
        self.needs_render = true;
        ctx.request_repaint();
    }

    /// One frame of the sculpt gesture. Returns `true` when sculpting owns
    /// the primary button this frame, so the selection gestures and the
    /// camera's primary interactions stay out of the way; RMB orbit, MMB
    /// retarget, and wheel zoom never set this and keep working.
    pub(super) fn handle_sculpt_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
    ) -> bool {
        if !self.edit_mode.has_active_session() {
            if self.sculpt.armed.is_some() || self.sculpt.active.is_some() {
                self.abort_sculpt_stroke();
                self.sculpt.disarm();
            }
            return false;
        }
        let Some(kind) = self.sculpt.armed else {
            return false;
        };
        if pan_drag_active {
            // LMB+RMB pan takes the primary away; a half-drawn stroke ends.
            self.commit_sculpt_stroke(ctx);
            return false;
        }

        let (primary_pressed, primary_down, pointer) = ctx.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.interact_pos(),
            )
        });

        if self.sculpt.active.is_some() && !primary_down {
            self.commit_sculpt_stroke(ctx);
            return true;
        }
        if primary_pressed && response.contains_pointer() && self.sculpt.active.is_none() {
            if let Some(pointer) = pointer {
                self.begin_sculpt_stroke(ctx, response.rect, pointer, kind);
            }
            return true;
        }
        if primary_down && self.sculpt.active.is_some() {
            if let Some(pointer) = pointer {
                self.apply_sculpt_frame(ctx, response.rect, pointer, kind);
            }
            return true;
        }
        if response.contains_pointer() {
            // Keep the brush ring tracking the live cursor, and swallow
            // primary clicks so the single-face pick never fires while armed.
            ctx.request_repaint();
            return primary_down || response.clicked_by(egui::PointerButton::Primary);
        }
        false
    }

    /// Start a stroke on the surface under the pointer: prepare the kernel
    /// session over that layer's mesh and land the first dab. A press beside
    /// the mesh arms nothing but still owns the primary gesture.
    fn begin_sculpt_stroke(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
        kind: SculptBrushKind,
    ) {
        let Some(hit) = self.sculpt_surface_hit(viewport_rect, pointer) else {
            return;
        };
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return;
        };
        if entry.id() != hit.layer_id || !entry.visible || entry.mesh.is_point_cloud() {
            return;
        }
        let buffers = mesh_edit_buffers_from_mesh(&entry.mesh);
        let session = match BrushSession::prepare(&buffers) {
            Ok(session) => session,
            Err(error) => {
                self.status_message = Some(format!("Cannot sculpt this layer: {error}"));
                return;
            }
        };
        let scale = mean_uniform_scale(&entry.transform);
        let stroke = SculptStroke {
            layer_id: entry.id(),
            session,
            shadow: entry.mesh.vertices().to_vec(),
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            world_to_local: entry.transform.inverse(),
            local_radius_mm: mesh_editor_overlay::sculpt_radius_mm(ctx) / scale,
            touched: false,
        };
        self.sculpt.active = Some(stroke);
        self.apply_sculpt_dab(ctx, hit.point, kind);
    }

    /// Continue a live stroke: dab wherever the pointer meets the SAME layer.
    /// Off-mesh or other-layer samples are skipped, not committed.
    fn apply_sculpt_frame(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
        kind: SculptBrushKind,
    ) {
        let Some(hit) = self.sculpt_surface_hit(viewport_rect, pointer) else {
            return;
        };
        let on_stroke_layer = self
            .sculpt
            .active
            .as_ref()
            .is_some_and(|stroke| stroke.layer_id == hit.layer_id);
        if on_stroke_layer {
            self.apply_sculpt_dab(ctx, hit.point, kind);
        }
    }

    /// Land one falloff dab at a world-space surface point and stream the
    /// touched vertices into the prepared GPU buffer for this frame.
    fn apply_sculpt_dab(
        &mut self,
        ctx: &egui::Context,
        world_point: glam::Vec3,
        kind: SculptBrushKind,
    ) {
        let strength_pct = mesh_editor_overlay::sculpt_strength_pct(ctx);
        let dt = ctx.input(|input| input.stable_dt);
        let Some(stroke) = self.sculpt.active.as_mut() else {
            return;
        };
        let local = stroke.world_to_local.transform_point3(world_point);
        let outcome = stroke.session.apply_stroke(
            BrushStroke {
                center: local.to_array(),
                radius_mm: stroke.local_radius_mm,
                strength: frame_strength(strength_pct, dt),
            },
            kind.brush_mode(),
        );
        if outcome.touched_vertices.is_empty() {
            return;
        }
        stroke.patch_shadow(&outcome.touched_vertices);
        stroke.touched = true;
        self.push_sculpt_vertices_to_gpu();
        ctx.request_repaint();
    }

    /// Write the live shadow vertex array into whichever prepared scene is
    /// rendering (the wgpu live viewport, or the offscreen fallback). If the
    /// prepared entry vanished (a mid-drag rebuild), the next commit's full
    /// sync restores coherence, so a failed write is not an error.
    fn push_sculpt_vertices_to_gpu(&mut self) {
        let Some(stroke) = self.sculpt.active.as_ref() else {
            return;
        };
        if let Some(live_viewport) = self.live_viewport.as_ref() {
            if let Ok(viewport) = live_viewport.lock() {
                let _ = viewport.write_scene_vertices(&stroke.topology, &stroke.shadow);
            }
        } else if let (Some(offscreen), Some(prepared)) =
            (self.offscreen.as_ref(), self.prepared_scene.as_ref())
        {
            let _ = prepared.write_entry_vertices(
                offscreen.renderer(),
                &stroke.topology,
                &stroke.shadow,
            );
        }
        self.needs_render = true;
    }

    /// Finish the drag: bake the session into the scene through the standard
    /// layer-edit path so the stroke lands as ONE undoable operation.
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) {
        let Some(stroke) = self.sculpt.active.take() else {
            return;
        };
        if !stroke.touched {
            return;
        }
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(index) = scene
            .meshes()
            .iter()
            .position(|entry| entry.id() == stroke.layer_id)
        else {
            self.revert_sculpt_preview(ctx);
            return;
        };
        let mut draft = scene.as_ref().clone();
        let entry = &mut draft.meshes_mut()[index];
        let Some(token) = self
            .edit_mode
            .begin_layer_edit(entry, EditModeCommand::Sculpt)
        else {
            self.status_message = Some("Layer edit already in progress".to_string());
            self.revert_sculpt_preview(ctx);
            return;
        };
        let result = stroke.session.finish();
        let moved = result.report.moved_vertices;
        match mesh_from_edit_buffers_like(&entry.mesh, result.mesh) {
            Ok(mesh) => {
                entry.mesh = mesh;
                let _ = self.edit_mode.finish_layer_edit_success(token);
                self.commit_structural_scene(Some(scene.as_ref()), draft, ctx);
                self.mark_mesh_edits_unsaved(stroke.layer_id);
                self.status_message = Some(format!("Sculpt: {moved} vertices (Ctrl+Z undoes)"));
            }
            Err(error) => {
                let _ = self
                    .edit_mode
                    .finish_layer_edit_error(token, error.to_string());
                self.status_message = Some(format!("Could not apply sculpt: {error}"));
                self.revert_sculpt_preview(ctx);
            }
        }
    }

    /// Drop any live stroke and restore the on-screen geometry from the
    /// scene's own (untouched) meshes.
    pub(super) fn abort_sculpt_stroke(&mut self) {
        let had_preview = self
            .sculpt
            .active
            .take()
            .is_some_and(|stroke| stroke.touched);
        if had_preview {
            self.live_viewport_scene_dirty = true;
            self.offscreen_scene_dirty = true;
            self.needs_render = true;
        }
    }

    fn revert_sculpt_preview(&mut self, ctx: &egui::Context) {
        self.live_viewport_scene_dirty = true;
        self.offscreen_scene_dirty = true;
        self.needs_render = true;
        ctx.request_repaint();
    }

    fn sculpt_surface_hit(
        &self,
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
    ) -> Option<ScenePickHit> {
        let camera = self.camera?;
        let scene = self.scene.as_ref()?;
        pick_scene_hit(&camera, viewport_rect, pointer, scene)
    }

    /// The screen-space brush ring following the cursor while a brush is
    /// armed: radius = the mm brush radius at the orthographic camera scale.
    pub(super) fn paint_sculpt_cursor_impl(&self, ui: &egui::Ui, viewport_rect: egui::Rect) {
        if self.sculpt.armed.is_none() || !self.edit_mode.has_active_session() {
            return;
        }
        let Some(camera) = self.camera.as_ref() else {
            return;
        };
        let Some(pointer) = ui.ctx().pointer_hover_pos() else {
            return;
        };
        if !viewport_rect.contains(pointer) {
            return;
        }
        let ortho_height = camera.orthographic_height.max(f32::EPSILON);
        let px_per_mm = viewport_rect.height() / ortho_height;
        let radius_px = mesh_editor_overlay::sculpt_radius_mm(ui.ctx()) * px_per_mm;
        if !radius_px.is_finite() || radius_px < 1.0 {
            return;
        }
        let color = egui::Color32::from_rgb(66, 117, 204);
        ui.painter()
            .circle_stroke(pointer, radius_px, egui::Stroke::new(1.5, color));
        ui.painter().circle_filled(pointer, 1.5, color);
    }
}
