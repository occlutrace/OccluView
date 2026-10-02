//! Workspace composition: which scenes are visible and how one frame is drawn.

use eframe::egui::{pos2, Rect};

use super::pane_geometry::{
    divider_rect, pane_header_rect, pane_is_visible, reconcile_single_layout, PANE_HEADER_HEIGHT,
};
use super::pointer::{VisiblePane, WorkspaceInputFrame};
use crate::app::app_loading::native_drop_paths;
use crate::app::workspace::id::SceneKey;
use crate::app::workspace::layout::{EffectiveLayout, LayoutConstraints, PaneRect};
use crate::app::{egui, OccluViewApp};

const DIVIDER_WIDTH: f32 = 8.0;
const MIN_PANE_WIDTH: f32 = 320.0;

impl OccluViewApp {
    /// Return the scenes that currently have a visible viewport.
    ///
    /// The root renderer uses this to leave hidden peers idle; their invalidation
    /// remains pending and is reconciled when the scene becomes visible again.
    pub(in crate::app) fn visible_scene_keys(&self, ctx: &egui::Context) -> Vec<SceneKey> {
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
    pub(in crate::app) fn show_workspace(&mut self, root_ui: &mut egui::Ui) {
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
            // Its own popup must remain interactive; decision dialogs and
            // window focus gate the controls as well as viewport gestures.
            let layers_enabled = ctx.input(|input| input.focused) && !self.ui.command_dialog_open();
            ui.add_enabled_ui(layers_enabled, |ui| {
                if let Some(mut scene) = self.active_context() {
                    scene.workspace_rect = Some(canvas_bounds);
                    scene.input_allowed &= ctx.input(|input| input.focused) && !modal_open;
                    scene.show_layers_overlay(ui, canvas_bounds, &ctx);
                }
            });

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
}
