//! Raw pointer input for one sculpt frame.

use eframe::egui;

use crate::sculpt::sculpt_tool::SculptToolKind;
use glam::Affine3A;

#[derive(Clone, Copy)]
pub(super) enum SculptPointerEvent {
    Moved(egui::Pos2, egui::Modifiers),
    PrimaryButton(egui::Pos2, bool, egui::Modifiers),
}

#[derive(Clone, Copy)]
pub(super) struct SculptRaySample {
    pub(super) viewport_rect: egui::Rect,
    pub(super) pointer: egui::Pos2,
    pub(super) kind: SculptToolKind,
    pub(super) shift: bool,
    pub(super) command: bool,
    pub(super) hold: bool,
}

impl SculptRaySample {
    pub(super) fn new(
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
pub(super) struct SculptTargetRaySample {
    pub(super) sample: SculptRaySample,
    pub(super) world_to_local: Affine3A,
    pub(super) local_per_world: f32,
}

pub(super) struct SculptPointerInput<'a> {
    pub(super) ctx: &'a egui::Context,
    pub(super) response: &'a egui::Response,
    pub(super) kind: SculptToolKind,
    pub(super) down: bool,
    pub(super) events: Vec<SculptPointerEvent>,
}

#[derive(Default)]
pub(super) struct SculptDispatchResult {
    pub(super) sampled_active_event: bool,
    pub(super) pressed: bool,
}

#[derive(Clone, Copy)]
pub(super) struct SculptFrameState {
    pub(super) kind: SculptToolKind,
    pub(super) down: bool,
    pub(super) viewport_pointer: Option<egui::Pos2>,
    pub(super) modifiers: egui::Modifiers,
    pub(super) dt: f32,
    pub(super) sampled_active_event: bool,
    pub(super) pressed: bool,
}

pub(super) struct SculptEventState {
    pub(super) button_down_for_events: bool,
    pub(super) active_drag_sampling: bool,
    pub(super) progress: SculptDispatchResult,
}

// Pointer positions are viewport coordinates. Exact comparison preserves all
// captured samples; an epsilon could silently skip a distinct pointer event.
#[allow(clippy::float_cmp)]
pub(super) fn pointer_changed(previous: [f32; 2], current: [f32; 2]) -> bool {
    previous != current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SculptSampleAdmission {
    Queued,
    Retained,
    Overflow,
}

pub(super) fn collect_sculpt_pointer_events(
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

