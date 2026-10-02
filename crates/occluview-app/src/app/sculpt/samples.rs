//! Retaining, flushing, and submitting sculpt samples.

use super::super::{egui, mesh_editor_overlay, SceneContext};
use super::geometry::sculpt_target;
use super::input::{
    pointer_changed, SculptRaySample, SculptSampleAdmission, SculptTargetRaySample,
};
use super::stroke::{local_brush_ray_step, LocalBrushRayInput};
use crate::sculpt::sculpt_kernel::BrushRayStep;
use crate::sculpt::sculpt_tool::{uniform_scene_scale, PendingSculptPress, RetainedSculptSample};

impl SceneContext<'_> {
    /// Retry captured input in order. A discontinuity belongs to the first
    /// sample after the gap and is consumed before that sample reaches the
    /// worker.
    pub(in crate::app) fn flush_retained_sculpt_samples(&mut self) -> bool {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return self
                .tools
                .sculpt
                .stroke
                .as_ref()
                .is_none_or(|stroke| stroke.retained_samples.is_empty());
        };
        let Some(stroke) = self.tools.sculpt.stroke.as_mut() else {
            return true;
        };
        loop {
            let Some(sample) = stroke.retained_samples.front() else {
                return true;
            };
            if sample.break_before {
                if !worker.try_break_ray_path() {
                    return false;
                }
                if let Some(sample) = stroke.retained_samples.front_mut() {
                    sample.break_before = false;
                }
            }
            let Some(step) = stroke
                .retained_samples
                .front()
                .map(|sample| sample.step.clone())
            else {
                return true;
            };
            if !worker.try_apply_ray_step(step) {
                return false;
            }
            let Some(sample) = stroke.retained_samples.pop_front() else {
                return true;
            };
            stroke.last_pointer = sample.pointer;
            stroke.last_ray = Some(sample.step);
            stroke.hold_seconds = 0.0;
        }
    }

    /// Admit a captured sample to the worker, retaining it locally if the
    /// worker's bounded queue refuses it. New input cannot pass a retained
    /// sample, and a full local FIFO cancels the unfinished stroke explicitly.
    pub(super) fn submit_retained_sculpt_sample(
        &mut self,
        step: BrushRayStep,
        pointer: [f32; 2],
    ) -> SculptSampleAdmission {
        let _ = self.flush_retained_sculpt_samples();
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            self.cancel_overflowed_sculpt_stroke();
            return SculptSampleAdmission::Overflow;
        };
        let retained_queue_nonempty = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .is_some_and(|stroke| !stroke.retained_samples.is_empty());
        if retained_queue_nonempty {
            let break_before = self
                .tools
                .sculpt
                .stroke
                .as_ref()
                .is_some_and(|stroke| stroke.path_break_pending);
            return self.retain_sculpt_sample(step, pointer, break_before);
        }

        let break_pending = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .is_some_and(|stroke| stroke.path_break_pending);
        if break_pending {
            if worker.try_break_ray_path() {
                if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                    stroke.path_break_pending = false;
                }
            } else {
                return self.retain_sculpt_sample(step, pointer, true);
            }
        }
        if worker.try_apply_ray_step(step.clone()) {
            if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                stroke.last_pointer = pointer;
                stroke.input_pointer = pointer;
                stroke.last_ray = Some(step);
                stroke.hold_seconds = 0.0;
            }
            SculptSampleAdmission::Queued
        } else {
            self.retain_sculpt_sample(step, pointer, false)
        }
    }

    fn retain_sculpt_sample(
        &mut self,
        step: BrushRayStep,
        pointer: [f32; 2],
        break_before: bool,
    ) -> SculptSampleAdmission {
        let retained = self.tools.sculpt.stroke.as_mut().is_some_and(|stroke| {
            let retained = stroke.retain_sample(RetainedSculptSample {
                step: step.clone(),
                pointer,
                break_before,
            });
            if retained {
                if break_before {
                    stroke.path_break_pending = false;
                }
                stroke.last_ray = Some(step);
                stroke.hold_seconds = 0.0;
            }
            retained
        });
        if retained {
            SculptSampleAdmission::Retained
        } else {
            self.cancel_overflowed_sculpt_stroke();
            SculptSampleAdmission::Overflow
        }
    }

    fn cancel_overflowed_sculpt_stroke(&mut self) {
        self.abort_sculpt_stroke();
        self.scene_ui.status_message = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-input-overflow")),
        );
    }

    /// Apply an active-drag event with the modifier state attached to that
    /// event, rather than the frame's final modifier state.
    pub(super) fn submit_active_sculpt_sample(
        &mut self,
        ctx: &egui::Context,
        sample: SculptRaySample,
        release_after_refusal: bool,
    ) -> bool {
        let Some((layer_id, input_pointer, path_break_pending)) =
            self.tools.sculpt.stroke.as_ref().map(|stroke| {
                (
                    stroke.layer_id,
                    stroke.input_pointer,
                    stroke.path_break_pending,
                )
            })
        else {
            return false;
        };
        let point = [sample.pointer.x, sample.pointer.y];
        let queue_drained = self.flush_retained_sculpt_samples();
        if !queue_drained && !path_break_pending && !pointer_changed(input_pointer, point) {
            return true;
        }
        if !path_break_pending && !pointer_changed(input_pointer, point) {
            return false;
        }
        if let Some(hit) = self.sculpt_surface_hit(sample.viewport_rect, sample.pointer) {
            if hit.layer_id == layer_id {
                self.tools.sculpt.set_cursor_hit(point, hit);
            }
        }
        let Some(step) = self.sculpt_pointer_ray_step(ctx, sample) else {
            return false;
        };
        let _admission = self.submit_retained_sculpt_sample(step.clone(), point);
        if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
            if release_after_refusal {
                stroke.release_pending = true;
            }
        }
        self.render.invalidation.request_redraw();
        true
    }

    pub(super) fn pending_sculpt_press(
        &self,
        ctx: &egui::Context,
        sample: SculptRaySample,
    ) -> Option<PendingSculptPress> {
        let scene = self.document.scene.as_ref()?;
        let (index, layer_id) = sculpt_target(scene, self.document.edit_mode.session_layer_id())?;
        let entry = scene.meshes().get(index)?;
        let scale = uniform_scene_scale(&entry.transform)?;
        let world_to_local = entry.transform.inverse();
        let local_per_world = 1.0 / scale;
        let step = self.sculpt_pending_ray_step(
            ctx,
            SculptTargetRaySample {
                sample,
                world_to_local,
                local_per_world,
            },
        )?;
        Some(PendingSculptPress {
            layer_id,
            topology_id: entry.mesh.topology_id(),
            world_to_local,
            local_per_world,
            press_pointer: [sample.pointer.x, sample.pointer.y],
            latest_pointer: [sample.pointer.x, sample.pointer.y],
            start_step: step.clone(),
            latest_step: step,
            moved: false,
            break_before_latest: false,
            released: false,
        })
    }

    pub(super) fn sculpt_pending_ray_step(
        &self,
        ctx: &egui::Context,
        target: SculptTargetRaySample,
    ) -> Option<BrushRayStep> {
        let SculptTargetRaySample {
            sample,
            world_to_local,
            local_per_world,
        } = target;
        let mut camera = *self.render.camera.as_ref()?;
        let scene = self.document.scene.as_ref()?;
        let bounds = self.effective_scene_bbox(scene);
        camera.fit_clip_planes_to_bbox(bounds);
        let clip_plane = self.active_viewport_clip_plane(bounds);
        let tip = mesh_editor_overlay::sculpt_tip(ctx, self.scene_key);
        local_brush_ray_step(LocalBrushRayInput {
            camera: &camera,
            viewport_rect: sample.viewport_rect,
            pointer: sample.pointer,
            world_to_local,
            local_per_world,
            clip_plane,
            kind: sample.kind,
            tip,
            shift: sample.shift,
            command: sample.command,
            radius_world_mm: mesh_editor_overlay::sculpt_radius_mm(ctx, self.scene_key, tip),
            strength: mesh_editor_overlay::sculpt_strength(ctx, self.scene_key, sample.kind),
            hold: sample.hold,
        })
    }
}
