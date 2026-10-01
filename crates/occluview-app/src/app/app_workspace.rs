//! Scene workspace composition: pane layout, input ownership, and explicit targets.

use super::app_loading::native_drop_paths;
use super::workspace::commands::{
    LayerDragPayload, LayerIds, SplitSide, TransferDestination, WorkspaceCommand,
};
use super::workspace::id::{PaneId, SceneKey};
use super::workspace::input::{GestureKind, PaneTarget, PointerButtons, PressResult};
use super::workspace::layout::{EffectiveLayout, LayoutConstraints, PaneRect, WorkspaceLayout};
use super::{egui, OccluViewApp};
use crate::icons::AppIcon;
use eframe::egui::{pos2, vec2, Rect};

const PANE_HEADER_HEIGHT: f32 = 30.0;
const DIVIDER_WIDTH: f32 = 8.0;
const MIN_PANE_WIDTH: f32 = 320.0;
const EDGE_DROP_ZONE_WIDTH: f32 = 56.0;

#[derive(Clone)]
struct VisiblePane {
    key: SceneKey,
    pane: PaneId,
    name: String,
    frame: Rect,
    canvas: Rect,
}

struct WorkspaceInputFrame<'a> {
    central_layer: egui::LayerId,
    panes: &'a [VisiblePane],
    workspace_rect: Rect,
    divider_rect: Option<Rect>,
    modal_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GestureCancelReason {
    Escape,
    FocusLost,
}

impl OccluViewApp {
    /// Return the scenes that currently have a visible viewport.
    ///
    /// The root renderer uses this to leave hidden peers idle; their invalidation
    /// remains pending and is reconciled when the scene becomes visible again.
    pub(super) fn visible_scene_keys(&self, ctx: &egui::Context) -> Vec<SceneKey> {
        let active_pane = self.workspace.input.active().pane;
        let effective = self.workspace.layout.effective_rects(
            ctx.content_rect(),
            active_pane,
            LayoutConstraints {
                divider_width: DIVIDER_WIDTH,
                minimum_pane_width: MIN_PANE_WIDTH,
            },
        );
        self.workspace
            .scenes
            .iter()
            .filter(|scene| pane_is_visible(effective, scene.pane))
            .map(|scene| scene.key)
            .collect()
    }

    /// Draw one central surface and route every input to one stable scene key.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_workspace(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        self.ui.workspace_modal_open = self.workspace.rename.is_some()
            || (self.workspace.pending_drop.is_some() && self.workspace.scenes.len() > 1);
        if self.ui.modal_dialog_open() {
            let dropped_file = ctx.input(|input| {
                native_drop_paths(&input.raw.dropped_files)
                    .iter()
                    .any(|path| !path.as_os_str().is_empty())
            });
            if dropped_file {
                let key = self.workspace.input.active().scene;
                let message = self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-close-dialog"));
                self.workspace_status(key, message);
            }
        } else {
            self.collect_native_drops(&ctx);
        }
        self.ui.workspace_modal_open = self.workspace.rename.is_some()
            || (self.workspace.pending_drop.is_some() && self.workspace.scenes.len() > 1);
        self.workspace.scene_tab_rects.clear();
        self.workspace.scene_create_rect = None;

        egui::CentralPanel::no_frame().show(root_ui, |ui| {
            let modal_open = self.ui.modal_dialog_open();
            self.handle_workspace_escape(&ctx, modal_open);
            self.sync_single_layout_to_active();

            let bounds = ui.max_rect();
            ui.painter().rect_filled(
                bounds,
                0.0,
                self.persistence.settings.viewport_background.srgb(),
            );
            let mut effective = self.workspace.layout.effective_rects(
                bounds,
                self.workspace.input.active().pane,
                LayoutConstraints {
                    divider_width: DIVIDER_WIDTH,
                    minimum_pane_width: MIN_PANE_WIDTH,
                },
            );
            let mut panes = self.visible_panes(effective);
            let canvas_bounds = Rect::from_min_max(
                pos2(
                    bounds.left(),
                    (bounds.top() + PANE_HEADER_HEIGHT).min(bounds.bottom()),
                ),
                bounds.max,
            );

            self.route_pointer_input(
                ui.ctx(),
                &WorkspaceInputFrame {
                    central_layer: ui.layer_id(),
                    panes: &panes,
                    workspace_rect: canvas_bounds,
                    divider_rect: divider_rect(effective),
                    modal_open,
                },
            );
            if self.sync_single_layout_to_active() {
                effective = self.workspace.layout.effective_rects(
                    bounds,
                    self.workspace.input.active().pane,
                    LayoutConstraints {
                        divider_width: DIVIDER_WIDTH,
                        minimum_pane_width: MIN_PANE_WIDTH,
                    },
                );
                panes = self.visible_panes(effective);
            }
            let input_frame = WorkspaceInputFrame {
                central_layer: ui.layer_id(),
                panes: &panes,
                workspace_rect: canvas_bounds,
                divider_rect: divider_rect(effective),
                modal_open,
            };

            for pane in &panes {
                self.show_pane_header(ui, pane, effective, modal_open);
            }

            let visible_keys: Vec<_> = panes.iter().map(|pane| pane.key).collect();
            for scene in &mut self.workspace.scenes {
                if !visible_keys.contains(&scene.key) {
                    scene.render.live_viewport_px = None;
                }
            }

            for pane in &panes {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt(pane.pane)
                        .max_rect(pane.canvas),
                    |ui| {
                        ui.set_clip_rect(pane.canvas);
                        if let Some(mut scene) = self.scene_context(pane.key) {
                            scene.workspace_rect = Some(canvas_bounds);
                            scene.input_allowed &= ctx.input(|input| input.focused) && !modal_open;
                            scene.show_pane(ui, pane.canvas, canvas_bounds, &ctx);
                        }
                    },
                );
            }

            // Layers belongs to the active document, but its one overlay uses
            // the full shared work area rather than stealing half of a split.
            if let Some(mut scene) = self.active_context() {
                scene.workspace_rect = Some(canvas_bounds);
                scene.input_allowed &= ctx.input(|input| input.focused) && !modal_open;
                scene.show_layers_overlay(ui, canvas_bounds, &ctx);
            }

            self.show_divider(ui, effective, &ctx, modal_open);
            self.finish_layer_drag(&ctx, &input_frame);
            self.show_layer_drag_preview(ui, &input_frame);
            self.finish_pointer_input(&ctx);
            if self.sync_single_layout_to_active() {
                ctx.request_repaint();
            }
        });

        self.show_workspace_dialogs(&ctx);
    }

    fn visible_panes(&self, layout: EffectiveLayout) -> Vec<VisiblePane> {
        let pane_rects = match layout {
            EffectiveLayout::Single { pane, rect, .. } => vec![PaneRect { pane, rect }],
            EffectiveLayout::SideBySide { left, right, .. } => vec![left, right],
        };
        pane_rects
            .into_iter()
            .filter_map(|pane_rect| {
                let scene = self
                    .workspace
                    .scenes
                    .iter()
                    .find(|scene| scene.pane == pane_rect.pane)?;
                let header = pane_header_rect(pane_rect.rect);
                Some(VisiblePane {
                    key: scene.key,
                    pane: scene.pane,
                    name: scene.name.clone(),
                    frame: pane_rect.rect,
                    canvas: Rect::from_min_max(
                        pos2(pane_rect.rect.left(), header.bottom()),
                        pane_rect.rect.max,
                    ),
                })
            })
            .collect()
    }

    fn sync_single_layout_to_active(&mut self) -> bool {
        reconcile_single_layout(
            &mut self.workspace.layout,
            self.workspace.input.active().pane,
        )
    }

    // One header binds its title and view controls to the same pane.
    #[allow(clippy::too_many_lines)]
    fn show_pane_header(
        &mut self,
        ui: &mut egui::Ui,
        pane: &VisiblePane,
        layout: EffectiveLayout,
        modal_open: bool,
    ) {
        let rect = pane_header_rect(pane.frame);
        let is_active = self.workspace.input.active().scene == pane.key;
        let split = matches!(layout, EffectiveLayout::SideBySide { .. });
        ui.painter()
            .rect_filled(rect, 0.0, crate::ui_theme::panel_fill());
        let separator_y = rect.bottom() - if is_active && split { 1.0 } else { 0.5 };
        ui.painter().line_segment(
            [
                pos2(rect.left(), separator_y),
                pos2(rect.right(), separator_y),
            ],
            if is_active && split {
                egui::Stroke::new(2.0, crate::ui_theme::accent())
            } else {
                egui::Stroke::new(1.0, crate::ui_theme::hairline())
            },
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("pane-header", pane.pane))
                .max_rect(rect.shrink2(vec2(8.0, 2.0))),
            |ui| {
                ui.horizontal_centered(|ui| {
                    let available_title_width = (ui.available_width() - 40.0).max(48.0);
                    let title = ui
                        .add_sized(
                            vec2(available_title_width, 25.0),
                            egui::Button::selectable(is_active, &pane.name)
                                .frame(false)
                                .truncate(),
                        )
                        .on_hover_text(&pane.name);
                    crate::accessibility::button(&title, &pane.name, !modal_open, Some(is_active));
                    if title.has_focus() && !modal_open {
                        ui.painter().rect_stroke(
                            title.rect,
                            3.0,
                            egui::Stroke::new(1.0, crate::ui_theme::accent()),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if title.clicked() && !modal_open && !is_active {
                        self.workspace
                            .commands
                            .push_back(WorkspaceCommand::Activate(PaneTarget {
                                scene: pane.key,
                                pane: pane.pane,
                            }));
                    }

                    if split {
                        let label = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-fullscreen"));
                        let (_, response) =
                            ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
                        paint_pane_action(
                            ui,
                            &response,
                            AppIcon::FitView,
                            if is_active {
                                crate::ui_theme::accent()
                            } else {
                                crate::ui_theme::text_weak()
                            },
                            !modal_open,
                        );
                        let response = response.on_hover_text(&label);
                        crate::accessibility::button(&response, &label, !modal_open, None);
                        if response.clicked() && !modal_open {
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::Activate(PaneTarget {
                                    scene: pane.key,
                                    pane: pane.pane,
                                }));
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::SetLayout(WorkspaceLayout::single(
                                    pane.pane,
                                )));
                        }
                    } else if self.workspace.scenes.len() > 1 {
                        let label = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-two-views"));
                        let (_, response) =
                            ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
                        paint_pane_action(
                            ui,
                            &response,
                            AppIcon::SplitView,
                            crate::ui_theme::text_weak(),
                            !modal_open,
                        );
                        let response = response.on_hover_text(&label);
                        crate::accessibility::button(&response, &label, !modal_open, None);
                        if response.clicked() && !modal_open {
                            let layout = self
                                .workspace
                                .saved_split
                                .unwrap_or_else(|| default_split(&self.workspace.scenes));
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::SetLayout(layout));
                        }
                    }
                });
            },
        );
    }

    fn show_divider(
        &mut self,
        ui: &mut egui::Ui,
        layout: EffectiveLayout,
        ctx: &egui::Context,
        modal_open: bool,
    ) -> Option<Rect> {
        let EffectiveLayout::SideBySide {
            left,
            right,
            divider,
        } = layout
        else {
            return None;
        };
        let id = egui::Id::new("workspace-scene-divider");
        let response = ui
            .interact(divider, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let label = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-divider"));
        let current_ratio = match self.workspace.layout {
            WorkspaceLayout::SideBySide { ratio, .. } => ratio,
            WorkspaceLayout::Single { .. } => return Some(divider),
        };
        if modal_open {
            crate::accessibility::slider(&response, &label, false, f64::from(current_ratio));
            return Some(divider);
        }

        let mut ratio = current_ratio;
        crate::accessibility::slider(&response, &label, true, f64::from(ratio));
        if response.double_clicked() {
            ratio = 0.5;
        } else if response.dragged_by(egui::PointerButton::Primary)
            && self
                .workspace
                .input
                .capture()
                .is_some_and(|owner| owner.kind == GestureKind::DividerResize)
        {
            if let Some(pointer) = ctx.input(|input| input.pointer.interact_pos()) {
                let content_width = (left.rect.width() + right.rect.width()).max(1.0);
                ratio = ((pointer.x - left.rect.left() - divider.width() * 0.5) / content_width)
                    .clamp(0.05, 0.95);
            }
        } else if response.has_focus() {
            let step = 0.05;
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft))
            {
                ratio -= step;
            }
            if ctx
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
            {
                ratio += step;
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Home)) {
                ratio = 0.5;
            }
        }
        let next_layout = self.workspace.layout.with_ratio(ratio);
        if next_layout != self.workspace.layout {
            self.workspace
                .commands
                .push_back(WorkspaceCommand::SetLayout(next_layout));
        }
        if response.has_focus() {
            ui.painter().rect_filled(
                divider.shrink(2.0),
                2.0,
                crate::ui_theme::accent().gamma_multiply(0.16),
            );
            ui.painter().rect_stroke(
                divider.shrink(1.0),
                2.0,
                egui::Stroke::new(1.0, crate::ui_theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
        Some(divider)
    }

    // Resolve one input frame in order: cancellation, capture, wheel, keyboard.
    #[allow(clippy::too_many_lines)]
    fn route_pointer_input(&mut self, ctx: &egui::Context, frame: &WorkspaceInputFrame<'_>) {
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

    fn pointer_over_workspace_chrome(&self, pointer: egui::Pos2, panes: &[VisiblePane]) -> bool {
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

    fn pointer_over_layers_panel(
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
                && scene.tools.editor_tab == crate::mesh_editor_overlay::EditorTab::EditMesh)
        {
            GestureKind::Lasso
        } else {
            GestureKind::Click
        }
    }

    fn cancel_captured_gesture(
        &mut self,
        owner: super::workspace::input::GestureOwner,
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

    fn finish_pointer_input(&mut self, ctx: &egui::Context) {
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

    fn handle_workspace_escape(&mut self, ctx: &egui::Context, modal_open: bool) {
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

    fn collect_native_drops(&mut self, ctx: &egui::Context) {
        let paths = ctx.input(|input| native_drop_paths(&input.raw.dropped_files));
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        if paths.is_empty() {
            return;
        }
        if let Some(pending) = self.workspace.pending_drop.as_mut() {
            pending.extend(paths);
        } else {
            self.workspace.pending_drop = Some(paths);
        }
        if self.workspace.scenes.len() == 1 {
            let key = self.workspace.scenes[0].key;
            self.enqueue_drop(key);
        }
    }

    fn enqueue_drop(&mut self, key: SceneKey) {
        let Some(paths) = self.workspace.pending_drop.take() else {
            return;
        };
        if let Some(mut scene) = self.scene_context(key) {
            scene.enqueue_dropped_paths(key, &paths);
        }
    }

    fn show_workspace_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((scene, value)) = self.workspace.rename.clone() {
            if self
                .workspace
                .scenes
                .iter()
                .any(|candidate| candidate.key == scene)
            {
                self.show_rename_dialog(ctx, scene, value);
            } else {
                self.workspace.rename = None;
            }
        } else if self.workspace.pending_drop.is_some() {
            if self.workspace.scenes.len() == 1 {
                let key = self.workspace.scenes[0].key;
                self.enqueue_drop(key);
            } else {
                self.show_drop_target_dialog(ctx);
            }
        }
        self.ui.workspace_modal_open = self.workspace.rename.is_some()
            || (self.workspace.pending_drop.is_some() && self.workspace.scenes.len() > 1);
    }

    fn show_drop_target_dialog(&mut self, ctx: &egui::Context) {
        let title = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-drop-title"));
        let description = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-drop-description"));
        let cancel = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-cancel"));
        let mut target = None;
        let mut cancel_drop = false;
        let modal = crate::modal_surface::show_information_modal(
            ctx,
            egui::Id::new("workspace-drop-target-dialog"),
            vec2(380.0, 180.0),
            &cancel,
            |ui| {
                ui.label(
                    egui::RichText::new(&title)
                        .strong()
                        .size(17.0)
                        .color(crate::ui_theme::text()),
                );
                ui.add_space(4.0);
                ui.label(&description);
                ui.add_space(12.0);
                for scene in self.workspace.summaries() {
                    if ui
                        .add_sized(
                            vec2(ui.available_width(), 30.0),
                            egui::Button::new(&scene.name).truncate(),
                        )
                        .on_hover_text(&scene.name)
                        .clicked()
                    {
                        target = Some(scene.key);
                    }
                }
                ui.add_space(8.0);
                if ui.button(&cancel).clicked() {
                    cancel_drop = true;
                }
            },
        );
        if cancel_drop || modal.should_close() {
            self.workspace.pending_drop = None;
        } else if let Some(target) = target {
            self.enqueue_drop(target);
        }
    }

    fn show_rename_dialog(&mut self, ctx: &egui::Context, scene: SceneKey, initial: String) {
        let title = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-rename-title"));
        let apply = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-rename-apply"));
        let cancel = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-cancel"));
        let mut value = initial;
        let mut submit = false;
        let mut dismiss = false;
        let modal = crate::modal_surface::show_information_modal(
            ctx,
            egui::Id::new("workspace-rename-dialog"),
            vec2(380.0, 160.0),
            &cancel,
            |ui| {
                let title_response = ui.label(
                    egui::RichText::new(&title)
                        .strong()
                        .size(17.0)
                        .color(crate::ui_theme::text()),
                );
                ui.add_space(8.0);
                let response = ui
                    .add(
                        egui::TextEdit::singleline(&mut value)
                            .id(egui::Id::new("workspace-rename-value"))
                            .char_limit(128)
                            .desired_width(ui.available_width()),
                    )
                    .labelled_by(title_response.id);
                if self.workspace.rename_focus_pending {
                    response.request_focus();
                    self.workspace.rename_focus_pending = false;
                }
                if (response.has_focus() || response.lost_focus())
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    submit = !value.trim().is_empty();
                    if !submit {
                        response.request_focus();
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!value.trim().is_empty(), egui::Button::new(&apply))
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui.button(&cancel).clicked() {
                        dismiss = true;
                    }
                });
            },
        );
        if let Some(rename) = self.workspace.rename.as_mut() {
            if rename.0 == scene {
                rename.1.clone_from(&value);
            }
        }
        if dismiss || modal.should_close() {
            self.workspace.rename = None;
        } else if submit {
            self.workspace
                .commands
                .push_back(WorkspaceCommand::RenameScene { scene, name: value });
            self.workspace.rename = None;
        }
    }

    fn finish_layer_drag(&mut self, ctx: &egui::Context, frame: &WorkspaceInputFrame<'_>) {
        let released =
            ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if !released {
            return;
        }
        let Some(payload) = self.workspace.layer_drag.take() else {
            return;
        };
        let Some(pointer) = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())
        }) else {
            return;
        };
        if frame.modal_open
            || self.ui.modal_dialog_open()
            || ctx
                .layer_id_at(pointer)
                .is_some_and(|layer| layer != frame.central_layer)
        {
            return;
        }

        if let Some((destination, _)) = self
            .workspace
            .scene_tab_rects
            .iter()
            .find(|(key, rect)| *key != payload.source && rect.contains(pointer))
        {
            self.queue_layer_transfer(payload, TransferDestination::Existing(*destination));
            return;
        }

        // The footer's plus is an explicit create target with the same
        // right-side default as clicking it. It takes priority over the broad
        // panel hit area below.
        if self.workspace.scenes.len() < 2
            && self
                .workspace
                .scene_create_rect
                .is_some_and(|rect| rect.contains(pointer))
        {
            self.create_scene_for_layer_transfer(payload, SplitSide::Right);
            return;
        }

        if let Some(destination) = frame
            .panes
            .iter()
            .find(|pane| pane.key != payload.source && pane.canvas.contains(pointer))
        {
            self.queue_layer_transfer(payload, TransferDestination::Existing(destination.key));
            return;
        }
        // Dropping back onto the source panel or pane is a cancellation. It
        // must not accidentally count as an edge split just because the Layers
        // panel sits against that edge.
        if self.pointer_over_workspace_chrome(pointer, frame.panes)
            || self.pointer_over_layers_panel(ctx, pointer, frame.workspace_rect)
            || !frame.workspace_rect.contains(pointer)
        {
            return;
        }
        if self.workspace.scenes.len() >= 2 {
            return;
        }
        let side = if pointer.x <= frame.workspace_rect.left() + EDGE_DROP_ZONE_WIDTH {
            Some(SplitSide::Left)
        } else if pointer.x >= frame.workspace_rect.right() - EDGE_DROP_ZONE_WIDTH {
            Some(SplitSide::Right)
        } else {
            None
        };
        let Some(side) = side else {
            return;
        };
        self.create_scene_for_layer_transfer(payload, side);
    }

    fn create_scene_for_layer_transfer(&mut self, payload: LayerDragPayload, side: SplitSide) {
        let Ok(scene) = self.workspace.ids.allocate_scene() else {
            return;
        };
        let Ok(pane) = self.workspace.ids.allocate_pane_id() else {
            return;
        };
        self.queue_layer_transfer(
            payload,
            TransferDestination::CreateBeside { scene, pane, side },
        );
    }

    fn queue_layer_transfer(
        &mut self,
        payload: LayerDragPayload,
        destination: TransferDestination,
    ) {
        self.workspace
            .commands
            .push_back(WorkspaceCommand::Transfer {
                source: payload.source,
                destination,
                layers: LayerIds::one(payload.layer),
            });
    }

    fn show_layer_drag_preview(&self, ui: &mut egui::Ui, frame: &WorkspaceInputFrame<'_>) {
        let Some(payload) = self.workspace.layer_drag else {
            return;
        };
        let Some(pointer) = ui.ctx().input(|input| input.pointer.hover_pos()) else {
            return;
        };
        if frame.modal_open
            || self.ui.modal_dialog_open()
            || ui
                .ctx()
                .layer_id_at(pointer)
                .is_some_and(|layer| layer != frame.central_layer)
        {
            return;
        }
        let color = crate::ui_theme::accent().gamma_multiply(0.17);
        let stroke = egui::Stroke::new(2.0, crate::ui_theme::accent());

        let target = self
            .workspace
            .scene_tab_rects
            .iter()
            .find(|(key, rect)| *key != payload.source && rect.contains(pointer))
            .map(|(_, rect)| *rect)
            .or_else(|| {
                if self.workspace.scenes.len() < 2 {
                    self.workspace
                        .scene_create_rect
                        .filter(|rect| rect.contains(pointer))
                } else {
                    None
                }
            })
            .or_else(|| {
                if self.pointer_over_workspace_chrome(pointer, frame.panes)
                    || self.pointer_over_layers_panel(ui.ctx(), pointer, frame.workspace_rect)
                {
                    return None;
                }
                frame
                    .panes
                    .iter()
                    .find(|pane| pane.key != payload.source && pane.canvas.contains(pointer))
                    .map(|pane| pane.canvas)
                    .or_else(|| {
                        if self.workspace.scenes.len() >= 2
                            || !frame.workspace_rect.contains(pointer)
                        {
                            return None;
                        }
                        if pointer.x <= frame.workspace_rect.left() + EDGE_DROP_ZONE_WIDTH {
                            Some(Rect::from_min_max(
                                frame.workspace_rect.min,
                                pos2(
                                    frame.workspace_rect.center().x,
                                    frame.workspace_rect.bottom(),
                                ),
                            ))
                        } else if pointer.x >= frame.workspace_rect.right() - EDGE_DROP_ZONE_WIDTH {
                            Some(Rect::from_min_max(
                                pos2(frame.workspace_rect.center().x, frame.workspace_rect.top()),
                                frame.workspace_rect.max,
                            ))
                        } else {
                            None
                        }
                    })
            });
        if let Some(target) = target {
            ui.painter().rect_filled(target, 3.0, color);
            ui.painter()
                .rect_stroke(target, 3.0, stroke, egui::StrokeKind::Inside);
            self.paint_layer_drop_label(ui.painter(), target, pointer, frame.workspace_rect);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }

    fn paint_layer_drop_label(
        &self,
        painter: &egui::Painter,
        target: Rect,
        drop_position: egui::Pos2,
        workspace_rect: Rect,
    ) {
        if target.height() <= 80.0 || target.width() <= 120.0 {
            return;
        }
        let label = self.ui.locale.tr(if self.workspace.scenes.len() < 2 {
            if drop_position.x < workspace_rect.center().x {
                crate::i18n::message_id!("workspace-new-left")
            } else {
                crate::i18n::message_id!("workspace-new-right")
            }
        } else {
            crate::i18n::message_id!("workspace-move-layer")
        });
        let galley = painter.layout_no_wrap(
            label,
            egui::FontId::proportional(14.0),
            crate::ui_theme::text(),
        );
        let label_rect = Rect::from_center_size(target.center(), galley.size() + vec2(24.0, 16.0));
        painter.rect_filled(label_rect, 6.0, crate::ui_theme::panel_fill());
        painter.galley(
            label_rect.min + vec2(12.0, 8.0),
            galley,
            crate::ui_theme::text(),
        );
    }
}

fn paint_pane_action(
    ui: &egui::Ui,
    response: &egui::Response,
    icon: AppIcon,
    ink: egui::Color32,
    enabled: bool,
) {
    let rect = response.rect;
    if enabled && (response.hovered() || response.has_focus()) {
        ui.painter()
            .rect_filled(rect, 3.0, crate::ui_theme::row_hover_fill());
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(1.0, crate::ui_theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
    }
    crate::icons::paint(ui.painter(), rect.shrink(5.0), icon, ink);
}

/// Roll back just the in-progress manual pose drag after focus leaves the
/// window. The Align session remains armed; unlike Escape this does not restore
/// earlier, already completed moves from the session.
fn rollback_align_drag(scene: &mut super::SceneContext<'_>) {
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

/// Tool `Click` captures are left for their scene overlay's Escape handler
/// (Cut and Bridge Split both have one). The workspace only consumes Escape
/// for gestures whose transient state it can cancel, or a standalone layer
/// drag with no competing tool capture.
fn workspace_owns_escape(
    layer_drag_active: bool,
    owner: Option<super::workspace::input::GestureOwner>,
) -> bool {
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

fn pane_header_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        rect.min,
        pos2(
            rect.right(),
            (rect.top() + PANE_HEADER_HEIGHT).min(rect.bottom()),
        ),
    )
}

fn pane_is_visible(layout: EffectiveLayout, pane: PaneId) -> bool {
    match layout {
        EffectiveLayout::Single { pane: visible, .. } => pane == visible,
        EffectiveLayout::SideBySide { left, right, .. } => pane == left.pane || pane == right.pane,
    }
}

fn reconcile_single_layout(layout: &mut WorkspaceLayout, active_pane: PaneId) -> bool {
    let WorkspaceLayout::Single { pane } = layout else {
        return false;
    };
    if *pane == active_pane {
        return false;
    }
    *layout = WorkspaceLayout::single(active_pane);
    true
}

fn default_split(scenes: &[super::workspace::state::SceneSession]) -> WorkspaceLayout {
    let Some(left) = scenes.first() else {
        return WorkspaceLayout::single(PaneId::INITIAL);
    };
    let Some(right) = scenes.get(1) else {
        return WorkspaceLayout::single(left.pane);
    };
    WorkspaceLayout::SideBySide {
        left: left.pane,
        right: right.pane,
        ratio: 0.5,
    }
}

fn divider_rect(layout: EffectiveLayout) -> Option<Rect> {
    match layout {
        EffectiveLayout::Single { .. } => None,
        EffectiveLayout::SideBySide { divider, .. } => Some(divider),
    }
}

fn pointer_buttons(input: &egui::InputState) -> PointerButtons {
    PointerButtons {
        primary: input.pointer.button_down(egui::PointerButton::Primary),
        secondary: input.pointer.button_down(egui::PointerButton::Secondary),
        middle: input.pointer.button_down(egui::PointerButton::Middle),
    }
}

fn restore_divider_ratio(
    layout: &mut WorkspaceLayout,
    owner: super::workspace::input::GestureOwner,
) -> bool {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::{reconcile_single_layout, restore_divider_ratio, workspace_owns_escape};
    use crate::app::workspace::id::{PaneId, SceneKey};
    use crate::app::workspace::input::{
        ActivationResult, GestureKind, InputArbiter, PaneTarget, PointerButtons, PressResult,
        ReleaseResult,
    };
    use crate::app::workspace::layout::WorkspaceLayout;

    fn captured(kind: GestureKind) -> crate::app::workspace::input::GestureOwner {
        let target = PaneTarget {
            scene: SceneKey::from_raw_for_test(1, 1).expect("test scene key"),
            pane: PaneId::from_raw_for_test(1).expect("test pane key"),
        };
        let mut input = InputArbiter::new(target);
        match input.primary_pressed(target, kind, None) {
            PressResult::Begin(owner) => owner,
            other => panic!("expected captured press, got {other:?}"),
        }
    }

    #[test]
    fn canceling_manual_alignment_restores_pose_and_clears_pointer_state() {
        use crate::app::app_align_drag::AlignDrag;
        use crate::app::app_test_support::{named_scene, test_app};
        use glam::{Affine3A, Vec3};
        use std::sync::Arc;

        let mut app = test_app("cancel-manual-align");
        let mut model = named_scene("moving", 0.0);
        let layer = model.meshes()[0].id();
        let before = Affine3A::from_translation(Vec3::X);
        model.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::Y);
        app.workspace.scenes[0].document.scene = Some(Arc::new(model));
        let mut scene = app.active_context().expect("live scene");
        scene.tools.align.drag = Some(AlignDrag {
            layer,
            start: before,
            pivot_local: Vec3::ZERO,
        });
        scene.tools.align.drag_last_pointer_pos = Some(egui::pos2(10.0, 20.0));
        scene.tools.align.drag_modifiers = Some(egui::Modifiers::CTRL);
        scene.tools.align.drag_pose_changed = true;
        scene.document.unsaved_drag_pose = true;

        super::rollback_align_drag(&mut scene);

        assert_eq!(
            scene.document.scene.as_ref().expect("scene").meshes()[0].transform,
            before
        );
        assert!(scene.tools.align.drag.is_none());
        assert!(scene.tools.align.drag_last_pointer_pos.is_none());
        assert!(scene.tools.align.drag_modifiers.is_none());
        assert!(!scene.tools.align.drag_pose_changed);
        assert!(!scene.document.unsaved_drag_pose);
    }

    #[test]
    fn click_capture_leaves_escape_for_scene_tools() {
        let click = captured(GestureKind::Click);
        assert!(!workspace_owns_escape(false, Some(click)));
        assert!(!workspace_owns_escape(true, Some(click)));
    }

    #[test]
    fn workspace_cancels_owned_gestures_and_uncontested_layer_drag() {
        let ruler = captured(GestureKind::Ruler);
        assert!(workspace_owns_escape(false, Some(ruler)));
        assert!(workspace_owns_escape(true, None));
        assert!(!workspace_owns_escape(false, None));
    }

    #[test]
    fn divider_cancel_restores_ratio_and_defers_focus_cycle_until_release() {
        let first = PaneTarget {
            scene: SceneKey::from_raw_for_test(1, 1).expect("first scene key"),
            pane: PaneId::from_raw_for_test(1).expect("first pane key"),
        };
        let second = PaneTarget {
            scene: SceneKey::from_raw_for_test(2, 2).expect("second scene key"),
            pane: PaneId::from_raw_for_test(2).expect("second pane key"),
        };
        let mut input = InputArbiter::new(first);
        let initial =
            WorkspaceLayout::side_by_side(first.pane, second.pane, 0.35).expect("distinct panes");
        let mut layout = initial;
        let PressResult::Begin(owner) = input.begin_divider_resize(first, 0.35) else {
            panic!("the active divider should capture its pointer");
        };
        layout = layout.with_ratio(0.72);

        assert_eq!(
            input.request_activation(second),
            ActivationResult::DeferredUntilGestureEnds
        );
        assert_eq!(input.active(), first);

        assert_eq!(
            input.escape(PointerButtons {
                primary: true,
                ..PointerButtons::default()
            }),
            Some(owner)
        );
        assert!(restore_divider_ratio(&mut layout, owner));
        assert_eq!(layout, initial);
        assert_eq!(
            input.active(),
            first,
            "Escape must not switch scenes mid-hold"
        );

        assert_eq!(input.primary_released(), ReleaseResult::Suppressed);
        assert_eq!(
            input.active(),
            second,
            "the deferred F6 switch applies on release"
        );
    }

    #[test]
    fn deferred_activation_reconciles_the_single_visible_pane_after_release() {
        let first = PaneTarget {
            scene: SceneKey::from_raw_for_test(1, 1).expect("first scene key"),
            pane: PaneId::from_raw_for_test(1).expect("first pane key"),
        };
        let second = PaneTarget {
            scene: SceneKey::from_raw_for_test(2, 2).expect("second scene key"),
            pane: PaneId::from_raw_for_test(2).expect("second pane key"),
        };
        let mut input = InputArbiter::new(first);
        let mut layout = WorkspaceLayout::single(first.pane);
        let PressResult::Begin(owner) = input.primary_pressed(first, GestureKind::CameraPan, None)
        else {
            panic!("the active scene should own the camera gesture");
        };

        assert_eq!(
            input.request_activation(second),
            ActivationResult::DeferredUntilGestureEnds
        );
        assert_eq!(layout, WorkspaceLayout::single(first.pane));
        assert_eq!(input.primary_released(), ReleaseResult::Finished(owner));
        assert!(reconcile_single_layout(&mut layout, input.active().pane));
        assert_eq!(layout, WorkspaceLayout::single(second.pane));
    }
}
