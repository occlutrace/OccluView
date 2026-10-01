//! Ordered-event replay for one frame of a hand drag.
//!
//! egui can deliver a complete press, move and release inside one frame, so the
//! raw event order is authoritative and the final button state is only a
//! fallback.

use eframe::egui;

use super::{AlignDragInput, DragReplayState};
use crate::app::SceneContext;

impl SceneContext<'_> {
    /// Replay ordered egui events so modifier changes and coalesced gestures
    /// retain their original sequence.
    pub(super) fn replay_align_drag_events(
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
    pub(super) fn apply_aggregate_drag_motion(
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
}
