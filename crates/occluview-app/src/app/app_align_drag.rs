//! Moving a scan by hand while Align Scans is armed.
//!
//! Split from `app_align` because it answers a different question: that module
//! routes clicks and jobs, this one owns one continuous pointer gesture and the
//! single history entry it becomes.

use eframe::egui;
use glam::{Affine3A, Vec3};
use occluview_core::SceneMeshId;

use super::app_align::layer_of;
use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
use crate::viewer::pick_scene_hit;

/// An open hand drag.
#[derive(Clone, Copy)]
pub(super) struct AlignDrag {
    /// The layer the operator grabbed.
    pub(super) layer: SceneMeshId,
    /// Its pose when the gesture began, so the whole drag is one undo step.
    pub(super) start: Affine3A,
    /// Its centre in world, the pivot a Ctrl-drag turns about.
    pub(super) centroid: Vec3,
}

impl OccluViewApp {
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

        if self.align_drag.is_none() {
            if !response.drag_started_by(egui::PointerButton::Primary) {
                return false;
            }
            let Some((camera, scene)) = self.camera.zip(self.scene.clone()) else {
                return false;
            };
            let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
                // A drag from empty space is the camera's, not the tool's.
                return false;
            };
            let Some(entry) = scene.meshes().get(hit.layer_index) else {
                return false;
            };
            self.align_drag = Some(AlignDrag {
                layer: hit.layer_id,
                start: entry.transform,
                centroid: entry
                    .transform
                    .transform_point3(entry.mesh.bbox_cached().center()),
            });
        }

        let Some(drag) = self.align_drag else {
            return false;
        };
        if motion.length_sq() <= f32::EPSILON {
            return true;
        }
        let Some(camera) = self.camera else {
            return true;
        };
        let up = camera.view_up();
        let right = camera.view_direction().cross(up).normalize_or_zero();
        let rotating = ctx.input(|input| input.modifiers.command);

        let step = if rotating {
            let turn = crate::align_drag::rotation_from_drag(
                motion,
                right,
                up,
                crate::align_drag::DEGREES_PER_PIXEL,
            );
            // Turn about the layer's own centre, so the scan spins in place
            // instead of orbiting the world origin.
            Affine3A::from_translation(drag.centroid)
                * Affine3A::from_quat(turn)
                * Affine3A::from_translation(-drag.centroid)
        } else {
            let world_per_pixel = camera.orthographic_height / response.rect.height().max(1.0);
            let moved =
                crate::align_drag::screen_delta_to_world(motion, right, up, world_per_pixel);
            Affine3A::from_translation(crate::align_drag::constrain_translation(
                moved,
                self.align_constraint,
            ))
        };

        self.nudge_align_layer(drag.layer, step);
        ctx.request_repaint();
        true
    }

    /// Apply one drag step directly to the scene, without touching history.
    ///
    /// History is written once at release: a drag is one operator gesture, and
    /// filling the undo stack with a hundred per-frame steps would make Ctrl+Z
    /// useless.
    fn nudge_align_layer(&mut self, layer: SceneMeshId, step: Affine3A) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let mut next = scene.as_ref().clone();
        if let Some(entry) = next
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == layer)
        {
            entry.transform = step * entry.transform;
        }
        self.set_scene(next, false);
    }

    /// Close an open drag, recording the whole gesture as one undo step.
    fn finish_align_drag(&mut self) -> bool {
        let Some(drag) = self.align_drag.take() else {
            return false;
        };
        let Some(scene) = self.scene.clone() else {
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
        let Some(token) =
            self.edit_mode
                .begin_scene_edit(&before, drag.layer, EditModeCommand::MoveLayer)
        else {
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
        self.edit_mode.finish_scene_edit_success(token, &after);
        self.set_scene(after, false);
        self.align_status = Some("Moved by hand (Ctrl+Z undoes)".into());
        self.measure_if_shown();
        true
    }
}
