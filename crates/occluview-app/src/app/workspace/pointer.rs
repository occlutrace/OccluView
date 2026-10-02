//! Pointer arbitration: which pane a gesture targets and how captures end.

use eframe::egui::Rect;

use super::input::GestureOwner;
use super::pane_geometry::pane_header_rect;
use crate::app::align::drag::rollback_align_drag;
use crate::app::workspace::id::{PaneId, SceneKey};
use crate::app::workspace::input::{GestureKind, PaneTarget, PointerButtons, PressResult};
use crate::app::workspace::layout::WorkspaceLayout;
use crate::app::{egui, OccluViewApp};

/// One scene with a visible viewport, together with the rectangles it owns.
#[derive(Clone)]
pub(super) struct VisiblePane {
    pub(super) key: SceneKey,
    pub(super) pane: PaneId,
    pub(super) name: String,
    pub(super) frame: Rect,
    pub(super) canvas: Rect,
}

/// The workspace geometry one input-routing pass reads.
pub(super) struct WorkspaceInputFrame<'a> {
    pub(super) central_layer: egui::LayerId,
    pub(super) panes: &'a [VisiblePane],
    pub(super) workspace_rect: Rect,
    pub(super) divider_rect: Option<Rect>,
    pub(super) modal_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GestureCancelReason {
    Escape,
    FocusLost,
}

impl OccluViewApp {
    // Resolve one input frame in order: cancellation, capture, wheel, keyboard.
    #[allow(clippy::too_many_lines)]
    pub(super) fn route_pointer_input(
        &mut self,
        ctx: &egui::Context,
        frame: &WorkspaceInputFrame<'_>,
    ) {
        let (focused, buttons, primary_pressed, secondary_pressed, middle_pressed, wheel) = ctx
            .input(|input| {
                (
                    input.focused,
                    pointer_buttons(input),
                    input.pointer.button_pressed(egui::PointerButton::Primary),
                    input.pointer.button_pressed(egui::PointerButton::Secondary),
                    input.pointer.button_pressed(egui::PointerButton::Middle),
                    input.raw.events.iter().any(|event| {
                        matches!(event, egui::Event::MouseWheel { .. } | egui::Event::Zoom(_))
                    }),
                )
            });
        if !focused {
            if let Some(owner) = self.workspace.input.focus_lost(buttons) {
                self.cancel_captured_gesture(owner, ctx, GestureCancelReason::FocusLost);
            }
            self.workspace.layer_drag = None;
            return;
        }
        self.workspace.input.focus_gained(buttons);
        if frame.modal_open || self.ui.modal_dialog_open() {
            if let Some(owner) = self.workspace.input.escape(buttons) {
                self.cancel_captured_gesture(owner, ctx, GestureCancelReason::Escape);
            }
            self.workspace.layer_drag = None;
            return;
        }

        if let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) {
            let in_divider = frame
                .divider_rect
                .is_some_and(|divider| divider.expand(0.5).contains(pointer));
            let in_layers_panel =
                self.pointer_over_layers_panel(ctx, pointer, frame.workspace_rect);
            let in_workspace_chrome = self.pointer_over_workspace_chrome(pointer, frame.panes);
            let over_noncentral_ui = ctx
                .layer_id_at(pointer)
                .is_some_and(|layer| layer != frame.central_layer);
            let target = (!in_divider && !in_layers_panel && !in_workspace_chrome)
                .then(|| {
                    frame
                        .panes
                        .iter()
                        .find(|pane| pane.canvas.contains(pointer))
                        .map(|pane| PaneTarget {
                            scene: pane.key,
                            pane: pane.pane,
                        })
                })
                .flatten();

            if primary_pressed {
                let active = self.workspace.input.active();
                if in_divider {
                    if let WorkspaceLayout::SideBySide { ratio, .. } = self.workspace.layout {
                        let _ = self.workspace.input.begin_divider_resize(active, ratio);
                    }
                    self.workspace
                        .input
                        .suppress_non_primary_buttons_until_release(buttons);
                } else if in_layers_panel {
                    let _ =
                        self.workspace
                            .input
                            .primary_pressed(active, GestureKind::LayerDrag, None);
                    self.workspace
                        .input
                        .suppress_non_primary_buttons_until_release(buttons);
                } else if in_workspace_chrome {
                    let _ = self.workspace.input.primary_pressed(
                        active,
                        GestureKind::WorkspaceControl,
                        None,
                    );
                    self.workspace
                        .input
                        .suppress_non_primary_buttons_until_release(buttons);
                } else if over_noncentral_ui
                    || (target.is_none() && frame.workspace_rect.contains(pointer))
                {
                    let _ =
                        self.workspace
                            .input
                            .primary_pressed(active, GestureKind::UiControl, None);
                    self.workspace
                        .input
                        .suppress_non_primary_buttons_until_release(buttons);
                } else if let Some(target) = target {
                    let kind = self.primary_gesture_kind(target.scene);
                    if matches!(
                        self.workspace.input.primary_pressed(target, kind, None),
                        PressResult::ActivateOnly(_)
                    ) {
                        self.workspace
                            .input
                            .suppress_non_primary_buttons_until_release(buttons);
                    }
                }
            } else if secondary_pressed || middle_pressed {
                if in_divider || in_layers_panel || in_workspace_chrome || over_noncentral_ui {
                    self.workspace
                        .input
                        .suppress_non_primary_buttons_until_release(buttons);
                } else if let Some(target) = target {
                    if secondary_pressed {
                        let _ = self
                            .workspace
                            .input
                            .begin_safe_gesture(target, GestureKind::CameraOrbit);
                    } else if middle_pressed {
                        let _ = self
                            .workspace
                            .input
                            .begin_safe_gesture(target, GestureKind::CameraPan);
                    }
                }
            } else if !in_divider && !in_layers_panel && !in_workspace_chrome && !over_noncentral_ui
            {
                if let Some(target) = target {
                    if wheel {
                        let _ = self.workspace.input.activate_under_pointer(target);
                    }
                }
            }
        }

        // Pointer ownership is established first so F6 during a press or drag
        // waits for that gesture to finish instead of changing its target.
        if !ctx.text_edit_focused() {
            let forward =
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F6));
            let backward =
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F6));
            if forward || backward {
                self.queue_focus_cycle(backward);
            }
        }
    }
    pub(super) fn pointer_over_workspace_chrome(
        &self,
        pointer: egui::Pos2,
        panes: &[VisiblePane],
    ) -> bool {
        panes
            .iter()
            .any(|pane| pane_header_rect(pane.frame).contains(pointer))
            || self
                .workspace
                .scene_tab_rects
                .iter()
                .any(|(_, rect)| rect.contains(pointer))
            || self
                .workspace
                .scene_create_rect
                .is_some_and(|rect| rect.contains(pointer))
    }
    pub(super) fn pointer_over_layers_panel(
        &self,
        ctx: &egui::Context,
        pointer: egui::Pos2,
        workspace_rect: Rect,
    ) -> bool {
        let Some(active) = self.workspace.scene(self.workspace.active_id()) else {
            return false;
        };
        let scene_count = active
            .document
            .scene
            .as_deref()
            .map_or(0, |scene| scene.meshes().len());
        crate::layers_overlay::current_panel_rect(ctx, workspace_rect, scene_count)
            .contains(pointer)
    }
    fn primary_gesture_kind(&mut self, key: SceneKey) -> GestureKind {
        let Some(scene) = self.scene_context(key) else {
            return GestureKind::Click;
        };
        if scene.tools.bridge_split_active() {
            return GestureKind::Click;
        }
        if scene.tools.sculpt.armed.is_some() {
            GestureKind::SculptStroke
        } else if scene.tools.align.tool.is_armed() {
            GestureKind::AlignDrag
        } else if scene.tools.measure.mode().is_some() {
            GestureKind::Ruler
        } else if scene.document.edit_mode.lasso_armed()
            || (scene.document.edit_mode.has_active_session()
                && scene.tools.editor_tab == crate::app::mesh_editor_overlay::EditorTab::EditMesh)
        {
            GestureKind::Lasso
        } else {
            GestureKind::Click
        }
    }
    fn cancel_captured_gesture(
        &mut self,
        owner: GestureOwner,
        ctx: &egui::Context,
        reason: GestureCancelReason,
    ) {
        match owner.kind {
            GestureKind::DividerResize => {
                if restore_divider_ratio(&mut self.workspace.layout, owner)
                    && matches!(self.workspace.layout, WorkspaceLayout::SideBySide { .. })
                {
                    self.workspace.saved_split = Some(self.workspace.layout);
                }
                return;
            }
            GestureKind::LayerDrag => {
                self.workspace.layer_drag = None;
                return;
            }
            GestureKind::UiControl | GestureKind::WorkspaceControl => return,
            GestureKind::Click
            | GestureKind::CameraOrbit
            | GestureKind::CameraPan
            | GestureKind::SculptStroke
            | GestureKind::Lasso
            | GestureKind::Ruler
            | GestureKind::AlignDrag => {}
        }
        let Some(mut scene) = self.scene_context(owner.target.scene) else {
            return;
        };
        match owner.kind {
            GestureKind::CameraOrbit => {
                scene.release_viewport_orbit_cursor(ctx);
                scene.scene_ui.viewport_secondary_gesture_moved_since_press = false;
            }
            GestureKind::Click if reason == GestureCancelReason::FocusLost => {
                if scene.tools.bridge_split_active() {
                    scene.tools.bridge_split.cancel();
                    scene.tools.bridge_split_disc.disarm();
                    scene.tools.bridge_split_section.reset();
                    scene.document.mesh_selection_drag = None;
                    scene.render.invalidation.overlay_tools_changed();
                } else if scene.tools.cut_view.cancel_pointer_gesture() {
                    scene.render.invalidation.overlay_tools_changed();
                    ctx.request_repaint();
                }
            }
            GestureKind::SculptStroke => scene.abort_sculpt_stroke(),
            GestureKind::Lasso => {
                scene.document.mesh_selection_drag = None;
                ctx.request_repaint();
            }
            GestureKind::Ruler => match reason {
                GestureCancelReason::Escape => scene.disarm_measure_and_probe_cut(),
                GestureCancelReason::FocusLost => {
                    scene.tools.measure.cancel_ruler_drag();
                    ctx.request_repaint();
                }
            },
            GestureKind::AlignDrag => match reason {
                GestureCancelReason::Escape => scene.cancel_align_session(ctx),
                GestureCancelReason::FocusLost => rollback_align_drag(&mut scene),
            },
            GestureKind::CameraPan
            | GestureKind::Click
            | GestureKind::UiControl
            | GestureKind::WorkspaceControl
            | GestureKind::DividerResize => {}
            GestureKind::LayerDrag => *scene.layer_drag = None,
        }
    }
    fn queue_focus_cycle(&mut self, backwards: bool) {
        if self.workspace.scenes.len() < 2 {
            return;
        }
        let active = self.workspace.input.active();
        let current = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == active.scene)
            .unwrap_or(0);
        let next = if backwards {
            (current + self.workspace.scenes.len() - 1) % self.workspace.scenes.len()
        } else {
            (current + 1) % self.workspace.scenes.len()
        };
        let scene = &self.workspace.scenes[next];
        let _ = self.workspace.input.request_activation(PaneTarget {
            scene: scene.key,
            pane: scene.pane,
        });
    }
    pub(super) fn finish_pointer_input(&mut self, ctx: &egui::Context) {
        let (buttons, primary_released, secondary_released, middle_released) = ctx.input(|input| {
            (
                pointer_buttons(input),
                input.pointer.button_released(egui::PointerButton::Primary),
                input
                    .pointer
                    .button_released(egui::PointerButton::Secondary),
                input.pointer.button_released(egui::PointerButton::Middle),
            )
        });
        if primary_released {
            let _ = self.workspace.input.primary_released();
        }
        if secondary_released {
            let _ = self
                .workspace
                .input
                .safe_gesture_released(GestureKind::CameraOrbit);
        }
        if middle_released {
            let _ = self
                .workspace
                .input
                .safe_gesture_released(GestureKind::CameraPan);
        }
        self.workspace.input.synchronize_pointer_state(buttons);
    }
    pub(super) fn handle_workspace_escape(&mut self, ctx: &egui::Context, modal_open: bool) {
        if modal_open || ctx.text_edit_focused() {
            return;
        }

        let owner = self.workspace.input.capture();
        if !workspace_owns_escape(self.workspace.layer_drag.is_some(), owner) {
            return;
        }
        if !ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            return;
        }

        self.workspace.layer_drag = None;
        let buttons = ctx.input(pointer_buttons);
        if let Some(owner) = self.workspace.input.escape(buttons) {
            self.cancel_captured_gesture(owner, ctx, GestureCancelReason::Escape);
        }
    }
}

fn pointer_buttons(input: &egui::InputState) -> PointerButtons {
    PointerButtons {
        primary: input.pointer.button_down(egui::PointerButton::Primary),
        secondary: input.pointer.button_down(egui::PointerButton::Secondary),
        middle: input.pointer.button_down(egui::PointerButton::Middle),
    }
}

/// Tool `Click` captures are left for their scene overlay's Escape handler
/// (Cut and Bridge Split both have one). The workspace only consumes Escape
/// for gestures whose transient state it can cancel, or a standalone layer
/// drag with no competing tool capture.
pub(super) fn workspace_owns_escape(layer_drag_active: bool, owner: Option<GestureOwner>) -> bool {
    let owner_needs_cancel = owner.is_some_and(|owner| {
        matches!(
            owner.kind,
            GestureKind::CameraOrbit
                | GestureKind::CameraPan
                | GestureKind::SculptStroke
                | GestureKind::Lasso
                | GestureKind::Ruler
                | GestureKind::AlignDrag
                | GestureKind::UiControl
                | GestureKind::WorkspaceControl
                | GestureKind::DividerResize
                | GestureKind::LayerDrag
        )
    });
    owner_needs_cancel || (layer_drag_active && owner.is_none())
}

pub(super) fn restore_divider_ratio(layout: &mut WorkspaceLayout, owner: GestureOwner) -> bool {
    let Some(ratio) = owner.divider_initial_ratio() else {
        return false;
    };
    let restored = layout.with_ratio(ratio);
    if restored == *layout {
        return false;
    }
    *layout = restored;
    true
}
