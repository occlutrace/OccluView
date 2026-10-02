//! Moving a scan by hand while Align Scans is armed.
//!
//! `align` routes clicks and jobs; this module owns one continuous pointer
//! gesture and the single history entry it becomes.

use eframe::egui;
use glam::{Affine3A, Vec3};
use occluview_core::SceneMeshId;

use super::super::SceneContext;
use super::layer_of;
use crate::edit_mode::EditModeCommand;
use crate::i18n::message_id;
use crate::viewer::pick_scene_hit;

mod replay;
#[cfg(test)]
mod tests;

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
    pub(in crate::app) layer: SceneMeshId,
    /// Its pose when the gesture began, so the whole drag is one undo step.
    pub(in crate::app) start: Affine3A,
    /// The surface point the operator grabbed, in the layer's own local frame.
    /// Each Ctrl-drag step maps it through the current pose and turns around
    /// that world point, so an earlier plain drag carries the anchor with it.
    pub(in crate::app) pivot_local: Vec3,
}

impl SceneContext<'_> {
    /// Commit an open drag before a scene transition that keeps its layer.
    pub(in crate::app) fn abandon_align_drag(&mut self) {
        self.finish_align_drag();
    }

    /// Discard a drag when its scene is replaced or cleared.
    pub(in crate::app) fn discard_align_drag(&mut self) {
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
    pub(in crate::app) fn handle_align_drag(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        // Dragging a scan lives in the Manually tab, the way lab software
        // splits it. In the Automatically tab a press is always a landmark:
        // egui promotes a press to a drag after six pixels or eight tenths of a
        // second, so without this gate a careful click on a cusp would move the
        // scan instead of placing a point.
        if self.tools.align.tab != crate::align::align_panel::AlignTab::Manually {
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
            let world_per_pixel = crate::align::align_drag::mm_per_pixel(
                camera.orthographic_height,
                viewport.height(),
            );
            let moved = crate::align::align_drag::screen_delta_to_world(
                motion,
                right,
                camera.view_up(),
                world_per_pixel,
            );
            return Some(Affine3A::from_translation(
                crate::align::align_drag::constrain_translation(moved, self.tools.align.constraint),
            ));
        }
        let right = camera
            .view_direction()
            .cross(camera.view_up())
            .normalize_or_zero();
        let world_per_pixel =
            crate::align::align_drag::mm_per_pixel(camera.orthographic_height, viewport.height());
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
        let pivot_local = crate::align::align_drag::drag_pivot_local(
            drag.pivot_local,
            centre_local,
            radius_local,
        );
        let pose_scale = entry
            .transform
            .matrix3
            .x_axis
            .length()
            .max(entry.transform.matrix3.y_axis.length())
            .max(entry.transform.matrix3.z_axis.length());
        let radius_world = radius_local * pose_scale;
        let turn = crate::align::align_drag::anchored_rotation_from_drag(
            motion,
            crate::align::align_drag::AnchoredRotationFrame::new(
                camera.view_direction(),
                right,
                camera.view_up(),
                world_per_pixel,
                radius_world,
            ),
        );
        let pivot_world = entry.transform.transform_point3(pivot_local);
        Some(crate::align::align_drag::rotation_about_pivot(
            turn,
            pivot_world,
        ))
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
    pub(in crate::app) fn nudge_align_layer(&mut self, layer: SceneMeshId, step: Affine3A) {
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
    pub(in crate::app) fn finish_align_drag(&mut self) -> bool {
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
        let number_format = self.ui.locale.number_format();
        let name = self
            .layer_display_name(drag.layer)
            .unwrap_or_else(|| self.ui.locale.tr(message_id!("align-status-one-scan")));
        // Teardown first so its status cannot overwrite the movement result.
        self.forget_align_fit(&self.ui.locale.tr(message_id!("align-status-moved-hand")));
        self.tools.align.status = Some(self.ui.locale.tr_with(
            message_id!("align-drag-moved"),
            &[
                ("name", &name),
                ("moved", &number_format.decimal(moved_mm, 2)),
            ],
        ));
        true
    }
}

/// Roll back just the in-progress manual pose drag after focus leaves the
/// window. The Align session remains armed; unlike Escape this does not restore
/// earlier, already completed moves from the session.
pub(in crate::app) fn rollback_align_drag(scene: &mut SceneContext<'_>) {
    let drag = scene.tools.align.drag.take();
    scene.discard_align_drag();
    let Some(drag) = drag else {
        return;
    };
    let changed = scene.document.live_scene_mut().is_some_and(|live| {
        let Some(entry) = live
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == drag.layer)
        else {
            return false;
        };
        let changed = entry.transform != drag.start;
        entry.transform = drag.start;
        changed
    });
    scene.document.unsaved_drag_pose = false;
    if changed {
        scene.mark_scene_materials_changed();
    }
}
