//! Viewport input for the sculpt brushes: the persistent per-layer kernel
//! session, the arc-length dab scheduler, sparse live vertex writes, the
//! re-upload-free stroke commit, wheel resize/re-intensify, and the brush
//! cursor. The geometry kernel lives in `occlu-mesh-edit`.

use super::{egui, mesh_editor_overlay, OccluViewApp};
use crate::sculpt_tool::{
    SculptToolKind, StrokeState, DAB_SPACING_FRACTION, HOLD_DAB_INTERVAL_SEC, MAX_DABS_PER_FRAME,
    SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN, SCULPT_SIZE_MAX, SCULPT_SIZE_MIN,
    SCULPT_WHEEL_STEP,
};
use crate::sculpt_worker::SculptWorker;
use crate::viewer::viewport_ray;
use glam::Vec3;
use occluview_core::{BrushMode, BrushStroke, SceneMeshId, ScenePickHit};

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
/// return the touched vertex ids. The spacing decision is the pure
/// [`plan_dab_centers`]; this only converts to local space and applies.
fn schedule_dabs(worker: &SculptWorker, stroke: &mut StrokeState, params: &DabParams) -> usize {
    let radius_local = (params.radius_world * worker.local_per_world).max(1e-4);
    let center = worker.world_to_local.transform_point3(params.hit_world);
    let view_local = worker
        .world_to_local
        .transform_vector3(params.view_world)
        .normalize_or_zero();
    let spacing = (radius_local * DAB_SPACING_FRACTION).max(1e-4);

    let (centers, last_dab, hold_seconds) = plan_dab_centers(
        stroke.last_dab_local,
        center,
        spacing,
        stroke.hold_seconds,
        params.dt,
    );
    stroke.last_dab_local = last_dab;
    stroke.hold_seconds = hold_seconds;

    let mut queued = 0;
    for at in centers {
        queued += usize::from(worker.try_apply(
            BrushStroke {
                center: at.to_array(),
                radius_mm: radius_local,
                strength: params.strength,
                view_dir: view_local.to_array(),
            },
            params.mode,
        ));
    }
    queued
}

/// Pure dab scheduler: given the previous dab, the cursor `center`, the
/// `spacing`, and the hold accumulator, returns this frame's dab centers and the
/// updated `(last_dab, hold_seconds)`. Dabs are spaced by arc length while
/// moving and by a time cadence while (near) stationary, at most
/// [`MAX_DABS_PER_FRAME`] per frame. If the cursor jumps farther than that
/// budget, the segment is sampled evenly and the scheduler advances all the
/// way to the current point; this keeps input latency bounded instead of
/// building an invisible backlog of expensive dabs.
#[allow(clippy::cast_precision_loss)]
fn plan_dab_centers(
    last_dab: Option<Vec3>,
    center: Vec3,
    spacing: f32,
    hold_seconds: f32,
    dt: f32,
) -> (Vec<Vec3>, Option<Vec3>, f32) {
    let Some(last) = last_dab else {
        return (vec![center], Some(center), 0.0);
    };
    let segment = center - last;
    let distance = segment.length();
    if distance >= spacing {
        if distance > spacing * MAX_DABS_PER_FRAME as f32 {
            let count = MAX_DABS_PER_FRAME as f32;
            let centers = (1..=MAX_DABS_PER_FRAME)
                .map(|step| last + segment * (step as f32 / count))
                .collect();
            return (centers, Some(center), 0.0);
        }
        let direction = segment / distance;
        let mut cursor = last;
        let mut walked = 0.0;
        let mut centers = Vec::new();
        while walked + spacing <= distance && centers.len() < MAX_DABS_PER_FRAME {
            cursor += direction * spacing;
            walked += spacing;
            centers.push(cursor);
        }
        (centers, Some(cursor), 0.0)
    } else {
        let mut hold = hold_seconds + dt.clamp(0.0, HOLD_DAB_INTERVAL_SEC * 4.0);
        let mut centers = Vec::new();
        while hold >= HOLD_DAB_INTERVAL_SEC && centers.len() < MAX_DABS_PER_FRAME {
            hold -= HOLD_DAB_INTERVAL_SEC;
            centers.push(center);
        }
        (centers, Some(last), hold)
    }
}

impl OccluViewApp {
    /// Arm/disarm a sculpt tool (toggling the armed one disarms).
    pub(super) fn toggle_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        self.abort_sculpt_stroke();
        self.sculpt.toggle(kind);
        if self.sculpt.armed.is_some() {
            // Arming a brush means the Sculpt tab: show it and drop selection.
            self.editor_tab = mesh_editor_overlay::EditorTab::Sculpt;
            self.mesh_selection_drag = None;
            // Prepare the target off the UI thread. Selection gesture state is
            // intentionally preserved; sculpt owns LMB while armed and must
            // not silently turn Lasso into Marquee.
            self.prepare_armed_sculpt_session();
        } else {
            self.sculpt.disarm();
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

    /// Switch the editor tab: Sculpt arms a brush, Edit Mesh drops it.
    pub(super) fn switch_editor_tab(
        &mut self,
        tab: mesh_editor_overlay::EditorTab,
        ctx: &egui::Context,
    ) {
        use mesh_editor_overlay::EditorTab;
        if self.editor_tab == tab {
            return;
        }
        self.editor_tab = tab;
        match tab {
            EditorTab::EditMesh => {
                self.abort_sculpt_stroke();
                self.sculpt.disarm();
            }
            EditorTab::Sculpt if self.sculpt.armed.is_none() => {
                self.toggle_sculpt_tool(SculptToolKind::AddRemove, ctx);
            }
            EditorTab::Sculpt => {}
        }
        self.needs_render = true;
        ctx.request_repaint();
    }

    /// Edit-Mesh-only sculpt hotkeys: `1` arms Add/Remove, `2` arms Smooth.
    /// Consumed only while a session is open and no text field has focus.
    pub(super) fn handle_sculpt_hotkeys(&mut self, ctx: &egui::Context) -> bool {
        if !self.edit_mode.has_active_session() || ctx.wants_keyboard_input() {
            return false;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)) {
            self.arm_sculpt_tool(SculptToolKind::AddRemove, ctx);
            return true;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)) {
            self.arm_sculpt_tool(SculptToolKind::Smooth, ctx);
            return true;
        }
        false
    }

    /// Arm a sculpt tool idempotently — the hotkey only turns a tool ON.
    fn arm_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        if self.sculpt.armed != Some(kind) {
            self.toggle_sculpt_tool(kind, ctx);
        }
    }

    /// One frame of the sculpt gesture. Returns `true` only while the PRIMARY
    /// button drives a sculpt this frame, so RMB orbit / MMB / wheel keep
    /// working with a brush armed.
    pub(super) fn handle_sculpt_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
    ) -> bool {
        self.poll_sculpt_preparation(ctx);
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
            self.sculpt.press_pending = false;
            return false;
        }

        let (pressed, down, pointer, shift) = ctx.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.interact_pos(),
                input.modifiers.shift,
            )
        });
        let dt = ctx.input(|input| input.stable_dt);

        // A fast release followed by a new press can be coalesced into one
        // egui frame. The edge is authoritative: finalize any stale previous
        // stroke before creating the next one, otherwise its old anchor and
        // hold timer can make the second drag look dead.
        if pressed && self.sculpt.stroke.is_some() {
            self.commit_sculpt_stroke(ctx);
            self.sculpt.press_pending = false;
        }

        if !down {
            if self.sculpt.stroke.is_some() {
                self.commit_sculpt_stroke(ctx);
                self.sculpt.press_pending = false;
                return true;
            }
            self.sculpt.press_pending = false;
            return false;
        }

        // Primary is held. Own the gesture; only lay dabs where there is a
        // surface under the cursor on the stroke's layer.
        let Some(pointer) = pointer else {
            return true;
        };
        if self.sculpt.stroke.is_none() && !response.contains_pointer() {
            self.sculpt.press_pending = false;
            return false;
        }
        let Some(hit) = self.sculpt_surface_hit(response.rect, pointer) else {
            // Keep owning this held gesture while the background BVH/brush
            // preparation finishes. The next frame retries the current point,
            // so the first press is never silently lost.
            self.sculpt.press_pending = true;
            ctx.request_repaint();
            return true;
        };
        self.sculpt.press_pending = false;
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
                    self.sculpt.press_pending = true;
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
        let Some(worker) = self.sculpt.worker.as_ref() else {
            return;
        };
        let queued = {
            let Some(stroke) = self.sculpt.stroke.as_mut() else {
                return;
            };
            schedule_dabs(worker, stroke, &params)
        };
        if queued > 0 {
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
        if entry.id() != hit.layer_id {
            return false;
        }
        if self
            .sculpt
            .session_matches(hit.layer_id, entry.mesh.topology_id())
        {
            return true;
        }
        let _ = self.sculpt.queue_preparation(scene, hit.layer_index);
        false
    }

    /// Prepare the active edit layer as soon as Edit Mesh/Sculpt becomes
    /// available. The one-time O(n) weld/adjacency/grid build stays off the UI
    /// thread and normally completes before the first brush press.
    pub(super) fn prepare_armed_sculpt_session(&mut self) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let target = self
            .edit_mode
            .session_layer_id()
            .and_then(|layer_id| {
                scene
                    .meshes()
                    .iter()
                    .position(|entry| entry.id() == layer_id)
            })
            .or_else(|| {
                let mut sculptable = scene.meshes().iter().enumerate().filter(|(_, entry)| {
                    entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
                });
                let first = sculptable.next().map(|(index, _)| index);
                first.filter(|_| sculptable.next().is_none())
            });
        if let Some(index) = target {
            let _ = self.sculpt.queue_preparation(scene, index);
        }
    }

    pub(super) fn poll_sculpt_preparation(&mut self, ctx: &egui::Context) {
        let Some(result) = self.sculpt.poll_preparation() else {
            return;
        };
        match result {
            Ok(session) => {
                let valid = self.scene.as_ref().is_some_and(|scene| {
                    scene.meshes().iter().any(|entry| {
                        entry.id() == session.layer_id
                            && entry.mesh.topology_id() == session.topology_id
                    })
                });
                if valid && self.edit_mode.has_active_session() {
                    self.sculpt.worker = Some(SculptWorker::spawn(session));
                    if self.sculpt.armed.is_some() {
                        self.status_message = None;
                    }
                    self.needs_render = true;
                    ctx.request_repaint();
                }
            }
            Err(error) => {
                self.status_message = Some(format!("Cannot sculpt this layer: {error}"));
                ctx.request_repaint();
            }
        }
    }

    /// Drop any in-flight stroke. If it had uncommitted dabs on the GPU, drop
    /// the persistent session too and force a full re-sync so the on-screen
    /// geometry reverts to the committed scene.
    pub(super) fn abort_sculpt_stroke(&mut self) {
        let had_stroke = self.sculpt.stroke.take().is_some();
        if had_stroke {
            self.invalidate_sculpt_session_silent();
        }
    }

    pub(super) fn invalidate_sculpt_session_silent(&mut self) {
        // Cancel any worker prepared from the pre-edit scene as well as the
        // live GPU shadow. Otherwise a stale background result could become
        // active after an undo, layer removal, or structural mesh edit.
        self.sculpt.invalidate_session();
        self.live_viewport_scene_dirty = true;
        self.offscreen_scene_dirty = true;
        self.needs_render = true;
    }

    /// Shift/Ctrl + wheel resizes / re-intensifies the brush instead of zooming.
    /// Returns `true` when it consumed the wheel so the caller skips the zoom.
    /// `over_viewport` gates it to the 3D view so a modified scroll over a panel
    /// (Layers, the mesh-editor window) keeps its normal meaning.
    pub(super) fn adjust_sculpt_brush_from_wheel(
        &mut self,
        ctx: &egui::Context,
        over_viewport: bool,
    ) -> bool {
        if !over_viewport || self.sculpt.armed.is_none() || !self.edit_mode.has_active_session() {
            return false;
        }
        let (scroll_x, scroll_y, shift, ctrl) = ctx.input(|input| {
            (
                input.raw_scroll_delta.x,
                input.raw_scroll_delta.y,
                input.modifiers.shift,
                input.modifiers.ctrl || input.modifiers.command,
            )
        });
        // Holding Shift makes many window managers deliver the wheel as
        // HORIZONTAL scroll, so read whichever axis actually moved — otherwise
        // Shift+wheel silently did nothing (only `.y` was read).
        let scroll = if scroll_y.abs() >= scroll_x.abs() {
            scroll_y
        } else {
            scroll_x
        };
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
        let layer_id = self.sculpt_target_layer_id(scene)?;
        let entry = scene.meshes().iter().find(|entry| entry.id() == layer_id)?;
        // The preparation worker warms this exact layer. Never allow a cold
        // scan-sized BVH to build on the egui thread, and do not wait on
        // unrelated visible layers.
        if !entry.mesh.bvh_is_ready() {
            return None;
        }
        let (origin, direction) = viewport_ray(&camera, viewport_rect, pointer)?;
        scene.pick_layer_ray_hit(origin, direction, layer_id)
    }

    fn sculpt_target_layer_id(&self, scene: &occluview_core::Scene) -> Option<SceneMeshId> {
        self.edit_mode
            .session_layer_id()
            .filter(|layer_id| {
                scene.meshes().iter().any(|entry| {
                    entry.id() == *layer_id
                        && entry.visible
                        && !entry.mesh.is_point_cloud()
                        && entry.mesh.triangle_count() > 0
                })
            })
            .or_else(|| {
                scene.meshes().iter().find_map(|entry| {
                    (entry.visible
                        && !entry.mesh.is_point_cloud()
                        && entry.mesh.triangle_count() > 0)
                        .then_some(entry.id())
                })
            })
    }

    /// The brush cursor is deliberately screen-space: a surface-projected ring
    /// required a second BVH pick plus 48 projected points and six filled glow
    /// polygons on every repaint. A quiet ring communicates brush size without
    /// competing with the model or introducing hover latency.
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
        let radius_world = mesh_editor_overlay::sculpt_radius_mm(ui.ctx());
        let intensity01 = mesh_editor_overlay::sculpt_intensity01(ui.ctx());
        let shift = ui.ctx().input(|input| input.modifiers.shift);
        let color = sculpt_cursor_color(kind, shift);

        let ortho_height = camera.orthographic_height.max(f32::EPSILON);
        let radius_px = radius_world * viewport_rect.height() / ortho_height;
        if radius_px.is_finite() && radius_px >= 2.0 {
            let canvas = ui.painter();
            let intensity = intensity01.clamp(0.0, 1.0);
            canvas.circle_filled(
                pointer,
                radius_px,
                color.gamma_multiply(0.025 + intensity * 0.035),
            );
            canvas.circle_stroke(
                pointer,
                radius_px,
                egui::Stroke::new(1.0, color.gamma_multiply(0.58 + intensity * 0.18)),
            );
            canvas.circle_stroke(
                pointer,
                (radius_px - 2.0).max(1.0),
                egui::Stroke::new(1.0, color.gamma_multiply(0.16)),
            );
            canvas.circle_filled(pointer, 1.5, color.gamma_multiply(0.62));
        }
    }
}

/// Quiet semantic colors: build, carve, and smooth remain distinguishable but
/// do not introduce the saturated blue accent used by the old editor chrome.
fn sculpt_cursor_color(kind: SculptToolKind, shift: bool) -> egui::Color32 {
    match (kind, shift) {
        (SculptToolKind::AddRemove, false) => egui::Color32::from_rgb(118, 151, 132),
        (SculptToolKind::AddRemove, true) => egui::Color32::from_rgb(164, 116, 108),
        (SculptToolKind::Smooth, false) => egui::Color32::from_rgb(142, 146, 154),
        (SculptToolKind::Smooth, true) => egui::Color32::from_rgb(172, 166, 151),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::cast_precision_loss)]
    use super::plan_dab_centers;
    use crate::sculpt_tool::{HOLD_DAB_INTERVAL_SEC, MAX_DABS_PER_FRAME};
    use glam::Vec3;

    #[test]
    fn first_dab_lands_at_the_cursor_and_arms_the_path() {
        let (centers, last, hold) =
            plan_dab_centers(None, Vec3::new(2.0, 0.0, 0.0), 1.0, 0.0, 0.016);
        assert_eq!(centers, vec![Vec3::new(2.0, 0.0, 0.0)]);
        assert_eq!(last, Some(Vec3::new(2.0, 0.0, 0.0)));
        assert_eq!(hold, 0.0);
    }

    #[test]
    fn a_straight_move_spaces_dabs_evenly_by_arc_length() {
        // Move exactly 3 spacings along +X: three dabs at 1,2,3, last at 3.
        let (centers, last, hold) =
            plan_dab_centers(Some(Vec3::ZERO), Vec3::new(3.0, 0.0, 0.0), 1.0, 0.0, 0.016);
        assert_eq!(
            centers,
            vec![
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(3.0, 0.0, 0.0),
            ]
        );
        assert_eq!(last, Some(Vec3::new(3.0, 0.0, 0.0)));
        assert_eq!(hold, 0.0);
    }

    #[test]
    fn a_huge_single_frame_jump_is_capped_without_backlog() {
        // A jump far beyond MAX_DABS_PER_FRAME spacings is sampled evenly, but
        // the anchor advances to the current cursor. The next frame must not
        // replay an invisible queue of old dabs.
        let far = (MAX_DABS_PER_FRAME + 50) as f32;
        let (centers, last, _) =
            plan_dab_centers(Some(Vec3::ZERO), Vec3::new(far, 0.0, 0.0), 1.0, 0.0, 0.016);
        assert_eq!(centers.len(), MAX_DABS_PER_FRAME);
        assert_eq!(last, Some(Vec3::new(far, 0.0, 0.0)));
        assert_eq!(centers.last(), Some(&Vec3::new(far, 0.0, 0.0)));
    }

    #[test]
    fn a_stationary_hold_fires_dabs_on_the_time_cadence() {
        // Cursor barely moves (< spacing): the hold accumulator fires a dab
        // every HOLD_DAB_INTERVAL_SEC, at the cursor, leaving `last` put.
        let last_dab = Some(Vec3::ZERO);
        let dt = HOLD_DAB_INTERVAL_SEC * 2.5;
        let (centers, last, hold) =
            plan_dab_centers(last_dab, Vec3::new(0.001, 0.0, 0.0), 1.0, 0.0, dt);
        assert_eq!(centers.len(), 2, "2.5 intervals of hold => 2 dabs");
        assert!(centers.iter().all(|c| *c == Vec3::new(0.001, 0.0, 0.0)));
        assert_eq!(
            last, last_dab,
            "a hold does not advance the arc-length anchor"
        );
        assert!(hold > 0.0 && hold < HOLD_DAB_INTERVAL_SEC);
    }

    #[test]
    fn a_stalled_frame_cannot_dump_a_huge_hold_backlog() {
        // A multi-second stall (dt) must be clamped so it doesn't fire dozens of
        // hold dabs at once when input resumes.
        let (centers, _, _) =
            plan_dab_centers(Some(Vec3::ZERO), Vec3::new(0.001, 0.0, 0.0), 1.0, 0.0, 5.0);
        assert!(
            centers.len() <= MAX_DABS_PER_FRAME,
            "hold backlog must stay bounded, got {}",
            centers.len()
        );
        assert!(
            centers.len() <= 5,
            "clamped dt should keep the backlog small"
        );
    }
}
