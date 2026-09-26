//! Moving a scan by hand while Align Scans is armed.
//!
//! `app_align` routes clicks and jobs; this module owns one continuous pointer
//! gesture and the single history entry it becomes.

use eframe::egui;
use glam::{Affine3A, Vec3};
use occluview_core::SceneMeshId;

use super::app_align::layer_of;
use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
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

/// An open hand drag.
#[derive(Clone, Copy)]
pub(crate) struct AlignDrag {
    /// The layer the operator grabbed.
    pub(super) layer: SceneMeshId,
    /// Its pose when the gesture began, so the whole drag is one undo step.
    pub(super) start: Affine3A,
    /// The surface point the operator grabbed, in the layer's own local frame.
    /// Kept local so a Ctrl-drag pivots about the point under the cursor even
    /// after the gesture has already moved or turned the scan.
    pub(super) pivot_local: Vec3,
}

impl OccluViewApp {
    /// Commit an open drag before a scene transition that keeps its layer.
    pub(super) fn abandon_align_drag(&mut self) {
        self.finish_align_drag();
    }

    /// Discard a drag when its scene is replaced or cleared.
    pub(super) fn discard_align_drag(&mut self) {
        self.tools.align.drag = None;
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
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if !primary_down {
            return self.finish_align_drag();
        }
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|input| input.pointer.hover_pos()))
        else {
            return false;
        };
        let motion = ctx.input(|input| input.pointer.delta());

        if self.tools.align.drag.is_none() {
            if !response.drag_started_by(egui::PointerButton::Primary) {
                return false;
            }
            let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) else {
                return false;
            };
            // The scan being placed gets first refusal on the grab.
            //
            // Two scans in an alignment overlap by definition, so the nearest
            // surface under the cursor is often the other one. Picking it would
            // move the reference while the operator aims at the arch being
            // placed, and exporting that arch would return it in its original
            // position. Only after the moving scan misses entirely does the
            // general pick run, so grabbing the reference on purpose still
            // works: aim where the moving scan is not.
            let hit = self
                .tools
                .align
                .tool
                .moving_layer()
                .and_then(|moving| {
                    crate::viewer::pick_layer_hit(&camera, response.rect, pointer, &scene, moving)
                })
                .or_else(|| pick_scene_hit(&camera, response.rect, pointer, &scene));
            let Some(hit) = hit else {
                // A drag from empty space is the camera's, not the tool's.
                return false;
            };
            let Some(entry) = scene.meshes().get(hit.layer_index) else {
                return false;
            };
            // The grabbed surface point, converted into the layer's own local
            // frame. A Ctrl-drag turns about it so the point the operator
            // pulled stays under the cursor; keeping it local means the pivot
            // is still correct after the gesture has already moved the scan.
            //
            // The inverse is guarded, and the guard has to be a magnitude check,
            // not `is_finite` alone. A nearly singular pose (a scale a few orders
            // below a millimetre) inverts to a finite matrix with entries around
            // 1e30, so `is_finite` passes and the pivot lands far off the scene;
            // the step built from it then carries the scan off screen. The
            // relative bound is applied later, against the layer's own size, so
            // the decision does not depend on where the scene sits in the world.
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
            // Nothing below reads the scene, and what follows edits it in
            // place: `forget_align_fit` reaches `live_scene_mut` through the
            // deviation overlay, and a second handle alive there copies the
            // container. This function drops its own handle instead of relying
            // on an early return in another module.
            drop(scene);
            // The map describes the pose the scan is leaving. Dropped once, at
            // the start of the gesture, rather than at the end: otherwise for
            // the whole hand-drag the colours would stay on the surface at
            // distances that are no longer true, reading as a heatmap that
            // agrees with wherever the operator drags it.
            self.forget_align_fit(&self.ui.locale.tr("align-status-moving-hand"));
            // Said at the start as well as the end, because this is the moment
            // the operator can still let go and try again if they grabbed the
            // arch they did not mean to.
            if let Some(name) = self.layer_display_name(hit.layer_id) {
                self.tools.align.status = Some(
                    self.ui
                        .locale
                        .tr_with("align-drag-moving", &[("name", &name)]),
                );
            }
        }

        let Some(drag) = self.tools.align.drag else {
            return false;
        };
        if motion.length_sq() <= f32::EPSILON {
            return true;
        }
        let Some(camera) = self.render.camera else {
            return true;
        };
        let rotating = ctx.input(|input| input.modifiers.command);
        let frame = DragFrame {
            viewport: response.rect,
            motion,
            rotating,
        };
        let Some(step) = self.align_drag_step(drag, &camera, frame) else {
            // The scene or the layer went away mid-gesture. Keep owning the
            // drag so releasing it still commits cleanly.
            return true;
        };

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
        let turn = crate::align_drag::constrained_rotation_from_drag(
            motion,
            right,
            camera.view_up(),
            crate::align_drag::DEGREES_PER_PIXEL,
            self.tools.align.constraint,
        );
        // Which point the turn fixes is always the point the operator grabbed:
        // the constraint chooses the rotation axis, never the pivot. The grabbed
        // point is carried in the layer's own frame and mapped through its
        // current pose, so a gesture that has already moved the scan keeps
        // turning about the same physical point under the cursor. A grab that
        // cannot be trusted (a singular pose, a point outside the scan) falls
        // back to the layer centre rather than freezing the gesture.
        let scene = self.document.scene.as_ref()?;
        let entry = layer_of(scene, drag.layer)?;
        let centre_local = entry.mesh.bbox_cached().center();
        let radius_local = entry.mesh.bbox_cached().size().length() * 0.5;
        let pivot_local =
            crate::align_drag::drag_pivot_local(drag.pivot_local, centre_local, radius_local);
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
            self.tools.align.status = Some(self.ui.locale.tr("align-drag-unrecorded"));
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
            .unwrap_or_else(|| self.ui.locale.tr("align-status-one-scan"));
        // Teardown first so its status cannot overwrite the movement result.
        self.forget_align_fit(&self.ui.locale.tr("align-status-moved-hand"));
        self.tools.align.status = Some(self.ui.locale.tr_with(
            "align-drag-moved",
            &[("name", &name), ("moved", &format!("{moved_mm:.2}"))],
        ));
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::app::app_test_support::{named_scene, test_app};
    use glam::Quat;
    use occluview_core::Camera;

    /// A layer and a camera looking down at it, active enough to drag.
    fn rig(name: &str) -> (OccluViewApp, SceneMeshId, Camera) {
        let mut app = test_app(name);
        app.document.scene = Some(std::sync::Arc::new(named_scene("jaw", 0.0)));
        let id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
        let camera = Camera::default().frame_occlusal(
            app.document.scene.as_ref().expect("scene").bbox(),
            45.0_f32.to_radians(),
        );
        app.render.camera = Some(camera);
        (app, id, camera)
    }

    fn frame(rotating: bool) -> DragFrame {
        DragFrame {
            viewport: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
            motion: egui::vec2(40.0, 0.0),
            rotating,
        }
    }

    /// The wiring the operator actually touches: a Ctrl-drag of a layer whose
    /// grab sits well off the mesh centre must turn about that grab, under every
    /// drag constraint.
    ///
    /// This is the end-to-end path the unit tests below cannot reach — the
    /// constraint choice, the local-to-world mapping and the pre-multiplication
    /// all live in `align_drag_step`, so a pivot error there (for example
    /// falling back to the layer centre) would otherwise go undetected.
    #[test]
    fn a_ctrl_drag_step_turns_about_the_grabbed_point_for_every_constraint() {
        for constraint in [
            crate::align_drag::DragConstraint::Free,
            crate::align_drag::DragConstraint::ZOnly,
            crate::align_drag::DragConstraint::XyPlane,
        ] {
            let (mut app, id, camera) = rig("ctrl-step-pivot");
            app.tools.align.constraint = constraint;
            // A grab far from the mesh centre, so a centre pivot is detectable.
            let grabbed_local = Vec3::new(9.0, -4.0, 2.0);
            let drag = AlignDrag {
                layer: id,
                start: Affine3A::IDENTITY,
                pivot_local: grabbed_local,
            };
            let step = app
                .align_drag_step(drag, &camera, frame(true))
                .expect("a Ctrl-drag over a live layer must produce a step");

            let pivot_world = step.transform_point3(grabbed_local);
            assert!(
                (pivot_world - grabbed_local).length() < 1e-3,
                "{constraint:?}: the grabbed point moved to {pivot_world:?}; the turn \
                 did not happen about what the operator pulled"
            );
            // And it is genuinely a rotation about that point, not the identity.
            assert!(
                step.transform_point3(Vec3::ZERO).length() > 1e-3,
                "{constraint:?}: the step collapsed to nothing"
            );
        }
    }

    /// A grab far outside the scan falls back to the centre rather than
    /// freezing the gesture or turning about a point at infinity.
    #[test]
    fn an_absurd_grab_still_produces_a_usable_step() {
        let (app, id, camera) = rig("absurd-grab");
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::splat(1.0e9),
        };
        let step = app
            .align_drag_step(drag, &camera, frame(true))
            .expect("an absurd grab must still leave the gesture alive");
        assert!(step.is_finite(), "{step:?}");
        // The discarded grab is replaced by the layer's own centre, so the
        // centre is what the turn fixes.
        let scene = app.document.scene.as_ref().expect("scene");
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
        app.document.scene = None;
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::ZERO,
        };
        assert!(
            app.align_drag_step(drag, &camera, frame(true)).is_none(),
            "a step with no scene must refuse rather than guess"
        );
    }

    /// A non-rotation frame is a translation and must not touch the rotation.
    #[test]
    fn a_plain_drag_step_translates_instead_of_turning() {
        let (app, id, camera) = rig("plain-drag");
        let drag = AlignDrag {
            layer: id,
            start: Affine3A::IDENTITY,
            pivot_local: Vec3::new(5.0, 5.0, 5.0),
        };
        let step = app
            .align_drag_step(drag, &camera, frame(false))
            .expect("a plain drag must produce a step");
        let rotation = Quat::from_mat3a(&step.matrix3);
        assert!(
            rotation.angle_between(Quat::IDENTITY) < 1e-4,
            "a plain drag must not rotate: {rotation:?}"
        );
    }
}
