//! One frame of the sculpt pointer gesture.

use std::collections::VecDeque;
use std::sync::Arc;

use super::super::{egui, SceneContext};
use super::input::{
    collect_sculpt_pointer_events, pointer_changed, SculptDispatchResult, SculptEventState,
    SculptFrameState, SculptPointerEvent, SculptPointerInput, SculptRaySample,
    SculptSampleAdmission, SculptTargetRaySample,
};
use crate::sculpt::sculpt_kernel::BrushRayStep;
use crate::sculpt::sculpt_tool::{PendingSculptPress, StrokeState, HOLD_DAB_INTERVAL_SEC};

impl SceneContext<'_> {
    /// One frame of the sculpt gesture. Returns `true` only while the primary
    /// button drives a sculpt this frame, so RMB orbit / MMB / wheel keep
    /// working with a brush armed.
    pub(in crate::app) fn handle_sculpt_drag(
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
            self.tools.sculpt.pending_presses.clear();
            if !self.commit_sculpt_stroke(ctx) {
                return true;
            }
            return false;
        }

        let previous_modifiers = self.tools.sculpt.pointer_modifiers;
        let (pointer_events, down, pointer, dt, frame_modifiers) = ctx.input(|input| {
            let mut pointer_events = collect_sculpt_pointer_events(
                &input.raw.events,
                previous_modifiers,
                input.modifiers,
            );
            let has_press_event = pointer_events
                .iter()
                .any(|event| matches!(event, SculptPointerEvent::PrimaryButton(_, true, _)));
            // Synthetic input sources sometimes update PointerState
            // without emitting a raw PointerButton event.
            if input.pointer.button_pressed(egui::PointerButton::Primary) && !has_press_event {
                if let Some(position) = input.pointer.interact_pos() {
                    pointer_events.insert(
                        0,
                        SculptPointerEvent::PrimaryButton(position, true, input.modifiers),
                    );
                }
            }
            (
                pointer_events,
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.interact_pos(),
                input.stable_dt,
                input.modifiers,
            )
        });
        self.tools.sculpt.pointer_modifiers = frame_modifiers;
        let viewport_pointer =
            pointer.filter(|point| self.viewport_press_owned(ctx, response, *point));
        let dispatch = self.dispatch_sculpt_pointer_events(SculptPointerInput {
            ctx,
            response,
            kind,
            down,
            events: pointer_events,
        });
        let frame = SculptFrameState {
            kind,
            down,
            viewport_pointer,
            modifiers: frame_modifiers,
            dt,
            sampled_active_event: dispatch.sampled_active_event,
            pressed: dispatch.pressed,
        };
        self.update_pending_sculpt_pointer(ctx, response, frame);

        if frame.pressed && self.tools.sculpt.stroke.is_some() && !self.commit_sculpt_stroke(ctx) {
            return true;
        }
        if self.process_next_pending_sculpt_press(ctx, response) {
            return true;
        }
        self.finish_sculpt_frame(ctx, response, frame)
    }

    fn update_pending_sculpt_pointer(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        frame: SculptFrameState,
    ) {
        if frame.down && frame.viewport_pointer.is_none() {
            self.break_pending_or_active_sculpt_path();
        }

        // The press ray remains immutable; only the eventual endpoint follows
        // camera and brush changes while preparation is pending.
        let pending_index = self
            .tools
            .sculpt
            .pending_presses
            .iter()
            .rposition(|pending| !pending.released);
        let Some(index) = pending_index else {
            return;
        };
        let Some(point) = frame.viewport_pointer else {
            if !frame.down {
                if let Some(pending) = self.tools.sculpt.pending_presses.get_mut(index) {
                    pending.released = true;
                }
            }
            return;
        };
        let update = self
            .tools
            .sculpt
            .pending_presses
            .get(index)
            .filter(|pending| pointer_changed(pending.latest_pointer, [point.x, point.y]))
            .and_then(|pending| {
                self.sculpt_pending_ray_step(
                    ctx,
                    SculptTargetRaySample {
                        sample: SculptRaySample::new(
                            response.rect,
                            point,
                            frame.kind,
                            frame.modifiers,
                            false,
                        ),
                        world_to_local: pending.world_to_local,
                        local_per_world: pending.local_per_world,
                    },
                )
            });
        if let Some(pending) = self.tools.sculpt.pending_presses.get_mut(index) {
            if let (Some(point), Some(step)) = (frame.viewport_pointer, update) {
                pending.latest_pointer = [point.x, point.y];
                pending.latest_step = step;
                pending.moved = true;
            }
            if !frame.down {
                pending.released = true;
            }
        }
    }

    fn process_next_pending_sculpt_press(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) -> bool {
        let Some(pending) = self.tools.sculpt.pending_presses.pop_front() else {
            return false;
        };
        self.start_pending_sculpt_press(ctx, response, pending);
        true
    }

    fn start_pending_sculpt_press(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pending: PendingSculptPress,
    ) {
        let Some(scene) = self.document.scene.as_ref() else {
            self.tools.sculpt.pending_presses.clear();
            return;
        };
        let Some((index, entry)) = scene
            .meshes()
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.id() == pending.layer_id && entry.visible)
        else {
            self.tools.sculpt.pending_presses.clear();
            return;
        };
        if self.sculpt_target_layer_id(scene) != Some(pending.layer_id)
            || entry.mesh.topology_id() != pending.topology_id
            || entry.transform.inverse() != pending.world_to_local
        {
            self.tools.sculpt.pending_presses.clear();
            return;
        }
        if self.tools.sculpt.pending_history.is_some() || self.tools.sculpt.finish_requested {
            self.tools.sculpt.pending_presses.push_front(pending);
            ctx.request_repaint();
            return;
        }

        let scene = Arc::clone(scene);
        let session_ready = self.ensure_sculpt_session_for_layer(&scene, index, pending.layer_id);
        let worker_ready = self.tools.sculpt.worker.as_ref().is_some_and(|worker| {
            worker.layer_id == pending.layer_id
                && worker.topology_id == pending.topology_id
                && worker.world_to_local == pending.world_to_local
                && worker.is_quiescent()
        });
        if !session_ready || !worker_ready {
            let can_still_prepare = self
                .tools
                .sculpt
                .pending_matches(pending.layer_id, pending.topology_id)
                || self.tools.sculpt.preparation_in_progress()
                || self.tools.sculpt.worker_has_pending_work();
            if !session_ready && !can_still_prepare {
                self.tools.sculpt.pending_presses.clear();
                self.scene_ui.status_message =
                    Some(self.ui.locale.tr(crate::i18n::message_id!("sculpt-failed")));
                return;
            }
            self.tools.sculpt.pending_presses.push_front(pending);
            if self.tools.sculpt.worker.is_none() {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("sculpt-preparing")),
                );
            }
            ctx.request_repaint();
            return;
        }

        self.submit_pending_sculpt_start(ctx, response, pending);
    }

    fn submit_pending_sculpt_start(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pending: PendingSculptPress,
    ) {
        if let Some(hit) = self.sculpt_surface_hit(
            response.rect,
            egui::pos2(pending.latest_pointer[0], pending.latest_pointer[1]),
        ) {
            if hit.layer_id == pending.layer_id {
                self.tools
                    .sculpt
                    .set_cursor_hit(pending.latest_pointer, hit);
            }
        }
        let start_step = pending.start_step.clone();
        let accepted = self
            .tools
            .sculpt
            .worker
            .as_ref()
            .is_some_and(|worker| worker.try_apply_ray_step(start_step.clone()));
        if !accepted {
            self.tools.sculpt.pending_presses.push_front(pending);
            ctx.request_repaint();
            return;
        }
        self.tools.sculpt.stroke = Some(StrokeState {
            layer_id: pending.layer_id,
            last_pointer: pending.press_pointer,
            input_pointer: pending.press_pointer,
            last_ray: Some(start_step),
            hold_seconds: 0.0,
            path_break_pending: pending.break_before_latest && (pending.moved || !pending.released),
            release_pending: false,
            retained_samples: VecDeque::new(),
        });
        self.document.unsaved_sculpt_stroke = true;

        let endpoint_accepted = if pending.moved {
            let accepted = self
                .submit_retained_sculpt_sample(pending.latest_step.clone(), pending.latest_pointer)
                != SculptSampleAdmission::Overflow;
            if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                stroke.release_pending = pending.released && !accepted;
            }
            accepted
        } else {
            true
        };
        if pending.released && endpoint_accepted {
            let _ = self.commit_sculpt_stroke(ctx);
        }
        self.render.invalidation.request_redraw();
        ctx.request_repaint();
    }

    fn finish_sculpt_frame(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        frame: SculptFrameState,
    ) -> bool {
        if !frame.down {
            return self.finish_released_sculpt_frame(ctx, response, frame);
        }
        let Some(pointer) = frame.viewport_pointer else {
            if self.tools.sculpt.stroke.is_some() {
                self.break_pending_or_active_sculpt_path();
                ctx.request_repaint();
                return true;
            }
            return false;
        };
        if frame.sampled_active_event {
            ctx.request_repaint();
            return true;
        }
        let Some(stroke_layer) = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .map(|stroke| stroke.layer_id)
        else {
            // A held button without its recorded press cannot start a stroke.
            ctx.request_repaint();
            return true;
        };
        let target_layer = self
            .document
            .scene
            .as_ref()
            .and_then(|scene| self.sculpt_target_layer_id(scene));
        if target_layer != Some(stroke_layer) {
            self.abort_sculpt_stroke();
            return true;
        }
        if let Some(hit) = self.sculpt_surface_hit(response.rect, pointer) {
            if hit.layer_id == stroke_layer {
                self.tools
                    .sculpt
                    .set_cursor_hit([pointer.x, pointer.y], hit);
            }
        }
        if !self.flush_retained_sculpt_samples() {
            ctx.request_repaint();
            return true;
        }
        self.submit_sculpt_hold_or_move(ctx, response, pointer, frame)
    }

    fn submit_sculpt_hold_or_move(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pointer: egui::Pos2,
        frame: SculptFrameState,
    ) -> bool {
        let (input_pointer, path_break_pending, previous_hold_seconds) =
            self.tools.sculpt.stroke.as_ref().map_or(
                ([pointer.x, pointer.y], false, 0.0),
                |stroke| {
                    (
                        stroke.input_pointer,
                        stroke.path_break_pending,
                        stroke.hold_seconds,
                    )
                },
            );
        let is_moving =
            path_break_pending || pointer_changed(input_pointer, [pointer.x, pointer.y]);
        let hold_seconds = if is_moving {
            0.0
        } else {
            (previous_hold_seconds + frame.dt.max(0.0)).min(0.12)
        };
        if !is_moving && hold_seconds < HOLD_DAB_INTERVAL_SEC {
            if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                stroke.hold_seconds = hold_seconds;
            }
            ctx.request_repaint();
            return true;
        }
        let sample = SculptRaySample::new(
            response.rect,
            pointer,
            frame.kind,
            frame.modifiers,
            !is_moving,
        );
        if let Some(step) = self.sculpt_pointer_ray_step(ctx, sample) {
            let admission = self.submit_retained_sculpt_sample(step, [pointer.x, pointer.y]);
            if admission != SculptSampleAdmission::Overflow {
                self.render.invalidation.request_redraw();
            } else if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                stroke.hold_seconds = hold_seconds;
            }
        }
        ctx.request_repaint();
        true
    }

    fn finish_released_sculpt_frame(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        frame: SculptFrameState,
    ) -> bool {
        let Some((release_pending, path_break_pending)) = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .map(|stroke| (stroke.release_pending, stroke.path_break_pending))
        else {
            return false;
        };
        if release_pending {
            // The physical release owns the final drain and its retry flag.
            let _ = self.commit_sculpt_stroke(ctx);
            return true;
        }
        if let Some(pointer) = frame.viewport_pointer.filter(|pointer| {
            path_break_pending
                || self.tools.sculpt.stroke.as_ref().is_some_and(|stroke| {
                    pointer_changed(stroke.input_pointer, [pointer.x, pointer.y])
                })
        }) {
            let sample =
                SculptRaySample::new(response.rect, pointer, frame.kind, frame.modifiers, false);
            if let Some(step) = self.sculpt_pointer_ray_step(ctx, sample) {
                let admission = self.submit_retained_sculpt_sample(step, [pointer.x, pointer.y]);
                if admission == SculptSampleAdmission::Overflow {
                    ctx.request_repaint();
                    return true;
                }
                if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
                    stroke.release_pending = true;
                }
            }
        }
        let _ = self.commit_sculpt_stroke(ctx);
        true
    }

    fn dispatch_sculpt_pointer_events(
        &mut self,
        input: SculptPointerInput<'_>,
    ) -> SculptDispatchResult {
        let mut button_down_for_events = input.down;
        for event in input.events.iter().rev() {
            if let SculptPointerEvent::PrimaryButton(_, pressed, _) = event {
                button_down_for_events = !pressed;
            }
        }
        let mut state = SculptEventState {
            button_down_for_events,
            active_drag_sampling: self.tools.sculpt.stroke.is_some(),
            progress: SculptDispatchResult::default(),
        };
        for event in input.events.iter().copied() {
            self.dispatch_sculpt_pointer_event(&input, event, &mut state);
        }
        state.progress
    }

    fn dispatch_sculpt_pointer_event(
        &mut self,
        input: &SculptPointerInput<'_>,
        event: SculptPointerEvent,
        state: &mut SculptEventState,
    ) {
        match event {
            SculptPointerEvent::Moved(point, modifiers) => {
                self.handle_sculpt_pointer_move(input, point, modifiers, state);
            }
            SculptPointerEvent::PrimaryButton(point, true, modifiers) => {
                self.handle_sculpt_primary_press(input, point, modifiers, state);
            }
            SculptPointerEvent::PrimaryButton(point, false, modifiers) => {
                self.handle_sculpt_primary_release(input, point, modifiers, state);
            }
        }
    }

    fn handle_sculpt_pointer_move(
        &mut self,
        input: &SculptPointerInput<'_>,
        point: egui::Pos2,
        modifiers: egui::Modifiers,
        state: &mut SculptEventState,
    ) {
        if !self.viewport_press_owned(input.ctx, input.response, point) {
            self.break_pending_or_active_sculpt_path();
            return;
        }
        let Some(index) = self
            .tools
            .sculpt
            .pending_presses
            .iter()
            .rposition(|pending| !pending.released)
        else {
            if state.button_down_for_events && state.active_drag_sampling {
                let sample =
                    SculptRaySample::new(input.response.rect, point, input.kind, modifiers, false);
                state.progress.sampled_active_event =
                    self.submit_active_sculpt_sample(input.ctx, sample, !input.down)
                        || state.progress.sampled_active_event;
            }
            return;
        };
        let update = self
            .tools
            .sculpt
            .pending_presses
            .get(index)
            .filter(|pending| pointer_changed(pending.latest_pointer, [point.x, point.y]))
            .and_then(|pending| {
                self.sculpt_pending_ray_step(
                    input.ctx,
                    SculptTargetRaySample {
                        sample: SculptRaySample::new(
                            input.response.rect,
                            point,
                            input.kind,
                            modifiers,
                            false,
                        ),
                        world_to_local: pending.world_to_local,
                        local_per_world: pending.local_per_world,
                    },
                )
            });
        if let (Some(step), Some(pending)) =
            (update, self.tools.sculpt.pending_presses.get_mut(index))
        {
            pending.latest_pointer = [point.x, point.y];
            pending.latest_step = step;
            pending.moved = true;
        }
    }

    fn handle_sculpt_primary_press(
        &mut self,
        input: &SculptPointerInput<'_>,
        point: egui::Pos2,
        modifiers: egui::Modifiers,
        state: &mut SculptEventState,
    ) {
        state.button_down_for_events = true;
        state.active_drag_sampling = false;
        if !self.viewport_press_owned(input.ctx, input.response, point) {
            return;
        }
        state.progress.pressed = true;
        let sample = SculptRaySample::new(input.response.rect, point, input.kind, modifiers, false);
        let Some(pending) = self.pending_sculpt_press(input.ctx, sample) else {
            return;
        };
        if !self.tools.sculpt.queue_pending_press(pending) {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-preparing")),
            );
        }
    }

    fn handle_sculpt_primary_release(
        &mut self,
        input: &SculptPointerInput<'_>,
        point: egui::Pos2,
        modifiers: egui::Modifiers,
        state: &mut SculptEventState,
    ) {
        let was_down = state.button_down_for_events;
        state.button_down_for_events = false;
        let Some(index) = self
            .tools
            .sculpt
            .pending_presses
            .iter()
            .rposition(|pending| !pending.released)
        else {
            if was_down && state.active_drag_sampling {
                if self.viewport_press_owned(input.ctx, input.response, point) {
                    let sample = SculptRaySample::new(
                        input.response.rect,
                        point,
                        input.kind,
                        modifiers,
                        false,
                    );
                    state.progress.sampled_active_event = self
                        .submit_active_sculpt_sample(input.ctx, sample, true)
                        || state.progress.sampled_active_event;
                } else {
                    self.break_pending_or_active_sculpt_path();
                }
            }
            state.active_drag_sampling = false;
            return;
        };
        let update = self
            .viewport_press_owned(input.ctx, input.response, point)
            .then(|| self.tools.sculpt.pending_presses.get(index))
            .flatten()
            .filter(|pending| pointer_changed(pending.latest_pointer, [point.x, point.y]))
            .and_then(|pending| {
                self.sculpt_pending_ray_step(
                    input.ctx,
                    SculptTargetRaySample {
                        sample: SculptRaySample::new(
                            input.response.rect,
                            point,
                            input.kind,
                            modifiers,
                            false,
                        ),
                        world_to_local: pending.world_to_local,
                        local_per_world: pending.local_per_world,
                    },
                )
            });
        if let Some(pending) = self.tools.sculpt.pending_presses.get_mut(index) {
            if let Some(step) = update {
                pending.latest_pointer = [point.x, point.y];
                pending.latest_step = step;
                pending.moved = true;
            }
            pending.released = true;
        }
        state.active_drag_sampling = false;
    }

    fn break_pending_or_active_sculpt_path(&mut self) {
        if let Some(pending) = self
            .tools
            .sculpt
            .pending_presses
            .iter_mut()
            .rfind(|pending| !pending.released)
        {
            pending.break_before_latest = true;
        } else if let Some(stroke) = self.tools.sculpt.stroke.as_mut() {
            stroke.path_break_pending = true;
        }
    }

    pub(super) fn sculpt_pointer_ray_step(
        &self,
        ctx: &egui::Context,
        sample: SculptRaySample,
    ) -> Option<BrushRayStep> {
        let worker = self.tools.sculpt.worker.as_ref()?;
        self.sculpt_pending_ray_step(
            ctx,
            SculptTargetRaySample {
                sample,
                world_to_local: worker.world_to_local,
                local_per_world: worker.local_per_world,
            },
        )
    }
}
