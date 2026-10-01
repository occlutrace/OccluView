//! Viewport input and rendering integration for the sculpt brushes.

pub(super) mod stroke;
pub(super) mod worker;
#[cfg(test)]
mod abort_tests;
#[cfg(test)]
mod characterization_tests;
#[cfg(test)]
mod lifecycle_tests;

pub(super) use self::stroke::apply_sculpt_wheel_settings;
use self::stroke::{
    local_brush_ray_step, local_clip_plane, local_ray_hit_is_visible, LocalBrushRayInput,
};
use super::{egui, live_viewport, mesh_editor_overlay, SceneContext};
use crate::app::workspace::id::SceneKey;
use crate::sculpt::sculpt_kernel::{BrushMode, BrushRayStep};
// Test-only re-export: sibling test modules build dabs through `super::`.
#[cfg(test)]
use crate::sculpt::sculpt_kernel::BrushStroke;
use crate::sculpt::sculpt_tool::{
    uniform_scene_scale, PendingSculptPress, RetainedSculptSample, SculptTip, SculptToolKind,
    StrokeState, HOLD_DAB_INTERVAL_SEC,
};
use crate::sculpt::sculpt_worker::SculptWorker;
use crate::viewer::viewport_ray;
use glam::{Affine3A, DVec3, Mat4, Quat, Vec3};
use occluview_core::{SceneMeshId, ScenePickHit};
use occluview_render::{
    sculpt_surface_light_intensity, sculpt_tool_length, PreparedSceneTopology, SculptBrushUniform,
    SculptFeedbackStyle, SculptToolShape, SculptToolUniform,
};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum SculptPointerEvent {
    Moved(egui::Pos2, egui::Modifiers),
    PrimaryButton(egui::Pos2, bool, egui::Modifiers),
}

#[derive(Clone, Copy)]
struct SculptRaySample {
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    kind: SculptToolKind,
    shift: bool,
    command: bool,
    hold: bool,
}

impl SculptRaySample {
    fn new(
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
        kind: SculptToolKind,
        modifiers: egui::Modifiers,
        hold: bool,
    ) -> Self {
        Self {
            viewport_rect,
            pointer,
            kind,
            shift: modifiers.shift,
            command: modifiers.ctrl || modifiers.command,
            hold,
        }
    }
}

#[derive(Clone, Copy)]
struct SculptTargetRaySample {
    sample: SculptRaySample,
    world_to_local: Affine3A,
    local_per_world: f32,
}

struct SculptPointerInput<'a> {
    ctx: &'a egui::Context,
    response: &'a egui::Response,
    kind: SculptToolKind,
    down: bool,
    events: Vec<SculptPointerEvent>,
}

#[derive(Default)]
struct SculptDispatchResult {
    sampled_active_event: bool,
    pressed: bool,
}

#[derive(Clone, Copy)]
struct SculptFrameState {
    kind: SculptToolKind,
    down: bool,
    viewport_pointer: Option<egui::Pos2>,
    modifiers: egui::Modifiers,
    dt: f32,
    sampled_active_event: bool,
    pressed: bool,
}

struct SculptEventState {
    button_down_for_events: bool,
    active_drag_sampling: bool,
    progress: SculptDispatchResult,
}

// Pointer positions are viewport coordinates. Exact comparison preserves all
// captured samples; an epsilon could silently skip a distinct pointer event.
#[allow(clippy::float_cmp)]
fn pointer_changed(previous: [f32; 2], current: [f32; 2]) -> bool {
    previous != current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SculptSampleAdmission {
    Queued,
    Retained,
    Overflow,
}

fn collect_sculpt_pointer_events(
    events: &[egui::Event],
    previous_modifiers: egui::Modifiers,
    frame_modifiers: egui::Modifiers,
) -> Vec<SculptPointerEvent> {
    let has_modifier_event = events
        .iter()
        .any(|event| matches!(event, egui::Event::ModifiersChanged(_)));
    let mut modifiers = if has_modifier_event {
        previous_modifiers
    } else {
        frame_modifiers
    };
    let mut pointer_events = Vec::new();
    for event in events {
        match event {
            egui::Event::ModifiersChanged(next) => modifiers = *next,
            egui::Event::PointerMoved(position) => {
                pointer_events.push(SculptPointerEvent::Moved(*position, modifiers));
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: button_modifiers,
            } => {
                modifiers = *button_modifiers;
                pointer_events.push(SculptPointerEvent::PrimaryButton(
                    *pos,
                    *pressed,
                    *button_modifiers,
                ));
            }
            _ => {}
        }
    }
    pointer_events
}

impl SceneContext<'_> {
    /// Arm/disarm a sculpt tool (toggling the armed one disarms).
    pub(super) fn toggle_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        // Finish a live stroke before switching brush modes. Other context
        // switches use their existing abort paths.
        if !self.commit_sculpt_stroke(ctx) {
            return;
        }
        self.tools.sculpt.toggle(kind);
        if self.tools.sculpt.armed.is_some() {
            if self.persistence.settings.last_sculpt_tool != kind {
                self.persistence.settings.last_sculpt_tool = kind;
                if self.persistence.settings.remember_sculpt_brush {
                    self.persistence.settings_persistence.mark_dirty();
                }
            }
            // Arming a brush means the Sculpt tab: show it and drop selection.
            self.tools.editor_tab = mesh_editor_overlay::EditorTab::Sculpt;
            self.document.mesh_selection_drag = None;
            // Prepare the target off the UI thread. Sculpt remains the owner of
            // the primary button while armed.
            self.prepare_armed_sculpt_session();
        } else if !self.tools.sculpt.worker_has_pending_work() {
            // The worker owns a queued Finish until the next poll.
            self.tools.sculpt.disarm();
        }
        self.scene_ui.status_message = Some(match self.tools.sculpt.armed {
            Some(SculptToolKind::AddRemove) if self.tools.sculpt.worker.is_some() => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-armed-addremove")),
            Some(SculptToolKind::Smooth) if self.tools.sculpt.worker.is_some() => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-armed-smooth")),
            Some(_) => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-preparing")),
            None => self.ui.locale.tr(crate::i18n::message_id!("sculpt-off")),
        });
        // Rebuild selection display data when Sculpt changes the visible mesh.
        self.render.invalidation.selection_changed();
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
                self.toggle_sculpt_tool(self.persistence.settings.last_sculpt_tool, ctx);
            }
            EditorTab::Sculpt => {}
        }
        self.render.invalidation.selection_changed();
        ctx.request_repaint();
    }

    /// In Mesh Editor, `1` opens Sculpt with Add/Remove and `2` with Smooth.
    /// Text fields retain digit keys.
    pub(super) fn handle_sculpt_hotkeys(&mut self, ctx: &egui::Context) -> bool {
        if !self.document.edit_mode.has_active_session() || ctx.egui_wants_keyboard_input() {
            return false;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                // Do not let Shift change the meaning of the mode switch.
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

    /// Arm a sculpt tool idempotently — the hotkey only turns a tool on.
    fn arm_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        if self.tools.sculpt.armed != Some(kind) {
            self.toggle_sculpt_tool(kind, ctx);
        }
    }

    /// One frame of the sculpt gesture. Returns `true` only while the primary
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

    fn sculpt_pointer_ray_step(
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

    /// Retry captured input in order. A discontinuity belongs to the first
    /// sample after the gap and is consumed before that sample reaches the
    /// worker.
    pub(super) fn flush_retained_sculpt_samples(&mut self) -> bool {
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
    fn submit_retained_sculpt_sample(
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
    fn submit_active_sculpt_sample(
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

    fn pending_sculpt_press(
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

    fn sculpt_pending_ray_step(
        &self,
        ctx: &egui::Context,
        target: SculptTargetRaySample,
    ) -> Option<BrushRayStep> {
        let SculptTargetRaySample {
            sample,
            world_to_local,
            local_per_world,
        } = target;
        let camera = *self.render.camera.as_ref()?;
        let scene = self.document.scene.as_ref()?;
        let clip_plane = self.active_viewport_clip_plane(scene.bbox());
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
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-nonuniform-scale")),
            );
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
                self.scene_ui.status_message = None;
            } else if scene
                .meshes()
                .get(index)
                .is_some_and(|entry| uniform_scene_scale(&entry.transform).is_none())
            {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("sculpt-nonuniform-scale")),
                );
            } else {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("sculpt-preparing")),
                );
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
                        self.scene_ui.status_message = None;
                    }
                    self.render.invalidation.overlay_tools_changed();
                    ctx.request_repaint();
                } else {
                    self.tools.sculpt.pending_presses.clear();
                }
            }
            Err(error) => {
                self.tools.sculpt.pending_presses.clear();
                self.scene_ui.status_message = Some(self.ui.locale.tr_with(
                    crate::i18n::message_id!("sculpt-failed"),
                    &[("detail", error.as_str())],
                ));
                ctx.request_repaint();
            }
        }
    }

    pub(super) fn sculpt_has_live_work(&self) -> bool {
        self.document.unsaved_sculpt_stroke
    }

    /// Drop any in-flight stroke. If it had uncommitted dabs on the GPU, drop
    /// the persistent session too and force a full re-sync so the on-screen
    /// geometry reverts to the committed scene.
    pub(super) fn abort_sculpt_stroke(&mut self) {
        self.tools.sculpt.pending_presses.clear();
        let had_stroke = self.tools.sculpt.stroke.take().is_some();
        let had_pending = self.tools.sculpt.worker_has_pending_work();
        if had_stroke || had_pending {
            self.invalidate_sculpt_session_silent();
        }
    }

    pub(super) fn invalidate_sculpt_session_silent(&mut self) {
        self.document.unsaved_sculpt_stroke = false;
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
        response: &egui::Response,
    ) -> bool {
        let over_viewport = ctx
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|point| self.viewport_press_owned(ctx, response, point));
        let Some(kind) = self.tools.sculpt.armed else {
            return false;
        };
        if !over_viewport || !self.document.edit_mode.has_active_session() {
            return false;
        }
        // Keep the existing camera-wheel behavior during a pointer gesture.
        if self.tools.sculpt.stroke.is_some() || !self.tools.sculpt.pending_presses.is_empty() {
            return false;
        }
        // A released stroke can still be draining its ordered Finish command.
        // Consume modified wheel events during that interval without changing
        // captured brush parameters or zooming the view under pending history.
        if self.tools.sculpt.is_busy() {
            return stroke::has_sculpt_settings_wheel(ctx, self.scene_key);
        }
        if !apply_sculpt_wheel_settings(ctx, self.scene_key, Some(kind)) {
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
        let (ray_origin, direction) = viewport_ray(&camera, viewport_rect, pointer)?;
        let direction = direction.normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let origin = ray_origin + direction * camera.near;
        let inverse = entry.transform.inverse();
        let local_origin = inverse.transform_point3(origin);
        let local_direction = inverse.transform_vector3(direction).normalize_or_zero();
        let worker = self.tools.sculpt.worker.as_ref().filter(|worker| {
            worker.layer_id == layer_id && worker.topology_id == entry.mesh.topology_id()
        });
        let local_per_world = worker.map_or_else(
            || Some(1.0 / uniform_scene_scale(&entry.transform)?),
            |w| Some(w.local_per_world),
        )?;
        let far_mm = (camera.far - camera.near) * local_per_world;
        let clip_plane = local_clip_plane(inverse, self.active_viewport_clip_plane(scene.bbox()));
        let keep = |point| {
            local_ray_hit_is_visible(point, local_origin, local_direction, far_mm, clip_plane)
        };
        // Preparation warms the shared tree off the UI thread.
        if worker.is_none() && !entry.mesh.bvh_is_ready() {
            return None;
        }
        // When the live tree is contended, defer this cursor frame instead of
        // showing a footprint on committed geometry under the live surface.
        let (triangle_index, local_point) = if let Some(worker) = worker {
            worker.pick_local_ray(local_origin, local_direction, keep)?
        } else {
            entry
                .mesh
                .pick_ray_local(local_origin, local_direction, keep)?
        };
        let point = entry.transform.transform_point3(local_point);
        let distance = (point - origin).dot(direction);
        (distance.is_finite() && distance >= 0.0 && distance <= camera.far - camera.near).then_some(
            ScenePickHit {
                layer_index: scene
                    .meshes()
                    .iter()
                    .position(|candidate| candidate.id() == layer_id)?,
                layer_id,
                triangle_index,
                point,
                distance,
            },
        )
    }

    fn sculpt_target_layer_id(&self, scene: &occluview_core::Scene) -> Option<SceneMeshId> {
        sculpt_target(scene, self.document.edit_mode.session_layer_id())
            .map(|(_, layer_id)| layer_id)
    }

    /// Paint the cursor using the hit cached by the viewport input pass.
    #[expect(clippy::too_many_lines)]
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
        // Match cursor ownership to the drag and wait for preparation to finish.
        if !viewport_response.contains_pointer() {
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
        let (shift, command) = ui.ctx().input(|input| {
            (
                input.modifiers.shift,
                input.modifiers.ctrl || input.modifiers.command,
            )
        });
        let tip = mesh_editor_overlay::sculpt_tip(ui.ctx(), self.scene_key);
        let radius_world = mesh_editor_overlay::sculpt_radius_mm(ui.ctx(), self.scene_key, tip);
        let strength_setting = mesh_editor_overlay::sculpt_strength(ui.ctx(), self.scene_key, kind);
        let mode = kind.brush_mode(shift, command);
        let color =
            animate_sculpt_cursor_color(ui.ctx(), self.scene_key, sculpt_cursor_color(mode));
        let strength = kind.dab_strength(strength_setting, shift);
        if mode == BrushMode::Remove && self.tools.sculpt.stroke.is_none() {
            if let Some(worker) = self.tools.sculpt.worker.as_ref().filter(|worker| {
                worker.layer_id == hit.layer_id && worker.topology_id == entry.mesh.topology_id()
            }) {
                let local = worker.world_to_local.transform_point3(hit.point);
                let center = DVec3::new(f64::from(local.x), f64::from(local.y), f64::from(local.z));
                let radius_mm = f64::from(radius_world * worker.local_per_world);
                let _ = worker.try_prime_wall_region(center, radius_mm, 128);
            }
        }
        let action = animate_sculpt_cursor_action(ui.ctx(), self.scene_key, mode);
        let shape = match tip {
            SculptTip::Ball => SculptToolShape::Cone,
            SculptTip::Knife => SculptToolShape::Knife,
            SculptTip::Cylinder => SculptToolShape::Cylinder,
        };
        let axis = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .and_then(|stroke| stroke.last_ray.as_ref())
            .and_then(|step| step.axis);
        let axis_world = axis
            .map(|axis| entry.transform.transform_vector3(Vec3::from_array(axis)))
            .filter(|axis| axis.is_finite() && axis.length_squared() > f32::EPSILON)
            .map(Vec3::normalize);
        let color_rgba = sculpt_cursor_linear_rgba(color);
        let direction = normal;
        let base_rotation = Quat::from_rotation_arc(Vec3::Z, direction);
        let tool_rotation = if tip == SculptTip::Knife {
            let fallback_axis = camera
                .view_direction()
                .cross(camera.view_up())
                .normalize_or_zero();
            orient_tool_axis(
                base_rotation,
                direction,
                axis_world.unwrap_or(fallback_axis),
            )
        } else {
            base_rotation
        };
        let tool_width = radius_world;
        let target_height = sculpt_cursor_height(mode, strength, radius_world);
        let tool_length = animate_sculpt_cursor_height(ui.ctx(), self.scene_key, target_height);
        let tool_model = Mat4::from_scale_rotation_translation(
            Vec3::new(tool_width, radius_world, tool_length),
            tool_rotation,
            hit.point + direction * 0.02,
        );
        self.publish_sculpt_cursor(Some(live_viewport::SculptCursor {
            target_index: hit.layer_index,
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            brush: SculptBrushUniform {
                center: hit.point.to_array(),
                radius: radius_world,
                normal: normal.to_array(),
                intensity: sculpt_surface_light_intensity(strength),
                axis: axis_world.map_or([0.0; 3], |axis| axis.to_array()),
                tip: tip.kernel_stamp(),
                color: color_rgba,
                visible: 1,
                edge_style: if mode == BrushMode::Relax {
                    SculptFeedbackStyle::Dashed as u32
                } else {
                    SculptFeedbackStyle::Solid as u32
                },
                ..SculptBrushUniform::hidden()
            },
            tool: SculptToolUniform {
                model: tool_model.to_cols_array(),
                color: color_rgba,
                // One fixed body opacity: the strength signal lives in the
                // surface mark, not in how dense the tool body looks.
                opacity: SCULPT_TOOL_OPACITY,
                shape: shape as u32,
                action,
            },
        }));
        // A CAD crosshair replaces the arrow while a sculpt tool is armed, so
        // the contact point is readable against the surface mark.
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        let ortho_height = camera.orthographic_height.max(f32::EPSILON);
        let radius_px = radius_world * viewport_rect.height() / ortho_height;
        if radius_px.is_finite() && radius_px >= 2.0 {
            let canvas = ui.painter();
            let intensity = strength;
            // A hairline circle only: a filled disc at small radii read as a
            // blob covering the very surface the operator is judging.
            let edge_color = color.gamma_multiply(0.58 + intensity * 0.18);
            if mode == BrushMode::Relax {
                paint_dashed_cursor_edge(canvas, pointer, radius_px, edge_color);
            } else {
                canvas.circle_stroke(pointer, radius_px, egui::Stroke::new(1.0_f32, edge_color));
            }
            // A cross marks the centre without hiding it.
            let cross = color.gamma_multiply(0.62);
            let stroke = egui::Stroke::new(1.0_f32, cross);
            canvas.line_segment(
                [
                    pointer + egui::vec2(-3.0, 0.0),
                    pointer + egui::vec2(3.0, 0.0),
                ],
                stroke,
            );
            canvas.line_segment(
                [
                    pointer + egui::vec2(0.0, -3.0),
                    pointer + egui::vec2(0.0, 3.0),
                ],
                stroke,
            );
        }
    }

    pub(in crate::app) fn publish_sculpt_cursor(
        &self,
        cursor: Option<live_viewport::SculptCursor>,
    ) {
        let Some(viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        if let Ok(mut viewport) = viewport.lock() {
            viewport.set_sculpt_cursor(cursor);
        }
    }
}

fn orient_tool_axis(base: Quat, surface_normal: Vec3, requested_axis: Vec3) -> Quat {
    let target =
        (requested_axis - surface_normal * requested_axis.dot(surface_normal)).normalize_or_zero();
    let current = (base * Vec3::Y).normalize_or_zero();
    if !target.is_finite()
        || target.length_squared() <= f32::EPSILON
        || current.length_squared() <= f32::EPSILON
    {
        return base;
    }
    let angle = surface_normal
        .dot(current.cross(target))
        .atan2(current.dot(target));
    Quat::from_axis_angle(surface_normal, angle) * base
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

/// Keep the four surface operations visually distinct.
const SCULPT_CURSOR_TRANSITION_SEC: f32 = 0.07;
const SCULPT_IRON_HEIGHT_SHARE: f32 = 0.22;
/// Source opacity of the translucent tool body. The fragment shader shapes it
/// with the Fresnel rim and the axial fade; the strength signal stays on the
/// surface mark instead.
const SCULPT_TOOL_OPACITY: f32 = 0.5;

fn sculpt_cursor_color(mode: BrushMode) -> egui::Color32 {
    match mode {
        BrushMode::Add => egui::Color32::from_rgb(22, 136, 74),
        BrushMode::Remove => egui::Color32::from_rgb(194, 58, 46),
        BrushMode::Relax => egui::Color32::from_rgb(43, 102, 177),
        BrushMode::Smooth => egui::Color32::from_rgb(60, 67, 72),
    }
}

/// Brush colour for the surface footprint and the tool body, in linear space.
///
/// The UI ink colours are dark once converted to linear, and a surface light
/// needs a pale tint so the tool colour does not erase the strength signal.
/// Every brush colour is therefore washed a fixed share toward white, which is
/// what keeps the footprint a pale mark on a bright surface instead of a
/// saturated dark one that reads as damage.
fn sculpt_cursor_linear_rgba(color: egui::Color32) -> [f32; 4] {
    /// Share of the way to white, in linear space.
    const WHITE_WASH: f32 = 0.75;
    let linear = egui::Rgba::from(color);
    let washed = |channel: f32| channel + (1.0 - channel) * WHITE_WASH;
    [
        washed(linear.r()),
        washed(linear.g()),
        washed(linear.b()),
        linear.a(),
    ]
}

fn sculpt_cursor_height(mode: BrushMode, strength: f32, radius: f32) -> f32 {
    if matches!(mode, BrushMode::Add | BrushMode::Remove) {
        sculpt_tool_length(strength)
    } else {
        (radius * SCULPT_IRON_HEIGHT_SHARE).max(0.05)
    }
}

fn animate_sculpt_cursor_height(context: &egui::Context, scene_key: SceneKey, target: f32) -> f32 {
    context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-height", scene_key)),
        target,
        SCULPT_CURSOR_TRANSITION_SEC,
    )
}

fn animate_sculpt_cursor_color(
    context: &egui::Context,
    scene_key: SceneKey,
    target: egui::Color32,
) -> egui::Color32 {
    let rgba = egui::Rgba::from(target).to_array();
    let red = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-red", scene_key)),
        rgba[0],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    let green = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-green", scene_key)),
        rgba[1],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    let blue = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-blue", scene_key)),
        rgba[2],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    egui::Rgba::from_rgba_unmultiplied(red, green, blue, 1.0).into()
}

fn animate_sculpt_cursor_action(
    context: &egui::Context,
    scene_key: SceneKey,
    mode: BrushMode,
) -> [f32; 2] {
    let target = sculpt_cursor_action(mode);
    [
        context.animate_value_with_time(
            egui::Id::new(("sculpt-cursor-invert", scene_key)),
            target[0],
            SCULPT_CURSOR_TRANSITION_SEC,
        ),
        context.animate_value_with_time(
            egui::Id::new(("sculpt-cursor-flat", scene_key)),
            target[1],
            SCULPT_CURSOR_TRANSITION_SEC,
        ),
    ]
}

fn sculpt_cursor_action(mode: BrushMode) -> [f32; 2] {
    match mode {
        BrushMode::Add => [0.0, 0.0],
        BrushMode::Remove => [1.0, 0.0],
        BrushMode::Relax | BrushMode::Smooth => [0.0, 1.0],
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "24 fixed segments are exactly representable"
)]
fn paint_dashed_cursor_edge(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    color: egui::Color32,
) {
    const DASH_COUNT: usize = 24;
    for dash in 0..DASH_COUNT {
        let start = (dash as f32 + 0.12) * TAU / DASH_COUNT as f32;
        let end = (dash as f32 + 0.68) * TAU / DASH_COUNT as f32;
        let point = |angle: f32| center + egui::vec2(radius * angle.cos(), radius * angle.sin());
        painter.line_segment([point(start), point(end)], egui::Stroke::new(1.0, color));
    }
}

#[cfg(test)]
mod cursor_color_tests {
    use super::*;

    #[test]
    fn cursor_palette_is_published_to_the_linear_gpu_uniform() {
        // Every mode is washed 75 % toward white, so even the darkest ink
        // (Smooth) leaves a pale mark that only tints the lit surface.
        let color = sculpt_cursor_linear_rgba(sculpt_cursor_color(BrushMode::Smooth));
        assert!((color[0] - 0.761).abs() < 0.002, "got {}", color[0]);
        assert!((color[1] - 0.764).abs() < 0.002, "got {}", color[1]);
        assert!((color[2] - 0.766).abs() < 0.002, "got {}", color[2]);
        assert!((color[3] - 1.0).abs() < f32::EPSILON);
    }

    /// Channel indices ordered from smallest to largest.
    fn channel_order(color: [f32; 3]) -> [usize; 3] {
        let mut indices = [0, 1, 2];
        indices.sort_by(|left, right| color[*left].total_cmp(&color[*right]));
        indices
    }

    #[test]
    fn every_cursor_colour_is_washed_and_still_names_its_mode() {
        for mode in [BrushMode::Add, BrushMode::Remove, BrushMode::Smooth] {
            let token = egui::Rgba::from(sculpt_cursor_color(mode));
            let raw = [token.r(), token.g(), token.b()];
            let washed = sculpt_cursor_linear_rgba(sculpt_cursor_color(mode));
            for (index, channel) in raw.iter().enumerate() {
                let expected = channel + (1.0 - channel) * 0.75;
                assert!(
                    (washed[index] - expected).abs() < 1e-6,
                    "the wash is three quarters toward white: {} vs {expected}",
                    washed[index]
                );
                assert!(
                    washed[index] > 0.7,
                    "a washed channel must stay pale, got {}",
                    washed[index]
                );
            }
            // The hue survives: a washed colour keeps its token's channel order,
            // so green still reads Add, red Remove and grey Smooth.
            assert_eq!(
                channel_order([washed[0], washed[1], washed[2]]),
                channel_order(raw),
                "the wash keeps the mode's channel order"
            );
        }
    }
}

#[cfg(test)]
#[path = "commit_tests.rs"]
mod commit_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
