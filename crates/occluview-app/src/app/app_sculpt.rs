//! Viewport input for the sculpt brushes: the persistent per-layer kernel
//! session, the arc-length dab scheduler, sparse live vertex writes, the
//! re-upload-free stroke commit, wheel resize/re-intensify, and the brush
//! cursor. The geometry kernel lives in `occlu-mesh-edit`.

use super::{egui, live_viewport, mesh_editor_overlay, OccluViewApp};
use crate::sculpt_tool::{
    uniform_scene_scale, SculptToolKind, StrokeState, DAB_SPACING_FRACTION, HOLD_DAB_INTERVAL_SEC,
    MAX_DABS_PER_FRAME, SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN, SCULPT_SIZE_MAX,
    SCULPT_SIZE_MIN, SCULPT_WHEEL_STEP,
};
use crate::sculpt_worker::SculptWorker;
use crate::viewer::viewport_ray;
use glam::{Mat4, Quat, Vec3};
use occluview_core::{BrushMode, BrushStroke, SceneMeshId, ScenePickHit};
use occluview_render::{
    sculpt_surface_light_intensity, sculpt_tool_length, PreparedSceneTopology, SculptBrushUniform,
    SculptToolShape, SculptToolUniform,
};
use std::sync::Arc;

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

pub(super) fn apply_sculpt_wheel_settings(ctx: &egui::Context) -> bool {
    let raw_scroll = super::app_input::raw_wheel_delta(ctx);
    let (shift, ctrl) = ctx.input(|input| {
        (
            input.modifiers.shift,
            input.modifiers.ctrl || input.modifiers.command,
        )
    });
    // Holding Shift makes many window managers deliver the wheel as
    // HORIZONTAL scroll, so read whichever axis actually moved — otherwise
    // Shift+wheel silently did nothing (only `.y` was read).
    let scroll = if raw_scroll.y.abs() >= raw_scroll.x.abs() {
        raw_scroll.y
    } else {
        raw_scroll.x
    };
    if scroll.abs() < f32::EPSILON || !(shift || ctrl) {
        return false;
    }
    let delta = scroll.signum() * SCULPT_WHEEL_STEP;
    if shift {
        let next =
            (mesh_editor_overlay::sculpt_size(ctx) + delta).clamp(SCULPT_SIZE_MIN, SCULPT_SIZE_MAX);
        mesh_editor_overlay::set_sculpt_size(ctx, next);
    } else {
        let next = (mesh_editor_overlay::sculpt_intensity(ctx) + delta)
            .clamp(SCULPT_INTENSITY_MIN, SCULPT_INTENSITY_MAX);
        mesh_editor_overlay::set_sculpt_intensity(ctx, next);
    }
    true
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
        // A brush-mode switch keeps the layer context, so a live stroke is
        // finished — not aborted: its dabs still become one undoable edit on
        // the shared worker queue (Finish runs before the new mode's first
        // dab), instead of dying in a cleared queue with no undo entry.
        // Context switches away from sculpt (tabs, lasso) still abort via
        // their own paths: the worker they drop cannot outlive the context.
        if !self.commit_sculpt_stroke(ctx) {
            return;
        }
        self.tools.sculpt.toggle(kind);
        if self.tools.sculpt.armed.is_some() {
            // Arming a brush means the Sculpt tab: show it and drop selection.
            self.tools.editor_tab = mesh_editor_overlay::EditorTab::Sculpt;
            self.document.mesh_selection_drag = None;
            // Prepare the target off the UI thread. Selection gesture state is
            // intentionally preserved; sculpt owns LMB while armed and must
            // not silently turn Lasso into Marquee.
            self.prepare_armed_sculpt_session();
        } else if !self.tools.sculpt.worker_has_pending_work() {
            // `commit_sculpt_stroke` may have just queued Finish. Dropping the
            // worker here would clear that command and lose the last stroke;
            // poll_sculpt_worker owns the completion before a later teardown.
            self.tools.sculpt.disarm();
        }
        self.ui.status_message = Some(match self.tools.sculpt.armed {
            Some(SculptToolKind::AddRemove) if self.tools.sculpt.worker.is_some() => {
                self.ui.locale.tr("sculpt-armed-addremove")
            }
            Some(SculptToolKind::Smooth) if self.tools.sculpt.worker.is_some() => {
                self.ui.locale.tr("sculpt-armed-smooth")
            }
            Some(_) => self.ui.locale.tr("sculpt-preparing"),
            None => self.ui.locale.tr("sculpt-off"),
        });
        self.render.invalidation.overlay_tools_changed();
        ctx.request_repaint();
    }

    /// Switch the editor tab: Sculpt arms a brush, Edit Mesh drops it.
    pub(super) fn switch_editor_tab(
        &mut self,
        tab: mesh_editor_overlay::EditorTab,
        ctx: &egui::Context,
    ) {
        use mesh_editor_overlay::EditorTab;
        if self.tools.editor_tab == tab {
            return;
        }
        self.tools.editor_tab = tab;
        match tab {
            EditorTab::EditMesh => {
                self.abort_sculpt_stroke();
                self.tools.sculpt.disarm();
            }
            EditorTab::Sculpt if self.tools.sculpt.armed.is_none() => {
                self.toggle_sculpt_tool(SculptToolKind::AddRemove, ctx);
            }
            EditorTab::Sculpt => {}
        }
        self.render.invalidation.overlay_tools_changed();
        ctx.request_repaint();
    }

    /// Mesh Editor-only sculpt hotkeys: `1` arms Add/Remove, `2` arms Smooth.
    /// Consumed only while the Sculpt tab owns the editor and no text field has
    /// focus. Edit Mesh keeps digit keys available for its own context.
    pub(super) fn handle_sculpt_hotkeys(&mut self, ctx: &egui::Context) -> bool {
        if self.tools.editor_tab != mesh_editor_overlay::EditorTab::Sculpt
            || !self.document.edit_mode.has_active_session()
            || ctx.egui_wants_keyboard_input()
        {
            return false;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                // A held Shift must not swallow the switch: otherwise the
                // operator believes Smooth is armed while AddRemove (+Shift
                // = Remove) still is, and the next dab carves.
                || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Num1)
        }) {
            self.arm_sculpt_tool(SculptToolKind::AddRemove, ctx);
            return true;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Num2)
        }) {
            self.arm_sculpt_tool(SculptToolKind::Smooth, ctx);
            return true;
        }
        false
    }

    /// Arm a sculpt tool idempotently — the hotkey only turns a tool ON.
    fn arm_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        if self.tools.sculpt.armed != Some(kind) {
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
        self.tools.sculpt.clear_cursor_hit();
        self.poll_sculpt_preparation(ctx);
        if !self.document.edit_mode.has_active_session() {
            if self.tools.sculpt.armed.is_some() || self.tools.sculpt.stroke.is_some() {
                self.abort_sculpt_stroke();
                self.tools.sculpt.disarm();
            }
            return false;
        }
        let Some(kind) = self.tools.sculpt.armed else {
            return false;
        };
        if pan_drag_active {
            // LMB+RMB pan takes the primary away; end the drag cleanly.
            if !self.commit_sculpt_stroke(ctx) {
                return true;
            }
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
        if pressed && self.tools.sculpt.stroke.is_some() && !self.commit_sculpt_stroke(ctx) {
            return true;
        }

        if !down {
            if self.tools.sculpt.stroke.is_some() {
                if !self.commit_sculpt_stroke(ctx) {
                    return true;
                }
                return true;
            }
            return false;
        }

        // Primary is held. Own the gesture; only lay dabs where there is a
        // surface under the cursor on the stroke's layer.
        let Some(pointer) = pointer else {
            return true;
        };
        if !response.contains_pointer() {
            // The viewport response becomes false both outside its rect and
            // when the Mesh Editor window is above it. A live drag may pause
            // and resume on re-entry, but it must never turn an out-of-window
            // pointer into a ray and sculpt an extrapolated surface.
            if self.tools.sculpt.stroke.is_some() {
                ctx.request_repaint();
                return true;
            }
            return false;
        }
        if self.tools.sculpt.stroke.is_none()
            && !self.ensure_sculpt_session_for_target()
            && self.tools.sculpt.worker.is_none()
        {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-preparing"));
            ctx.request_repaint();
        }
        let Some(hit) = self.sculpt_surface_hit(response.rect, pointer) else {
            // Keep owning this held gesture while the background BVH/brush
            // preparation finishes. The next frame retries the current point,
            // so the first press is never silently lost.
            ctx.request_repaint();
            return true;
        };
        self.tools
            .sculpt
            .set_cursor_hit([pointer.x, pointer.y], hit);
        self.paint_sculpt_dabs(ctx, &hit, DabInput { kind, shift, dt });
        true
    }

    /// Lay the dabs this frame calls for and stream the touched vertices to the
    /// GPU. Starts (or continues) the persistent session and the stroke, then
    /// hands the actual spacing to [`schedule_dabs`].
    fn paint_sculpt_dabs(&mut self, ctx: &egui::Context, hit: &ScenePickHit, input: DabInput) {
        // Mid-stroke the session/stroke are locked to the stroke's own layer;
        // dabs that wander onto another arch are ignored, not committed there.
        match self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .map(|stroke| stroke.layer_id)
        {
            Some(layer) if layer != hit.layer_id => {
                ctx.request_repaint();
                return;
            }
            Some(_) => {}
            None => {
                if !self.ensure_sculpt_session_for_hit(hit) {
                    ctx.request_repaint();
                    return;
                }
                self.tools.sculpt.stroke = Some(StrokeState {
                    layer_id: hit.layer_id,
                    last_dab_local: None,
                    hold_seconds: 0.0,
                });
            }
        }

        let params = DabParams {
            hit_world: hit.point,
            view_world: self
                .render
                .camera
                .as_ref()
                .map_or(Vec3::NEG_Z, |camera| camera.view_direction()),
            radius_world: input
                .kind
                .dab_radius_mm(mesh_editor_overlay::sculpt_radius_mm(ctx), input.shift),
            strength: input
                .kind
                .dab_strength(mesh_editor_overlay::sculpt_intensity01(ctx), input.shift),
            mode: input.kind.brush_mode(input.shift),
            dt: input.dt,
        };
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-preparing"));
            return;
        };
        let queued = {
            let Some(stroke) = self.tools.sculpt.stroke.as_mut() else {
                return;
            };
            schedule_dabs(worker, stroke, &params)
        };
        if queued > 0 {
            // Dab bytes reach the GPU through the worker poll's sparse writes;
            // only the repaint is owed here.
            self.render.invalidation.request_redraw();
        }
        ctx.request_repaint();
    }

    /// Prepare the target without requiring a successful BVH hit first.
    fn ensure_sculpt_session_for_target(&mut self) -> bool {
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };
        let Some((index, layer_id)) =
            sculpt_target(&scene, self.document.edit_mode.session_layer_id())
        else {
            return false;
        };
        self.ensure_sculpt_session_for_layer(&scene, index, layer_id)
    }

    fn ensure_sculpt_session_for_hit(&mut self, hit: &ScenePickHit) -> bool {
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return false;
        };
        if entry.id() != hit.layer_id {
            return false;
        }
        self.ensure_sculpt_session_for_layer(&scene, hit.layer_index, hit.layer_id)
    }

    fn ensure_sculpt_session_for_layer(
        &mut self,
        scene: &Arc<occluview_core::Scene>,
        index: usize,
        layer_id: SceneMeshId,
    ) -> bool {
        let Some(entry) = scene.meshes().get(index) else {
            return false;
        };
        if entry.id() != layer_id {
            return false;
        }
        if uniform_scene_scale(&entry.transform).is_none() {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-nonuniform-scale"));
            return false;
        }
        if self
            .tools
            .sculpt
            .session_matches(layer_id, entry.mesh.topology_id())
        {
            return true;
        }
        self.tools
            .sculpt
            .queue_preparation(Arc::clone(scene), index)
    }

    /// Prepare the active edit layer as soon as Edit Mesh/Sculpt becomes
    /// available. The one-time O(n) weld/adjacency/grid build stays off the UI
    /// thread and normally completes before the first brush press.
    pub(super) fn prepare_armed_sculpt_session(&mut self) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let target = self
            .document
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
            if self
                .tools
                .sculpt
                .queue_preparation(Arc::clone(&scene), index)
            {
                self.ui.status_message = None;
            } else if scene
                .meshes()
                .get(index)
                .is_some_and(|entry| uniform_scene_scale(&entry.transform).is_none())
            {
                self.ui.status_message = Some(self.ui.locale.tr("sculpt-nonuniform-scale"));
            } else {
                self.ui.status_message = Some(self.ui.locale.tr("sculpt-preparing"));
            }
        }
    }

    pub(super) fn poll_sculpt_preparation(&mut self, ctx: &egui::Context) {
        let Some(result) = self.tools.sculpt.poll_preparation() else {
            return;
        };
        match result {
            Ok(session) => {
                let valid = self.document.scene.as_ref().is_some_and(|scene| {
                    scene.meshes().iter().any(|entry| {
                        entry.id() == session.layer_id
                            && entry.mesh.topology_id() == session.topology_id
                    })
                });
                if valid && self.document.edit_mode.has_active_session() {
                    self.tools.sculpt.worker = Some(SculptWorker::spawn(session));
                    if self.tools.sculpt.armed.is_some() {
                        self.ui.status_message = None;
                    }
                    self.render.invalidation.overlay_tools_changed();
                    ctx.request_repaint();
                }
            }
            Err(error) => {
                self.ui.status_message = Some(
                    self.ui
                        .locale
                        .tr_with("sculpt-failed", &[("detail", error.as_str())]),
                );
                ctx.request_repaint();
            }
        }
    }

    /// Drop any in-flight stroke. If it had uncommitted dabs on the GPU, drop
    /// the persistent session too and force a full re-sync so the on-screen
    /// geometry reverts to the committed scene.
    pub(super) fn abort_sculpt_stroke(&mut self) {
        let had_stroke = self.tools.sculpt.stroke.take().is_some();
        let had_pending = self.tools.sculpt.worker_has_pending_work();
        if had_stroke || had_pending {
            self.invalidate_sculpt_session_silent();
        }
    }

    pub(super) fn invalidate_sculpt_session_silent(&mut self) {
        // Cancel any worker prepared from the pre-edit scene as well as the
        // live GPU shadow. Otherwise a stale background result could become
        // active after an undo, layer removal, or structural mesh edit.
        self.tools.sculpt.invalidate_session();
        self.render.invalidation.sculpt_topology_changed();
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
        if !over_viewport
            || self.tools.sculpt.armed.is_none()
            || !self.document.edit_mode.has_active_session()
        {
            return false;
        }
        if !apply_sculpt_wheel_settings(ctx) {
            return false;
        }
        self.render.invalidation.overlay_tools_changed();
        ctx.request_repaint();
        true
    }

    fn sculpt_surface_hit(
        &self,
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
    ) -> Option<ScenePickHit> {
        let camera = self.render.camera?;
        let scene = self.document.scene.as_ref()?;
        let layer_id = self.sculpt_target_layer_id(scene)?;
        let entry = scene.meshes().iter().find(|entry| entry.id() == layer_id)?;
        let worker = self.tools.sculpt.worker.as_ref()?;
        if worker.layer_id != layer_id || worker.topology_id != entry.mesh.topology_id() {
            return None;
        }
        let (origin, direction) = viewport_ray(&camera, viewport_rect, pointer)?;
        let inverse = entry.transform.inverse();
        let (triangle_index, local_point) = worker.pick_local_ray(
            inverse.transform_point3(origin),
            inverse.transform_vector3(direction),
        )?;
        let point = entry.transform.transform_point3(local_point);
        let distance = (point - origin).dot(direction.normalize_or_zero());
        distance.is_finite().then_some(ScenePickHit {
            layer_index: scene
                .meshes()
                .iter()
                .position(|candidate| candidate.id() == layer_id)?,
            layer_id,
            triangle_index,
            point,
            distance,
        })
    }

    fn sculpt_target_layer_id(&self, scene: &occluview_core::Scene) -> Option<SceneMeshId> {
        sculpt_target(scene, self.document.edit_mode.session_layer_id())
            .map(|(_, layer_id)| layer_id)
    }

    /// Paint the cursor after viewport input has had a chance to cache its
    /// authoritative hit. Hovering performs one guarded BVH pick; a held drag
    /// reuses the exact hit that scheduled the dabs, so the surface light never
    /// adds a second scan-sized traversal to the hot path.
    pub(super) fn paint_sculpt_cursor_impl(
        &self,
        ui: &egui::Ui,
        viewport_response: &egui::Response,
    ) {
        let Some(kind) = self.tools.sculpt.armed else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if !self.document.edit_mode.has_active_session() {
            self.publish_sculpt_cursor(None);
            return;
        }
        // The cursor must follow the same ownership boundary as the drag: a
        // foreground editor window owns the pointer even when it sits inside
        // the viewport rectangle, and a preparing worker is not ready to
        // accept a dab yet.
        if !viewport_response.contains_pointer() || self.tools.sculpt.worker.is_none() {
            self.publish_sculpt_cursor(None);
            return;
        }
        let viewport_rect = viewport_response.rect;
        let Some(camera) = self.render.camera.as_ref() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let Some(pointer) = ui.ctx().pointer_hover_pos() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if !viewport_rect.contains(pointer) {
            self.publish_sculpt_cursor(None);
            return;
        }
        let pointer_key = [pointer.x, pointer.y];
        let hit = self
            .tools
            .sculpt
            .cursor_hit_for(pointer_key)
            .or_else(|| self.sculpt_surface_hit(viewport_rect, pointer));
        let Some(hit) = hit else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let Some(scene) = self.document.scene.as_ref() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if entry.id() != hit.layer_id {
            self.publish_sculpt_cursor(None);
            return;
        }
        let live_normal = self
            .tools
            .sculpt
            .worker
            .as_ref()
            .filter(|worker| {
                worker.layer_id == hit.layer_id && worker.topology_id == entry.mesh.topology_id()
            })
            .and_then(|worker| worker.local_triangle_normal(hit.triangle_index));
        let Some(normal) = sculpt_face_normal(scene, &hit, camera, live_normal) else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let shift = ui.ctx().input(|input| input.modifiers.shift);
        // The ring shows the footprint a dab would actually cover, so the
        // Shift-widened Smooth reads on screen before the first stroke lands.
        let radius_world =
            kind.dab_radius_mm(mesh_editor_overlay::sculpt_radius_mm(ui.ctx()), shift);
        let intensity01 = mesh_editor_overlay::sculpt_intensity01(ui.ctx());
        let color = sculpt_cursor_color(kind, shift);
        let strength = kind.dab_strength(intensity01, shift);
        let shape = match kind {
            SculptToolKind::AddRemove => SculptToolShape::Cone,
            SculptToolKind::Smooth => SculptToolShape::Cylinder,
        };
        let color_rgba = color.to_array().map(|channel| f32::from(channel) / 255.0);
        let tool_length = sculpt_tool_length(strength);
        let tool_rotation = Quat::from_rotation_arc(Vec3::Z, normal);
        let tool_model = Mat4::from_scale_rotation_translation(
            Vec3::new(radius_world, radius_world, tool_length),
            tool_rotation,
            hit.point + normal * 0.02,
        );
        self.publish_sculpt_cursor(Some(live_viewport::SculptCursor {
            target_index: hit.layer_index,
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            brush: SculptBrushUniform {
                center: hit.point.to_array(),
                radius: radius_world,
                normal: normal.to_array(),
                intensity: sculpt_surface_light_intensity(strength),
                color: color_rgba,
                tip: shape as u32,
                visible: 1,
                padding: [0; 2],
            },
            tool: SculptToolUniform {
                model: tool_model.to_cols_array(),
                color: color_rgba,
                opacity: 0.20 + 0.12 * strength,
                shape: shape as u32,
                visible: 1,
                padding: 0,
            },
        }));

        let ortho_height = camera.orthographic_height.max(f32::EPSILON);
        let radius_px = radius_world * viewport_rect.height() / ortho_height;
        if radius_px.is_finite() && radius_px >= 2.0 {
            let canvas = ui.painter();
            // The ring must preview the force the dab will actually use:
            // Shift+Smooth is a full-strength pass, not a dim 50% cursor.
            let intensity = strength;
            canvas.circle_filled(
                pointer,
                radius_px,
                color.gamma_multiply(0.025 + intensity * 0.035),
            );
            canvas.circle_stroke(
                pointer,
                radius_px,
                egui::Stroke::new(1.0_f32, color.gamma_multiply(0.58 + intensity * 0.18)),
            );
            canvas.circle_stroke(
                pointer,
                (radius_px - 2.0).max(1.0),
                egui::Stroke::new(1.0_f32, color.gamma_multiply(0.16)),
            );
            canvas.circle_filled(pointer, 1.5, color.gamma_multiply(0.62));
        }
    }

    fn publish_sculpt_cursor(&self, cursor: Option<live_viewport::SculptCursor>) {
        let Some(viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        if let Ok(mut viewport) = viewport.lock() {
            viewport.set_sculpt_cursor(cursor);
        }
    }
}

fn sculpt_face_normal(
    scene: &occluview_core::Scene,
    hit: &ScenePickHit,
    camera: &occluview_core::Camera,
    live_local_normal: Option<Vec3>,
) -> Option<Vec3> {
    let entry = scene.meshes().get(hit.layer_index)?;
    if entry.id() != hit.layer_id {
        return None;
    }
    let local = live_local_normal.unwrap_or_else(|| {
        let base = hit.triangle_index.saturating_mul(3);
        let Some(indices) = entry.mesh.indices().get(base..base.saturating_add(3)) else {
            return Vec3::ZERO;
        };
        let vertex = |index: u32| {
            entry
                .mesh
                .vertices()
                .get(usize::try_from(index).ok()?)
                .map(|vertex| Vec3::from_array(vertex.position))
        };
        let (Some(a), Some(b), Some(c)) =
            (vertex(indices[0]), vertex(indices[1]), vertex(indices[2]))
        else {
            return Vec3::ZERO;
        };
        (b - a).cross(c - a).normalize_or_zero()
    });
    if !local.is_finite() || local.length_squared() <= f32::EPSILON {
        return None;
    }
    let determinant = entry.transform.matrix3.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let normal = entry
        .transform
        .matrix3
        .inverse()
        .transpose()
        .mul_vec3(local)
        .normalize_or_zero();
    if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
        return None;
    }
    let toward_camera = (camera.eye() - hit.point).normalize_or_zero();
    if toward_camera.length_squared() > f32::EPSILON && normal.dot(toward_camera) < 0.0 {
        Some(-normal)
    } else {
        Some(normal)
    }
}

fn sculpt_target(
    scene: &occluview_core::Scene,
    preferred: Option<SceneMeshId>,
) -> Option<(usize, SceneMeshId)> {
    let valid = |entry: &occluview_core::SceneMesh| {
        entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
    };
    preferred
        .and_then(|layer_id| {
            scene
                .meshes()
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.id() == layer_id && valid(entry))
                .map(|(index, _)| (index, layer_id))
        })
        .or_else(|| {
            scene
                .meshes()
                .iter()
                .enumerate()
                .find(|(_, entry)| valid(entry))
                .map(|(index, entry)| (index, entry.id()))
        })
}

/// Quiet semantic colors: build, carve, and smooth remain distinguishable but
/// do not introduce the saturated blue accent used by the old editor chrome.
fn sculpt_cursor_color(kind: SculptToolKind, shift: bool) -> egui::Color32 {
    match (kind, shift) {
        (SculptToolKind::AddRemove, false) => egui::Color32::from_rgb(255, 145, 58),
        (SculptToolKind::AddRemove, true) => egui::Color32::from_rgb(74, 177, 255),
        (SculptToolKind::Smooth, _) => egui::Color32::from_rgb(178, 126, 255),
    }
}

#[cfg(test)]
#[path = "app_sculpt_tests.rs"]
mod tests;
