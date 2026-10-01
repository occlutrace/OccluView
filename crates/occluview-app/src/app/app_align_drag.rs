//! Moving a scan by hand while Align Scans is armed.
//!
//! `app_align` routes clicks and jobs; this module owns one continuous pointer
//! gesture and the single history entry it becomes.

use eframe::egui;
use glam::{Affine3A, Vec3};
use occluview_core::SceneMeshId;

use super::app_align::layer_of;
use super::SceneContext;
use crate::edit_mode::EditModeCommand;
use crate::i18n::message_id;
use crate::viewer::pick_scene_hit;

/// One pointer frame's inputs, carried together into the step builder.
#[derive(Clone, Copy)]
struct DragFrame {
    /// The viewport the pointer moved in, for the millimetres-per-pixel scale.
    viewport: egui::Rect,
    /// Pointer motion this frame, in pixels.
    motion: egui::Vec2,
    /// Whether Ctrl is held, which makes the frame a rotation.
    rotating: bool,
}

/// Pointer input delivered by egui during one frame.
struct AlignDragInput {
    events: Vec<egui::Event>,
    primary_down: bool,
    pointer_pos: Option<egui::Pos2>,
    pointer_delta: egui::Vec2,
    input_modifiers: egui::Modifiers,
}

/// State accumulated while replaying one frame's ordered input events.
struct DragReplayState {
    had_drag: bool,
    modifiers: egui::Modifiers,
    fallback_previous: Option<egui::Pos2>,
    owned: bool,
    saw_pointer_move: bool,
}

impl DragReplayState {
    fn new(
        had_drag: bool,
        last_modifiers: Option<egui::Modifiers>,
        input: &AlignDragInput,
    ) -> Self {
        Self {
            had_drag,
            modifiers: if had_drag {
                last_modifiers.unwrap_or(input.input_modifiers)
            } else {
                input.input_modifiers
            },
            fallback_previous: input
                .pointer_pos
                .map(|position| position - input.pointer_delta),
            owned: had_drag,
            saw_pointer_move: false,
        }
    }
}

/// An open hand drag.
#[derive(Clone, Copy)]
pub(crate) struct AlignDrag {
    /// The layer the operator grabbed.
    pub(super) layer: SceneMeshId,
    /// Its pose when the gesture began, so the whole drag is one undo step.
    pub(super) start: Affine3A,
    /// The surface point the operator grabbed, in the layer's own local frame.
    /// Each Ctrl-drag step maps it through the current pose and turns around
    /// that world point, so an earlier plain drag carries the anchor with it.
    pub(super) pivot_local: Vec3,
}

impl SceneContext<'_> {
    /// Commit an open drag before a scene transition that keeps its layer.
    pub(super) fn abandon_align_drag(&mut self) {
        self.finish_align_drag();
    }

    /// Discard a drag when its scene is replaced or cleared.
    pub(super) fn discard_align_drag(&mut self) {
        self.tools.align.drag = None;
        self.tools.align.drag_last_pointer_pos = None;
        self.tools.align.drag_modifiers = None;
        self.tools.align.drag_pose_changed = false;
        self.document.unsaved_drag_pose = false;
    }

    /// Begin, continue, or finish a hand drag. Returns whether the drag owns
    /// this frame's pointer.
    ///
    /// Whatever layer the operator grabs is the one that moves — the fixed
    /// scan included. This tool has no locked roles, and the map simply
    /// recomputes afterwards.
    pub(super) fn handle_align_drag(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        // Dragging a scan lives in the Manually tab, the way lab software
        // splits it. In the Automatically tab a press is always a landmark:
        // egui promotes a press to a drag after six pixels or eight tenths of a
        // second, so without this gate a careful click on a cusp would move the
        // scan instead of placing a point.
        if self.tools.align.tab != crate::align_panel::AlignTab::Manually {
            return self.finish_align_drag();
        }
        let mut input = ctx.input(|input| AlignDragInput {
            events: input.events.clone(),
            primary_down: input.pointer.button_down(egui::PointerButton::Primary),
            pointer_pos: input.pointer.hover_pos(),
            pointer_delta: input.pointer.delta(),
            input_modifiers: input.modifiers,
        });
        let had_drag = self.tools.align.drag.is_some();
        let mut replay = DragReplayState::new(had_drag, self.tools.align.drag_modifiers, &input);

        // Replay the event stream: a complete gesture can arrive in one frame,
        // even though egui's final button state already says the pointer is up.
        self.replay_align_drag_events(
            response,
            ctx,
            std::mem::take(&mut input.events),
            &mut replay,
        );
        self.apply_aggregate_drag_motion(response, ctx, &input, &mut replay);

        // Some backends change button state without preserving an explicit
        // release event. Still close a live gesture in that case.
        if self.tools.align.drag.is_some() && !input.primary_down {
            self.finish_align_drag();
            replay.owned = true;
        }
        replay.owned
    }

    /// Replay ordered egui events so modifier changes and coalesced gestures
    /// retain their original sequence.
    fn replay_align_drag_events(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        replay: &mut DragReplayState,
    ) {
        for event in events {
            self.replay_align_drag_event(response, ctx, event, replay);
        }
    }

    /// Apply one event from the frame's pointer stream.
    fn replay_align_drag_event(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        event: egui::Event,
        replay: &mut DragReplayState,
    ) {
        match event {
            egui::Event::ModifiersChanged(next) => {
                replay.modifiers = next;
                if self.tools.align.drag.is_some() {
                    self.tools.align.drag_modifiers = Some(next);
                }
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            } => {
                replay.modifiers = modifiers;
                if self.tools.align.drag.is_none() && self.viewport_press_owned(ctx, response, pos)
                {
                    if self.begin_align_drag_at(response, pos) {
                        self.tools.align.drag_last_pointer_pos = Some(pos);
                        self.tools.align.drag_modifiers = Some(modifiers);
                        replay.owned = true;
                    } else {
                        // Empty-space drags remain camera input.
                        self.tools.align.drag_last_pointer_pos = None;
                        self.tools.align.drag_modifiers = None;
                    }
                }
            }
            egui::Event::PointerMoved(pos) if self.tools.align.drag.is_some() => {
                replay.saw_pointer_move = true;
                let previous = self.tools.align.drag_last_pointer_pos.or_else(|| {
                    replay
                        .had_drag
                        .then_some(replay.fallback_previous)
                        .flatten()
                });
                if let Some(previous) = previous {
                    self.apply_align_drag_motion(
                        response,
                        ctx,
                        pos - previous,
                        replay.modifiers.ctrl || replay.modifiers.command,
                    );
                }
                self.tools.align.drag_last_pointer_pos = Some(pos);
                replay.owned = true;
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            } => {
                replay.modifiers = modifiers;
                self.tools.align.drag_modifiers = Some(modifiers);
                if self.tools.align.drag.is_some() {
                    if let Some(previous) = self.tools.align.drag_last_pointer_pos {
                        self.apply_align_drag_motion(
                            response,
                            ctx,
                            pos - previous,
                            modifiers.ctrl || modifiers.command,
                        );
                    }
                    self.finish_align_drag();
                    replay.owned = true;
                }
                self.tools.align.drag_last_pointer_pos = None;
                self.tools.align.drag_modifiers = None;
            }
            egui::Event::PointerButton { modifiers, .. } => {
                replay.modifiers = modifiers;
                if self.tools.align.drag.is_some() {
                    self.tools.align.drag_modifiers = Some(modifiers);
                }
            }
            _ => {}
        }
    }

    /// Use egui's aggregate pointer delta only when this backend omitted move
    /// events, avoiding a second application when the raw stream was complete.
    fn apply_aggregate_drag_motion(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        input: &AlignDragInput,
        replay: &mut DragReplayState,
    ) {
        if self.tools.align.drag.is_none()
            || !replay.had_drag
            || replay.saw_pointer_move
            || input.pointer_delta.length_sq() <= f32::EPSILON
        {
            return;
        }
        if let (Some(previous), Some(current)) = (
            self.tools
                .align
                .drag_last_pointer_pos
                .or(replay.fallback_previous),
            input.pointer_pos,
        ) {
            replay.owned |= self.apply_align_drag_motion(
                response,
                ctx,
                current - previous,
                replay.modifiers.ctrl || replay.modifiers.command,
            );
            self.tools.align.drag_last_pointer_pos = Some(current);
        }
    }

    /// Ray-pick the surface under the primary press and open its drag.
    fn begin_align_drag_at(&mut self, response: &egui::Response, grab: egui::Pos2) -> bool {
        let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) else {
            return false;
        };
        // Prefer the selected moving scan where arches overlap. If it has no
        // hit, allow the general scene pick so the other scan remains
        // available for deliberate movement.
        let hit = self
            .tools
            .align
            .tool
            .moving_layer()
            .and_then(|moving| {
                crate::viewer::pick_layer_hit(&camera, response.rect, grab, &scene, moving)
            })
            .or_else(|| pick_scene_hit(&camera, response.rect, grab, &scene));
        let Some(hit) = hit else {
            return false;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return false;
        };
        // Keep the clicked surface point in layer-local coordinates. Each Ctrl
        // step resolves it through the live pose, so an earlier plain
        // translation carries the anchor with it.
        //
        // The inverse is guarded, and the guard has to be a magnitude check,
        // not `is_finite` alone. A nearly singular pose can invert to finite
        // matrix entries around 1e30 and throw the pivot off the scene.
        let inverse = entry.transform.inverse();
        let mapped = if inverse.is_finite() {
            inverse.transform_point3(hit.point)
        } else {
            entry.mesh.bbox_cached().center()
        };
        self.tools.align.drag = Some(AlignDrag {
            layer: hit.layer_id,
            start: entry.transform,
            pivot_local: mapped,
        });
        self.tools.align.drag_pose_changed = false;
        true
    }

    /// Apply one pointer segment while keeping the open gesture's history
    /// boundary intact.
    fn apply_align_drag_motion(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
        motion: egui::Vec2,
        rotating: bool,
    ) -> bool {
        let Some(drag) = self.tools.align.drag else {
            return false;
        };
        if motion.length_sq() <= f32::EPSILON {
            return true;
        }
        let Some(camera) = self.render.camera else {
            return true;
        };
        let frame = DragFrame {
            viewport: response.rect,
            motion,
            rotating,
        };
        let Some(step) = self.align_drag_step(drag, &camera, frame) else {
            // The scene or layer went away mid-gesture. Keep owning the drag
            // so release still closes it cleanly.
            return true;
        };
        let current_pose = self
            .document
            .scene
            .as_ref()
            .and_then(|scene| layer_of(scene, drag.layer))
            .map(|entry| entry.transform);
        let Some(current_pose) = current_pose else {
            return true;
        };
        if step * current_pose == current_pose {
            return true;
        }
        if !self.tools.align.drag_pose_changed {
            // A click on a surface is only a grab. Invalidate the old fit and
            // announce movement when the first pointer segment actually changes
            // the pose, not while the operator is still deciding what to do.
            self.forget_align_fit(&self.ui.locale.tr(message_id!("align-status-moving-hand")));
            if let Some(name) = self.layer_display_name(drag.layer) {
                self.tools.align.status = Some(
                    self.ui
                        .locale
                        .tr_with(message_id!("align-drag-moving"), &[("name", &name)]),
                );
            }
            self.tools.align.drag_pose_changed = true;
        }
        self.nudge_align_layer(drag.layer, step);
        ctx.request_repaint();
        true
    }

    /// The world-space step one drag frame applies.
    ///
    /// Kept apart from the gesture handler so the pivot decision, the constraint
    /// handling and the screen-to-world conversion are readable on their own and
    /// the handler stays a state machine. `None` means the frame produced no
    /// usable step (a rotation whose pivot could not be resolved), which is not
    /// an error: the gesture continues and the pose is left alone.
    fn align_drag_step(
        &self,
        drag: AlignDrag,
        camera: &occluview_core::Camera,
        frame: DragFrame,
    ) -> Option<Affine3A> {
        let DragFrame {
            viewport,
            motion,
            rotating,
        } = frame;
        if !rotating {
            let right = camera
                .view_direction()
                .cross(camera.view_up())
                .normalize_or_zero();
            let world_per_pixel =
                crate::align_drag::mm_per_pixel(camera.orthographic_height, viewport.height());
            let moved = crate::align_drag::screen_delta_to_world(
                motion,
                right,
                camera.view_up(),
                world_per_pixel,
            );
            return Some(Affine3A::from_translation(
                crate::align_drag::constrain_translation(moved, self.tools.align.constraint),
            ));
        }
        let right = camera
            .view_direction()
            .cross(camera.view_up())
            .normalize_or_zero();
        let world_per_pixel =
            crate::align_drag::mm_per_pixel(camera.orthographic_height, viewport.height());
        // The clicked point is the rotation anchor in the current pose. Using
        // the current transform matters when the operator begins with a plain
        // translation and presses Ctrl partway through the same held drag.
        // A grab outside the mesh bounds falls back to the bounds centre.
        //
        // The translation constraint chips do not enter here. They are labelled
        // for movement, and a Ctrl-drag is a turn: letting a chip choose the
        // axis made the same gesture behave differently depending on a chip
        // about translation, and dropped the vertical component of the drag.
        let scene = self.document.scene.as_ref()?;
        let entry = layer_of(scene, drag.layer)?;
        let centre_local = entry.mesh.bbox_cached().center();
        let radius_local = entry.mesh.bbox_cached().size().length() * 0.5;
        let pivot_local =
            crate::align_drag::drag_pivot_local(drag.pivot_local, centre_local, radius_local);
        let pose_scale = entry
            .transform
            .matrix3
            .x_axis
            .length()
            .max(entry.transform.matrix3.y_axis.length())
            .max(entry.transform.matrix3.z_axis.length());
        let radius_world = radius_local * pose_scale;
        let turn = crate::align_drag::anchored_rotation_from_drag(
            motion,
            crate::align_drag::AnchoredRotationFrame::new(
                camera.view_direction(),
                right,
                camera.view_up(),
                world_per_pixel,
                radius_world,
            ),
        );
        let pivot_world = entry.transform.transform_point3(pivot_local);
        Some(crate::align_drag::rotation_about_pivot(turn, pivot_world))
    }

    /// Apply one drag step directly to the scene, without touching history.
    ///
    /// History is written once at release: a drag is one operator gesture, and
    /// filling the undo stack with a hundred per-frame steps would make Ctrl+Z
    /// useless.
    ///
    /// The update goes through the material path, not the structural one. A
    /// pose change is four rows of numbers; routing it through `set_scene` per
    /// mouse-move frame would cancel the bridge-split session, invalidate the
    /// sculpt session, and wipe every ruler measurement on screen mid-drag.
    /// It also goes in place. A pose is a transform, and the commit path that
    /// would carry it also rebuilds bookkeeping the drag would then have to
    /// undo each frame; going in place keeps a mouse-move frame to the fields
    /// it actually changes.
    pub(super) fn nudge_align_layer(&mut self, layer: SceneMeshId, step: Affine3A) {
        let started_at = self.tools.align.drag.map(|drag| drag.start);
        let Some(live) = self.document.live_scene_mut() else {
            return;
        };
        let mut pose = None;
        if let Some(entry) = live
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == layer)
        {
            entry.transform = step * entry.transform;
            pose = Some(entry.transform);
        }
        self.mark_scene_materials_changed();
        // Track the open drag separately so returning to its start does not
        // clear an older committed edit on the same layer.
        let Some(pose) = pose else {
            return;
        };
        self.document.unsaved_drag_pose = Some(pose) != started_at;
    }

    /// Close an open drag, recording the whole gesture as one undo step.
    pub(super) fn finish_align_drag(&mut self) -> bool {
        self.document.unsaved_drag_pose = false;
        self.tools.align.drag_last_pointer_pos = None;
        self.tools.align.drag_modifiers = None;
        self.tools.align.drag_pose_changed = false;
        let Some(drag) = self.tools.align.drag.take() else {
            return false;
        };
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };
        let Some(current) = layer_of(&scene, drag.layer).map(|entry| entry.transform) else {
            return false;
        };
        if current == drag.start {
            return false;
        }
        // Rewind to the pre-drag pose, open one history step, then re-apply the
        // pose the operator actually ended on.
        let mut before = scene.as_ref().clone();
        if let Some(entry) = before
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == drag.layer)
        {
            entry.transform = drag.start;
        }
        // Marked unsaved before the history step is attempted, because the pose
        // is already in the live scene (`nudge_align_layer` put it there frame
        // by frame). A moved scan is unsaved work: the viewer has no project
        // file, so the pose is the work product. When the edit state machine is
        // busy at the release frame this function returns without a history
        // step, and the flag still lets the close guard ask before the app shuts.
        self.document.mark_mesh_edits_unsaved(drag.layer);
        let Some(token) = self.document.edit_mode.begin_scene_edit(
            &before,
            drag.layer,
            EditModeCommand::MoveLayer,
        ) else {
            self.tools.align.status = Some(self.ui.locale.tr(message_id!("align-drag-unrecorded")));
            return false;
        };
        let mut after = before;
        if let Some(entry) = after
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == drag.layer)
        {
            entry.transform = current;
        }
        self.document
            .edit_mode
            .finish_scene_edit_success(token, &after);
        self.set_scene(after, false);
        // The status names the scan and the distance. This tool moves whichever
        // scan the operator grabbed, the fixed one included, so the status is
        // where an operator who grabbed the wrong arch by accident finds out.
        let moved_mm = f64::from((current.translation - drag.start.translation).length());
        let name = self
            .layer_display_name(drag.layer)
            .unwrap_or_else(|| self.ui.locale.tr(message_id!("align-status-one-scan")));
        // Teardown first so its status cannot overwrite the movement result.
        self.forget_align_fit(&self.ui.locale.tr(message_id!("align-status-moved-hand")));
        self.tools.align.status = Some(self.ui.locale.tr_with(
            message_id!("align-drag-moved"),
            &[("name", &name), ("moved", &format!("{moved_mm:.2}"))],
        ));
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use crate::app::OccluViewApp;

    use super::*;
    use crate::app::app_test_support::{named_scene, test_app};
    use glam::Quat;
    use occluview_core::Camera;

    /// A layer and a camera looking down at it, active enough to drag.
    fn rig(name: &str) -> (OccluViewApp, SceneMeshId, Camera) {
        let mut app = test_app(name);
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(named_scene("jaw", 0.0)));
        let id = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .id();
        let camera = Camera::default().frame_occlusal(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .bbox(),
            45.0_f32.to_radians(),
        );
        app.workspace.scenes[0].render.camera = Some(camera);
        (app, id, camera)
    }

    fn frame(rotating: bool) -> DragFrame {
        DragFrame {
            viewport: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
            motion: egui::vec2(40.0, 0.0),
            rotating,
        }
    }

    fn actual_drag_fixture(
        name: &str,
    ) -> (
        OccluViewApp,
        SceneMeshId,
        egui::Context,
        egui::Rect,
        egui::Pos2,
        egui::Modifiers,
        egui::Id,
        Vec3,
    ) {
        use crate::align_panel::AlignTab;
        use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
        use std::sync::Arc;

        let mesh = Mesh::new(
            Some("jaw".to_string()),
            vec![
                Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(40.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 40.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("a triangle is a mesh");
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        let id = scene.meshes()[0].id();
        let mut app = test_app(name);
        app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
        let camera = Camera::default().frame_occlusal(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .bbox(),
            45.0_f32.to_radians(),
        );
        app.workspace.scenes[0].render.camera = Some(camera);
        app.workspace.scenes[0].tools.align.tool.arm();
        app.workspace.scenes[0].tools.align.tab = AlignTab::Manually;

        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let target = Vec3::new(30.0, 5.0, 0.0);
        let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target)
            .expect("a surface point must project into the viewport");
        let ctx = egui::Context::default();
        app.ui.repaint_ctx = ctx.clone();
        let modifiers = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let viewport_id = egui::Id::new("align-raw-gesture-viewport");
        (app, id, ctx, rect, press_at, modifiers, viewport_id, target)
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn drive_actual_drag_frame(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        rect: egui::Rect,
        viewport_id: egui::Id,
        events: Vec<egui::Event>,
    ) -> bool {
        let raw = egui::RawInput {
            screen_rect: Some(rect),
            events,
            ..Default::default()
        };
        let mut consumed = false;
        ctx.run_ui(raw, |ui| {
            let response = ui.interact(rect, viewport_id, egui::Sense::click_and_drag());
            let frame_ctx = ui.ctx().clone();
            consumed = app
                .active_context()
                .expect("live test scene")
                .handle_align_drag(&response, &frame_ctx);
        })
        .drop_without_applying_deltas();
        consumed
    }

    fn expected_free_translation(
        camera: &Camera,
        viewport: egui::Rect,
        motion: egui::Vec2,
    ) -> Vec3 {
        let right = camera
            .view_direction()
            .cross(camera.view_up())
            .normalize_or_zero();
        let world_per_pixel =
            crate::align_drag::mm_per_pixel(camera.orthographic_height, viewport.height());
        crate::align_drag::screen_delta_to_world(motion, right, camera.view_up(), world_per_pixel)
    }

    /// A Ctrl-drag of a transformed scan keeps the clicked world point fixed.
    ///
    /// This exercises the transformed layer pose, camera-relative step, and
    /// world-space anchor together. The translation constraints must not alter
    /// the Ctrl turn.
    #[test]
    fn a_ctrl_drag_step_pins_the_pressed_point_for_every_constraint() {
        let mut steps = Vec::new();
        for constraint in [
            crate::align_drag::DragConstraint::Free,
            crate::align_drag::DragConstraint::ZOnly,
            crate::align_drag::DragConstraint::XyPlane,
        ] {
            let (mut app, id, _) = rig("ctrl-step-anchor");
            app.workspace.scenes[0].tools.align.constraint = constraint;
            let pose = Affine3A::from_translation(Vec3::new(30.0, -12.0, 7.0))
                * Affine3A::from_quat(Quat::from_rotation_y(0.45));
            let mut scene = app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .as_ref()
                .clone();
            scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == id)
                .expect("layer")
                .transform = pose;
            app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));
            let mut camera = Camera::default().frame_occlusal(
                app.workspace.scenes[0]
                    .document
                    .scene
                    .as_ref()
                    .expect("scene")
                    .bbox(),
                45.0_f32.to_radians(),
            );
            camera.orbit_view_by(0.35, -0.2);
            app.workspace.scenes[0].render.camera = Some(camera);

            // A valid off-centre point in the mesh's local frame.
            let grabbed_local = Vec3::new(0.75, 0.1, 0.0);
            let drag = AlignDrag {
                layer: id,
                start: pose,
                pivot_local: grabbed_local,
            };
            let anchor_world = pose.transform_point3(grabbed_local);
            let first_step = app
                .active_context()
                .expect("live test scene")
                .align_drag_step(drag, &camera, frame(true))
                .expect("a Ctrl-drag over a live layer must produce a step");
            assert!(
                (first_step.transform_point3(anchor_world) - anchor_world).length() < 1e-3,
                "{constraint:?}: the clicked point moved"
            );
            let scene = app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene");
            let entry = scene.meshes().iter().find(|e| e.id() == id).expect("layer");
            let centre_local = entry.mesh.bbox_cached().center();
            let centre_world = entry.transform.transform_point3(centre_local);
            assert!(
                (first_step.transform_point3(centre_world) - centre_world).length() > 1e-3,
                "{constraint:?}: an off-centre anchor must tilt the rest of the layer"
            );
            steps.push(first_step);
        }
        // The translation chips are labelled for movement; none of them may
        // change a Ctrl turn.
        for step in &steps[1..] {
            assert_eq!(
                *step, steps[0],
                "a translation chip changed the Ctrl-drag turn"
            );
        }
    }

    /// Switching modifiers during one held gesture keeps the same local
    /// surface anchor attached to the pose as translation moves it.
    #[test]
    fn changing_between_translation_and_tilt_keeps_the_current_grab_point() {
        let (mut app, id, mut camera) = rig("mixed-manual-gesture");
        camera.orbit_view_by(0.25, -0.18);
        app.workspace.scenes[0].render.camera = Some(camera);
        let pose = Affine3A::from_translation(Vec3::new(4.0, -2.0, 6.0))
            * Affine3A::from_quat(Quat::from_rotation_x(0.3));
        let anchor_local = Vec3::new(0.75, 0.1, 0.0);
        let drag = AlignDrag {
            layer: id,
            start: pose,
            pivot_local: anchor_local,
        };
        app.workspace.scenes[0].tools.align.drag = Some(drag);
        let mut scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .as_ref()
            .clone();
        scene
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == id)
            .expect("layer")
            .transform = pose;
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));

        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let anchor_before = pose.transform_point3(anchor_local);
        let translation = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(
                drag,
                &camera,
                DragFrame {
                    viewport,
                    motion: egui::vec2(12.0, -6.0),
                    rotating: false,
                },
            )
            .expect("plain movement step");
        app.active_context()
            .expect("live test scene")
            .nudge_align_layer(id, translation);
        let translated_scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let translated = translated_scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("layer")
            .transform;
        let translated_anchor = translated.transform_point3(anchor_local);
        assert!(
            (translated_anchor - anchor_before - Vec3::from(translation.translation)).length()
                < 1e-3,
            "plain movement should carry the grabbed point with the layer"
        );

        let rotation = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(true))
            .expect("Ctrl movement step");
        assert!(
            (rotation.transform_point3(translated_anchor) - translated_anchor).length() < 1e-3,
            "adding Ctrl should turn around the point in its current pose"
        );
        app.active_context()
            .expect("live test scene")
            .nudge_align_layer(id, rotation);

        let after_rotation = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let turned = after_rotation
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("layer")
            .transform;
        let turned_anchor = turned.transform_point3(anchor_local);
        assert!((turned_anchor - translated_anchor).length() < 1e-3);

        let next_translation = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(
                drag,
                &camera,
                DragFrame {
                    viewport,
                    motion: egui::vec2(-5.0, 7.0),
                    rotating: false,
                },
            )
            .expect("plain movement after Ctrl");
        app.active_context()
            .expect("live test scene")
            .nudge_align_layer(id, next_translation);
        let moved_again = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let moved_pose = moved_again
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("layer")
            .transform;
        let moved_anchor = moved_pose.transform_point3(anchor_local);
        assert!(
            (moved_anchor - turned_anchor - Vec3::from(next_translation.translation)).length()
                < 1e-3,
            "plain movement after Ctrl should carry the same grabbed point"
        );

        let final_rotation = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(true))
            .expect("second Ctrl movement");
        assert!(
            (final_rotation.transform_point3(moved_anchor) - moved_anchor).length() < 1e-3,
            "a later Ctrl step should preserve the translated grab point"
        );
    }

    /// A grab far outside the scan falls back to the centre rather than
    /// freezing the gesture or turning about a point at infinity.
    #[test]
    fn an_absurd_grab_still_produces_a_usable_step() {
        let (mut app, id, camera) = rig("absurd-grab");
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::splat(1.0e9),
        };
        let step = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(true))
            .expect("an absurd grab must still leave the gesture alive");
        assert!(step.is_finite(), "{step:?}");
        // The discarded grab is replaced by the layer's own centre, so the
        // centre is what the turn fixes.
        let scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let centre = scene.meshes()[0].mesh.bbox_cached().center();
        let pinned = step.transform_point3(centre);
        assert!(
            (pinned - centre).length() < 1e-3,
            "the fallback pivot should be the layer centre, which moved {pinned:?} \
             away from {centre:?}"
        );
    }

    /// A layer that vanished mid-gesture refuses the step instead of panicking.
    #[test]
    fn a_step_for_a_missing_layer_produces_nothing() {
        let (mut app, id, camera) = rig("missing-layer");
        // Drop the scene out from under the open gesture.
        app.workspace.scenes[0].document.scene = None;
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::ZERO,
        };
        assert!(
            app.active_context()
                .expect("live test scene")
                .align_drag_step(drag, &camera, frame(true))
                .is_none(),
            "a step with no scene must refuse rather than guess"
        );
    }

    /// A non-rotation frame is a translation and must not touch the rotation.
    #[test]
    fn a_plain_drag_step_translates_instead_of_turning() {
        let (mut app, id, camera) = rig("plain-drag");
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::new(5.0, 5.0, 5.0),
        };
        let step = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(false))
            .expect("a plain drag must produce a step");
        let rotation = Quat::from_mat3a(&step.matrix3);
        assert!(
            rotation.angle_between(Quat::IDENTITY) < 1e-4,
            "a plain drag must not rotate: {rotation:?}"
        );
    }

    /// The actual press ray must select a surface anchor that stays fixed
    /// through a camera-relative Ctrl turn.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "One actual press-drag-release scenario keeps its UI setup and geometry assertions together."
    )]
    fn a_real_ctrl_drag_gesture_pivots_at_the_grabbed_surface_point() {
        use crate::align_panel::AlignTab;
        use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
        use std::sync::Arc;

        /// A wide triangle, so a click lands well off the mesh centre.
        fn wide_scene() -> (Scene, SceneMeshId) {
            let mesh = Mesh::new(
                Some("jaw".to_string()),
                vec![
                    Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
                    Vertex::at(Vec3::new(40.0, 0.0, 0.0)),
                    Vertex::at(Vec3::new(0.0, 40.0, 0.0)),
                ],
                vec![0, 1, 2],
            )
            .expect("a triangle is a mesh");
            let mut scene = Scene::new();
            scene.add(SceneMesh::new(mesh));
            let id = scene.meshes()[0].id();
            (scene, id)
        }

        let mut app = test_app("real-ctrl-drag-gesture");
        let (scene, id) = wide_scene();
        app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
        let bbox = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .bbox();
        let camera = Camera::default().frame_occlusal(bbox, 45.0_f32.to_radians());
        app.workspace.scenes[0].render.camera = Some(camera);
        app.workspace.scenes[0].tools.align.tool.arm();
        app.workspace.scenes[0].tools.align.tab = AlignTab::Manually;

        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        // A point on the surface, far from the centre at (20, 20, 0).
        let target = Vec3::new(30.0, 5.0, 0.0);
        let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target)
            .expect("a surface point must project into the viewport");
        let modifiers = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let viewport_id = egui::Id::new("gesture-viewport");

        // The interaction state must persist across frames, so one context
        // drives every frame of the gesture.
        let ctx = egui::Context::default();
        let mut frame = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(rect),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| {
                let response = ui.interact(rect, viewport_id, egui::Sense::click_and_drag());
                let frame_ctx = ui.ctx().clone();
                app.active_context()
                    .expect("live test scene")
                    .handle_align_drag(&response, &frame_ctx);
            })
            .drop_without_applying_deltas();
        };

        // Frame 0: register the viewport widget so egui can hit-test the press.
        frame(vec![]);
        // Frame 1: the coalesced frame. A fast flick, or a delayed egui pass,
        // delivers the primary press and the pointer's move in one batch: the
        // press lands on the surface, then the pointer is already 60 px away by
        // the end of the same frame. The frame's current pointer position is
        // therefore NOT where the button went down, and neither `hover_pos` nor
        // `interact_pointer_pos` can be used to anchor the turn.
        frame(vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(press_at),
            egui::Event::PointerButton {
                pos: press_at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerMoved(press_at + egui::vec2(60.0, 20.0)),
        ]);
        // Frame 2: a secondary press lands elsewhere. egui keeps a single
        // `press_origin` for every button, so this is what would displace the
        // anchor if the grab read that shared slot.
        frame(vec![
            egui::Event::PointerButton {
                pos: press_at + egui::vec2(-120.0, -60.0),
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerMoved(press_at + egui::vec2(90.0, 40.0)),
        ]);
        // Frame 3: no button is being pressed now, so egui promotes the held
        // primary to a drag and the grab is resolved.
        frame(vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(press_at + egui::vec2(100.0, 45.0)),
        ]);

        let Some(drag) = app.workspace.scenes[0].tools.align.drag else {
            panic!("a Ctrl-drag over the surface must open a drag");
        };
        let scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let entry = scene.meshes().iter().find(|e| e.id() == id).expect("layer");
        let centre = entry.mesh.bbox_cached().center();

        // The fixture must be an off-centre grab, or a centre pivot would look
        // identical and the test would prove nothing.
        assert!(
            (drag.pivot_local - centre).length() > 5.0,
            "the grab {:?} is too close to the centre {centre:?} to tell the two apart",
            drag.pivot_local
        );
        // The grab must be the surface point under the cursor, not the centre.
        assert!(
            (drag.pivot_local - target).length() < 0.5,
            "the grab stored {:?} but the operator pressed on {target:?}",
            drag.pivot_local
        );

        // A direct comparison against the original ray hit verifies the actual
        // grabbed surface point, not just a synthetic pivot argument.
        let pressed_world = entry.transform.transform_point3(drag.pivot_local);
        let pressed_before = drag.start.transform_point3(drag.pivot_local);
        assert!(
            (pressed_world - pressed_before).length() < 1e-2,
            "the clicked surface point moved from {pressed_before:?} to {pressed_world:?}"
        );
        // The grabbed point is off-centre, so the turn changes the layer around
        // it rather than silently turning about the bounds centre.
        let centre_world = entry.transform.transform_point3(centre);
        assert!(
            (centre_world - drag.start.transform_point3(centre)).length() > 0.1,
            "an off-centre pivot should turn the layer centre"
        );
        assert_ne!(
            entry.transform,
            Affine3A::IDENTITY,
            "the Ctrl-drag produced no pose change at all"
        );
    }

    /// A fast gesture can be delivered entirely in one egui frame. The raw
    /// event stream must still open, move, and close one Ctrl tilt at the point
    /// under the press, with one history entry.
    #[test]
    fn a_coalesced_ctrl_press_move_release_pins_and_records_one_drag() {
        let (mut app, id, ctx, rect, press_at, modifiers, viewport_id, target) =
            actual_drag_fixture("coalesced-ctrl-drag");
        let moved_at = press_at + egui::vec2(60.0, 20.0);
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);

        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, modifiers),
                egui::Event::PointerMoved(moved_at),
                pointer_button(moved_at, false, modifiers),
            ],
        ));

        assert!(
            app.workspace.scenes[0].tools.align.drag.is_none(),
            "the release closes the drag"
        );
        let entry = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("jaw");
        assert_ne!(entry.transform, Affine3A::IDENTITY, "the move was applied");
        assert!(
            (entry.transform.transform_point3(target) - target).length() < 1e-2,
            "Ctrl rotation must keep the pressed surface point in place"
        );
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
    }

    /// The actual pointer path has to ray-pick in a transformed instance, not
    /// just preserve a pivot a unit test placed into `AlignDrag` itself.
    #[test]
    fn a_ctrl_press_ray_picks_and_pins_a_transformed_surface_anchor() {
        let (mut app, id, ctx, rect, _, modifiers, viewport_id, _) =
            actual_drag_fixture("transformed-ctrl-ray-pick");
        let pose = Affine3A::from_translation(Vec3::new(11.0, -7.0, 5.0))
            * Affine3A::from_quat(Quat::from_rotation_y(0.34) * Quat::from_rotation_x(-0.21));
        let mut scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .as_ref()
            .clone();
        scene
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == id)
            .expect("jaw")
            .transform = pose;
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));
        let camera = Camera::default().frame_occlusal(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .bbox(),
            45.0_f32.to_radians(),
        );
        app.workspace.scenes[0].render.camera = Some(camera);

        let target_local = Vec3::new(12.0, 12.0, 0.0);
        let target_world = pose.transform_point3(target_local);
        let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target_world)
            .expect("the transformed surface point projects into the viewport");
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, modifiers),
            ],
        ));

        let drag = app.workspace.scenes[0]
            .tools
            .align
            .drag
            .expect("the press ray opens a drag");
        assert_eq!(drag.layer, id);
        assert!(
            (drag.pivot_local - target_local).length() < 1e-2,
            "the press ray must recover the known local point: {:?} vs {:?}",
            drag.pivot_local,
            target_local
        );
        let grabbed_world = pose.transform_point3(drag.pivot_local);
        let moved_at = press_at + egui::vec2(48.0, 26.0);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(moved_at),
                pointer_button(moved_at, false, modifiers),
            ],
        ));

        let moved = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("jaw");
        assert_ne!(moved.transform, pose, "the Ctrl gesture must turn the scan");
        assert!(
            (moved.transform.transform_point3(drag.pivot_local) - grabbed_world).length() < 1e-2,
            "the ray-picked world point must stay fixed through Ctrl tilt"
        );
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
    }

    /// Once a surface press owns a drag, moving and releasing outside the
    /// viewport still belongs to the align gesture rather than to the camera.
    #[test]
    fn a_surface_drag_can_release_outside_the_viewport() {
        let (mut app, id, ctx, rect, press_at, _modifiers, viewport_id, _) =
            actual_drag_fixture("outside-release-drag");
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, egui::Modifiers::NONE),
            ],
        ));
        assert!(app.workspace.scenes[0].tools.align.drag.is_some());

        let outside = egui::pos2(900.0, 700.0);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, false, egui::Modifiers::NONE),
            ],
        ));
        assert!(app.workspace.scenes[0].tools.align.drag.is_none());
        assert_ne!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .meshes()[0]
                .transform,
            Affine3A::IDENTITY,
            "motion through release outside the viewport must be applied"
        );
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .meshes()[0]
                .id(),
            id
        );
    }

    /// Pressing and releasing without motion starts no edit, so an already
    /// landed fit remains valid until the operator actually changes the pose.
    #[test]
    fn a_surface_click_without_motion_keeps_the_landed_fit() {
        let (mut app, _, ctx, rect, press_at, _modifiers, viewport_id, _) =
            actual_drag_fixture("align-grab-without-move");
        app.workspace.scenes[0].tools.align.refined_match_ready = true;
        app.workspace.scenes[0].tools.align.settings.show_deviation = true;
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, egui::Modifiers::NONE),
            ],
        ));
        assert!(app.workspace.scenes[0].tools.align.refined_match_ready);
        assert!(app.workspace.scenes[0].tools.align.settings.show_deviation);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![pointer_button(press_at, false, egui::Modifiers::NONE,)],
        ));
        assert!(app.workspace.scenes[0].tools.align.refined_match_ready);
        assert!(app.workspace.scenes[0].tools.align.settings.show_deviation);
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
    }

    /// The final `InputState::modifiers` value applies to the whole egui pass,
    /// not to each event in order. Replaying a move before and after Ctrl changes
    /// must therefore start from the modifier state held by the previous frame.
    #[test]
    fn modifier_changes_keep_raw_move_order_across_frames() {
        // Plain translation, then Ctrl rotation in one batch that ends with
        // Ctrl down. Its first move must still translate the clicked anchor.
        let (mut app, _, ctx, rect, press_at, ctrl, viewport_id, _) =
            actual_drag_fixture("translate-before-ctrl-same-batch");
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, egui::Modifiers::NONE),
            ],
        ));
        let drag = app.workspace.scenes[0]
            .tools
            .align
            .drag
            .expect("surface press opens drag");
        let camera = app.workspace.scenes[0].render.camera.expect("camera");
        let plain_motion = egui::vec2(16.0, 8.0);
        let turn_motion = egui::vec2(48.0, -18.0);
        let after_plain = press_at + plain_motion;
        let after_turn = after_plain + turn_motion;
        let expected_anchor = drag.start.transform_point3(drag.pivot_local)
            + expected_free_translation(&camera, rect, plain_motion);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(after_plain),
                egui::Event::ModifiersChanged(ctrl),
                egui::Event::PointerMoved(after_turn),
                pointer_button(after_turn, false, ctrl),
            ],
        ));
        let moved = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .transform;
        assert!(
            (moved.transform_point3(drag.pivot_local) - expected_anchor).length() < 1e-2,
            "the first movement must translate before the later Ctrl turn"
        );
        assert_ne!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);

        // The reverse ordering starts with Ctrl down, then releases it before
        // the second move in a batch whose final modifiers are NONE.
        let (mut app, _, ctx, rect, press_at, ctrl, viewport_id, _) =
            actual_drag_fixture("ctrl-before-translation-same-batch");
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::ModifiersChanged(ctrl),
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, ctrl),
            ],
        ));
        let drag = app.workspace.scenes[0]
            .tools
            .align
            .drag
            .expect("surface press opens drag");
        let camera = app.workspace.scenes[0].render.camera.expect("camera");
        let turn_motion = egui::vec2(46.0, 22.0);
        let plain_motion = egui::vec2(-14.0, 11.0);
        let after_turn = press_at + turn_motion;
        let after_plain = after_turn + plain_motion;
        let expected_anchor = drag.start.transform_point3(drag.pivot_local)
            + expected_free_translation(&camera, rect, plain_motion);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::PointerMoved(after_turn),
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(after_plain),
                pointer_button(after_plain, false, egui::Modifiers::NONE),
            ],
        ));
        let moved = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .transform;
        assert!(
            (moved.transform_point3(drag.pivot_local) - expected_anchor).length() < 1e-2,
            "the first Ctrl movement must pin the press before later translation"
        );
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
    }

    /// Cancel discards the open gesture before restoring the session snapshot.
    /// The restoration is one undoable cancellation entry; undoing it recovers
    /// the pose the active drag had reached.
    #[test]
    fn cancel_discards_an_open_ctrl_drag_and_records_only_the_restore() {
        let (mut app, id, ctx, rect, press_at, modifiers, viewport_id, _) =
            actual_drag_fixture("cancel-open-ctrl-drag");
        app.active_context()
            .expect("live test scene")
            .arm_align_tool(&ctx);
        app.workspace.scenes[0].tools.align.tab = crate::align_panel::AlignTab::Manually;
        drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
        let moved_at = press_at + egui::vec2(50.0, -16.0);
        assert!(drive_actual_drag_frame(
            &mut app,
            &ctx,
            rect,
            viewport_id,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(press_at),
                pointer_button(press_at, true, modifiers),
                egui::Event::PointerMoved(moved_at),
            ],
        ));
        assert!(app.workspace.scenes[0].tools.align.drag.is_some());
        let moved_pose = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("jaw")
            .transform;
        assert_ne!(moved_pose, Affine3A::IDENTITY);
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);

        app.active_context()
            .expect("live test scene")
            .cancel_align_session(&ctx);
        assert!(app.workspace.scenes[0].tools.align.drag.is_none());
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .meshes()[0]
                .transform,
            Affine3A::IDENTITY
        );
        assert_eq!(
            app.workspace.scenes[0].document.edit_mode.undo_len(),
            1,
            "Cancel should have one restore entry, with no drag commit before it"
        );

        app.active_context()
            .expect("live test scene")
            .apply_history_navigation_now(false, &ctx);
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .meshes()
                .iter()
                .find(|entry| entry.id() == id)
                .expect("jaw")
                .transform,
            moved_pose,
            "one undo should cancel the Cancel"
        );
        assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
    }
}
