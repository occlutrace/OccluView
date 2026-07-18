//! Viewport glue for the interactive sculpt brushes (exocad Freeforming on
//! scans): an Add/Remove clay knife and a Smooth relaxer dragged directly on
//! the surface. This file owns the input side of it:
//!
//! - a persistent per-layer kernel session, prepared once and reused across
//!   strokes (no per-press O(n) prepare stall);
//! - an arc-length dab scheduler so buildup is even and framerate-independent
//!   (Blender's stroke spacing), with a time cadence for a held stationary
//!   brush;
//! - live feedback streamed as SPARSE vertex writes (only the touched vertices,
//!   not the whole buffer) so a drag never stutters;
//! - a commit that swaps the sculpted mesh in WITHOUT a full GPU re-upload (the
//!   buffers already hold the result and `topology_id` is preserved), landing
//!   each stroke as one undoable layer edit;
//! - Shift/Ctrl + wheel to resize / re-intensify the brush, and a brush cursor
//!   that shows the radius, intensity, and what the click will do.

use std::sync::Arc;

use super::{egui, mesh_editor_overlay, pick_scene_hit, EditModeCommand, OccluViewApp};
use crate::sculpt_tool::{
    mean_uniform_scale, SculptSession, SculptToolKind, StrokeState, DAB_SPACING_FRACTION,
    HOLD_DAB_INTERVAL_SEC, MAX_DABS_PER_FRAME, SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN,
    SCULPT_SIZE_MAX, SCULPT_SIZE_MIN, SCULPT_WHEEL_STEP,
};
use glam::Vec3;
use occluview_core::{
    mesh_edit_buffers_from_mesh, BrushMode, BrushSession, BrushStroke, Mesh, ScenePickHit,
};
use occluview_render::PreparedSceneTopology;

/// What the pointer/keyboard said this frame, resolved once so the dab loop
/// does not re-read input.
struct DabInput {
    kind: SculptToolKind,
    shift: bool,
    dt: f32,
}

/// A frame's dab request in WORLD space plus the resolved kernel mode/strength;
/// [`schedule_dabs`] converts to the layer's local space and spaces the dabs.
struct DabParams {
    hit_world: Vec3,
    view_world: Vec3,
    radius_world: f32,
    strength: f32,
    mode: BrushMode,
    dt: f32,
}

/// Lay this frame's dabs on `session`, updating `stroke`'s scheduler state, and
/// return the touched vertex ids. Dabs are spaced by arc length along the drag
/// (even, framerate-independent buildup) with a time cadence while stationary.
fn schedule_dabs(
    session: &mut SculptSession,
    stroke: &mut StrokeState,
    params: &DabParams,
) -> Vec<usize> {
    let radius_local = (params.radius_world * session.local_per_world).max(1e-4);
    let center = session.world_to_local.transform_point3(params.hit_world);
    let view_local = session
        .world_to_local
        .transform_vector3(params.view_world)
        .normalize_or_zero();
    let spacing = (radius_local * DAB_SPACING_FRACTION).max(1e-4);
    let dab = |at: Vec3| BrushStroke {
        center: at.to_array(),
        radius_mm: radius_local,
        strength: params.strength,
        view_dir: view_local.to_array(),
    };

    let mut touched: Vec<usize> = Vec::new();
    match stroke.last_dab_local {
        None => {
            touched.extend(session.apply_dab(dab(center), params.mode));
            stroke.last_dab_local = Some(center);
            stroke.hold_seconds = 0.0;
        }
        Some(last) => {
            let segment = center - last;
            let distance = segment.length();
            if distance >= spacing {
                let direction = segment / distance;
                let mut cursor = last;
                let mut walked = 0.0;
                let mut count = 0;
                while walked + spacing <= distance && count < MAX_DABS_PER_FRAME {
                    cursor += direction * spacing;
                    walked += spacing;
                    count += 1;
                    touched.extend(session.apply_dab(dab(cursor), params.mode));
                }
                stroke.last_dab_local = Some(cursor);
                stroke.hold_seconds = 0.0;
            } else {
                stroke.hold_seconds += params.dt.clamp(0.0, HOLD_DAB_INTERVAL_SEC * 4.0);
                let mut count = 0;
                while stroke.hold_seconds >= HOLD_DAB_INTERVAL_SEC && count < MAX_DABS_PER_FRAME {
                    stroke.hold_seconds -= HOLD_DAB_INTERVAL_SEC;
                    count += 1;
                    touched.extend(session.apply_dab(dab(center), params.mode));
                }
            }
        }
    }
    touched
}

impl OccluViewApp {
    /// Arm/disarm one sculpt tool from the Mesh Editor. Arming takes the primary
    /// gesture away from the selection tools; toggling the armed tool again
    /// disarms. Switching tools keeps the prepared session (same layer, so the
    /// next stroke stays instant).
    pub(super) fn toggle_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        self.abort_sculpt_stroke();
        self.sculpt.toggle(kind);
        if self.sculpt.armed.is_some() {
            let _ = self.edit_mode.set_lasso_armed(false);
            let _ = self.edit_mode.set_object_mode(false);
            self.mesh_selection_drag = None;
        }
        self.status_message = Some(match self.sculpt.armed {
            Some(SculptToolKind::AddRemove) => {
                "Add/Remove: drag to build, hold Shift to carve".to_string()
            }
            Some(SculptToolKind::Smooth) => {
                "Smooth: drag to relax, hold Shift to force it".to_string()
            }
            None => "Sculpt off".to_string(),
        });
        self.needs_render = true;
        ctx.request_repaint();
    }

    /// One frame of the sculpt gesture. Returns `true` only when the PRIMARY
    /// button is actively driving a sculpt this frame, so RMB orbit / MMB
    /// retarget / wheel zoom (none of which set the primary) keep working while
    /// a brush is armed.
    pub(super) fn handle_sculpt_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
    ) -> bool {
        if !self.edit_mode.has_active_session() {
            if self.sculpt.armed.is_some() || self.sculpt.stroke.is_some() {
                self.abort_sculpt_stroke();
                self.sculpt.disarm();
            }
            return false;
        }
        let Some(kind) = self.sculpt.armed else {
            return false;
        };
        if pan_drag_active {
            // LMB+RMB pan takes the primary away; end the drag cleanly.
            self.commit_sculpt_stroke(ctx);
            return false;
        }

        let (down, pointer, shift) = ctx.input(|input| {
            (
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.interact_pos(),
                input.modifiers.shift,
            )
        });
        let dt = ctx.input(|input| input.stable_dt);

        if !down {
            if self.sculpt.stroke.is_some() {
                self.commit_sculpt_stroke(ctx);
                return true;
            }
            return false;
        }

        // Primary is held. Own the gesture; only lay dabs where there is a
        // surface under the cursor on the stroke's layer.
        let Some(pointer) = pointer else {
            return true;
        };
        if self.sculpt.stroke.is_none() && !response.contains_pointer() {
            return false;
        }
        let Some(hit) = self.sculpt_surface_hit(response.rect, pointer) else {
            ctx.request_repaint();
            return true;
        };
        self.paint_sculpt_dabs(ctx, &hit, DabInput { kind, shift, dt });
        true
    }

    /// Lay the dabs this frame calls for and stream the touched vertices to the
    /// GPU. Starts (or continues) the persistent session and the stroke, then
    /// hands the actual spacing to [`schedule_dabs`].
    fn paint_sculpt_dabs(&mut self, ctx: &egui::Context, hit: &ScenePickHit, input: DabInput) {
        // Mid-stroke the session/stroke are locked to the stroke's own layer;
        // dabs that wander onto another arch are ignored, not committed there.
        match self.sculpt.stroke.as_ref().map(|stroke| stroke.layer_id) {
            Some(layer) if layer != hit.layer_id => {
                ctx.request_repaint();
                return;
            }
            Some(_) => {}
            None => {
                if !self.ensure_sculpt_session(hit) {
                    ctx.request_repaint();
                    return;
                }
                self.sculpt.stroke = Some(StrokeState {
                    layer_id: hit.layer_id,
                    last_dab_local: None,
                    hold_seconds: 0.0,
                });
            }
        }

        let params = DabParams {
            hit_world: hit.point,
            view_world: self
                .camera
                .as_ref()
                .map_or(Vec3::NEG_Z, |camera| camera.view_direction()),
            radius_world: mesh_editor_overlay::sculpt_radius_mm(ctx),
            strength: input
                .kind
                .dab_strength(mesh_editor_overlay::sculpt_intensity01(ctx), input.shift),
            mode: input.kind.brush_mode(input.shift),
            dt: input.dt,
        };
        let mut touched = {
            let (Some(session), Some(stroke)) =
                (self.sculpt.session.as_mut(), self.sculpt.stroke.as_mut())
            else {
                return;
            };
            schedule_dabs(session, stroke, &params)
        };
        if !touched.is_empty() {
            touched.sort_unstable();
            touched.dedup();
            self.flush_sculpt_vertices(&touched);
            self.needs_render = true;
        }
        ctx.request_repaint();
    }

    /// Ensure a valid prepared session covers the hit layer, preparing one (the
    /// one-time O(n) weld/adjacency/grid cost) only when the layer or its
    /// topology identity changed since the last stroke.
    fn ensure_sculpt_session(&mut self, hit: &ScenePickHit) -> bool {
        let Some(scene) = self.scene.clone() else {
            return false;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return false;
        };
        if entry.id() != hit.layer_id || !entry.visible || entry.mesh.is_point_cloud() {
            return false;
        }
        let topology_id = entry.mesh.topology_id();
        if self.sculpt.session_matches(hit.layer_id, topology_id) {
            return true;
        }
        let buffers = mesh_edit_buffers_from_mesh(&entry.mesh);
        let session = match BrushSession::prepare(&buffers) {
            Ok(session) => session,
            Err(error) => {
                self.status_message = Some(format!("Cannot sculpt this layer: {error}"));
                return false;
            }
        };
        let scale = mean_uniform_scale(&entry.transform);
        self.sculpt.session = Some(SculptSession {
            layer_id: entry.id(),
            topology_id,
            session,
            shadow: entry.mesh.vertices().to_vec(),
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            world_to_local: entry.transform.inverse(),
            local_per_world: 1.0 / scale,
            dirty_stroke: false,
        });
        true
    }

    /// Stream the `touched` sculpted vertices into whichever prepared scene is
    /// rendering (the wgpu live viewport, or the offscreen fallback). A failed
    /// write is harmless — the next full sync restores coherence.
    fn flush_sculpt_vertices(&mut self, touched: &[usize]) {
        let Some(session) = self.sculpt.session.as_ref() else {
            return;
        };
        if let Some(live_viewport) = self.live_viewport.as_ref() {
            if let Ok(viewport) = live_viewport.lock() {
                let _ = viewport.write_scene_vertices_sparse(
                    &session.topology,
                    &session.shadow,
                    touched,
                );
            }
        } else if let (Some(offscreen), Some(prepared)) =
            (self.offscreen.as_ref(), self.prepared_scene.as_ref())
        {
            let _ = prepared.write_entry_vertices_sparse(
                offscreen.renderer(),
                &session.topology,
                &session.shadow,
                touched,
            );
        }
    }

    /// Finish the drag: bake the accumulated dabs into the scene as ONE
    /// undoable layer edit, WITHOUT a full GPU re-upload (the buffers already
    /// hold the sculpted result and `topology_id` is preserved). The persistent
    /// session is kept for the next stroke.
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) {
        let Some(_stroke) = self.sculpt.stroke.take() else {
            return;
        };
        let (layer_id, dirty, shadow) = match self.sculpt.session.as_mut() {
            Some(session) => {
                let dirty = session.dirty_stroke;
                session.dirty_stroke = false;
                (session.layer_id, dirty, session.shadow.clone())
            }
            None => return,
        };
        if !dirty {
            return;
        }
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(index) = scene
            .meshes()
            .iter()
            .position(|entry| entry.id() == layer_id)
        else {
            self.invalidate_sculpt_session(ctx);
            return;
        };
        let entry = &scene.meshes()[index];
        let Some(sculpted) = entry.mesh.with_sculpted_vertices(shadow) else {
            self.invalidate_sculpt_session(ctx);
            return;
        };
        let Some(token) = self
            .edit_mode
            .begin_layer_edit(entry, EditModeCommand::Sculpt)
        else {
            self.status_message = Some("Layer edit already in progress".to_string());
            return;
        };
        drop(scene);
        if self.commit_sculpt_scene(layer_id, sculpted, ctx) {
            let _ = self.edit_mode.finish_layer_edit_success(token);
            self.mark_mesh_edits_unsaved(layer_id);
            self.status_message = Some("Sculpt applied (Ctrl+Z undoes)".to_string());
        } else {
            let _ = self
                .edit_mode
                .finish_layer_edit_error(token, "sculpt commit failed".to_string());
            self.invalidate_sculpt_session(ctx);
        }
    }

    /// Swap the sculpted mesh into the live scene in place. The prepared GPU
    /// scene is deliberately NOT torn down or re-uploaded: it already holds the
    /// sculpted vertices from the per-dab sparse writes, and the mesh keeps its
    /// `topology_id`, so the render sync's topology token still matches.
    fn commit_sculpt_scene(
        &mut self,
        layer_id: occluview_core::SceneMeshId,
        mesh: Mesh,
        ctx: &egui::Context,
    ) -> bool {
        let Some(mut scene_arc) = self.scene.take() else {
            return false;
        };
        {
            let scene = Arc::make_mut(&mut scene_arc);
            let Some(entry) = scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
            else {
                self.scene = Some(scene_arc);
                return false;
            };
            entry.mesh = mesh;
        }
        self.edit_mode.sync_to_scene(&scene_arc);
        self.scene_stats = Some(super::app_render::scene_stats(&scene_arc));
        self.scene = Some(scene_arc);
        self.needs_render = true;
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        }
        ctx.request_repaint();
        true
    }

    /// Drop any in-flight stroke. If it had uncommitted dabs on the GPU, drop
    /// the persistent session too and force a full re-sync so the on-screen
    /// geometry reverts to the committed scene.
    pub(super) fn abort_sculpt_stroke(&mut self) {
        let had_stroke = self.sculpt.stroke.take().is_some();
        let dirty = self
            .sculpt
            .session
            .as_ref()
            .is_some_and(|session| session.dirty_stroke);
        if let Some(session) = self.sculpt.session.as_mut() {
            session.dirty_stroke = false;
        }
        if had_stroke && dirty {
            self.invalidate_sculpt_session_silent();
        }
    }

    fn invalidate_sculpt_session(&mut self, ctx: &egui::Context) {
        self.invalidate_sculpt_session_silent();
        ctx.request_repaint();
    }

    fn invalidate_sculpt_session_silent(&mut self) {
        self.sculpt.session = None;
        self.sculpt.stroke = None;
        self.live_viewport_scene_dirty = true;
        self.offscreen_scene_dirty = true;
        self.needs_render = true;
    }

    /// Shift/Ctrl + wheel resizes / re-intensifies the brush instead of zooming.
    /// Returns `true` when it consumed the wheel so the caller skips the zoom.
    pub(super) fn adjust_sculpt_brush_from_wheel(&mut self, ctx: &egui::Context) -> bool {
        if self.sculpt.armed.is_none() || !self.edit_mode.has_active_session() {
            return false;
        }
        let (scroll, shift, ctrl) = ctx.input(|input| {
            (
                input.raw_scroll_delta.y,
                input.modifiers.shift,
                input.modifiers.ctrl || input.modifiers.command,
            )
        });
        if scroll.abs() < f32::EPSILON || !(shift || ctrl) {
            return false;
        }
        let delta = scroll.signum() * SCULPT_WHEEL_STEP;
        if shift {
            let next = (mesh_editor_overlay::sculpt_size(ctx) + delta)
                .clamp(SCULPT_SIZE_MIN, SCULPT_SIZE_MAX);
            mesh_editor_overlay::set_sculpt_size(ctx, next);
        } else {
            let next = (mesh_editor_overlay::sculpt_intensity(ctx) + delta)
                .clamp(SCULPT_INTENSITY_MIN, SCULPT_INTENSITY_MAX);
            mesh_editor_overlay::set_sculpt_intensity(ctx, next);
        }
        self.needs_render = true;
        ctx.request_repaint();
        true
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

    /// The screen-space brush cursor following the pointer while a tool is
    /// armed: an outer ring at the brush radius, a translucent core whose
    /// opacity reads the intensity, and a color that says what the click will
    /// do (green build / red carve / blue smooth, brighter when Shift forces).
    pub(super) fn paint_sculpt_cursor_impl(&self, ui: &egui::Ui, viewport_rect: egui::Rect) {
        let Some(kind) = self.sculpt.armed else {
            return;
        };
        if !self.edit_mode.has_active_session() {
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
        if !radius_px.is_finite() || radius_px < 2.0 {
            return;
        }
        let intensity01 = mesh_editor_overlay::sculpt_intensity01(ui.ctx());
        let shift = ui.ctx().input(|input| input.modifiers.shift);
        let color = sculpt_cursor_color(kind, shift);
        let brush = ui.painter();
        brush.circle_filled(
            pointer,
            radius_px,
            color.gamma_multiply(0.08 + 0.22 * intensity01.clamp(0.0, 1.0)),
        );
        brush.circle_stroke(pointer, radius_px, egui::Stroke::new(1.5, color));
        brush.circle_filled(pointer, 2.0, color);
    }
}

/// Brush cursor color by tool and modifier: green builds, red carves, blue
/// smooths (a brighter blue when Shift forces maximum smoothing).
fn sculpt_cursor_color(kind: SculptToolKind, shift: bool) -> egui::Color32 {
    match (kind, shift) {
        (SculptToolKind::AddRemove, false) => egui::Color32::from_rgb(72, 174, 122),
        (SculptToolKind::AddRemove, true) => egui::Color32::from_rgb(206, 84, 72),
        (SculptToolKind::Smooth, false) => egui::Color32::from_rgb(70, 132, 204),
        (SculptToolKind::Smooth, true) => egui::Color32::from_rgb(120, 176, 244),
    }
}
