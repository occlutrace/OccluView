//! Workspace commands and document lifetime transitions.

use super::app_guard_dialog::{show_guard_dialog, GuardDialogAction, GuardDialogSpec};
use super::app_mesh_export::SaveEditedLayersOutcome;
use super::workspace::commands::{LayerIds, SplitSide, TransferDestination, WorkspaceCommand};
use super::workspace::history::HistoryDirection;
use super::workspace::id::{PaneId, SceneKey};
use super::workspace::input::InputArbiter;
use super::workspace::layout::WorkspaceLayout;
use super::workspace::state::{CloseRequest, SceneSession};
use super::{egui, OccluViewApp, SceneContext};
use occluview_core::SceneMeshId;

impl OccluViewApp {
    pub(in crate::app) fn make_scene_session(
        &self,
        key: SceneKey,
        pane: PaneId,
        name: String,
    ) -> Result<SceneSession, String> {
        let viewport = self
            .workspace
            .scenes
            .iter()
            .find_map(|scene| scene.render.live_viewport.as_ref());
        let peer = match viewport {
            Some(viewport) => Some(
                viewport
                    .lock()
                    .map_err(|error| error.to_string())?
                    .new_peer()
                    .map_err(|error| error.to_string())?,
            ),
            None => None,
        };
        self.seed_scene_brush(key);
        tracing::debug!(
            scene_id = key.id.get(),
            scene_epoch = key.epoch.get(),
            pane_id = pane.get(),
            "scene created"
        );
        Ok(SceneSession::new(
            key,
            pane,
            name,
            peer,
            &self.workspace.history,
        ))
    }

    fn seed_scene_brush(&self, key: SceneKey) {
        let mut settings = self.persistence.settings.clone();
        super::state::restore_sculpt_preferences(&self.ui.repaint_ctx, key, &mut settings);
    }

    pub(in crate::app) fn workspace_status(&mut self, key: SceneKey, message: String) {
        if let Some(scene) = self
            .workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == key)
        {
            scene.presentation.status_message = Some(message);
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn apply_workspace_commands(&mut self, ctx: &egui::Context) {
        let had_commands = !self.workspace.commands.is_empty();
        while let Some(command) = self.workspace.commands.pop_front() {
            if self.ui.command_dialog_open() && !matches!(command, WorkspaceCommand::RetryGraphics)
            {
                continue;
            }
            let result = match command {
                WorkspaceCommand::RetryGraphics => {
                    for key in self.workspace.keys() {
                        if let Some(mut scene) = self.scene_context(key) {
                            scene.retry_gpu_after_fault(ctx);
                        }
                    }
                    Ok(())
                }
                WorkspaceCommand::Activate(target) => {
                    if !self.ui.command_dialog_open()
                        && self
                            .workspace
                            .scenes
                            .iter()
                            .any(|scene| scene.target() == target)
                    {
                        self.workspace.input.request_activation(target);
                        if matches!(self.workspace.layout, WorkspaceLayout::Single { .. }) {
                            self.workspace.layout =
                                WorkspaceLayout::single(self.workspace.input.active().pane);
                        }
                    }
                    Ok(())
                }
                WorkspaceCommand::CreateScene {
                    anchor,
                    scene,
                    pane,
                    side,
                } => self.create_workspace_scene(anchor, scene, pane, side),
                WorkspaceCommand::SetLayout(layout) => {
                    let valid = match layout {
                        WorkspaceLayout::Single { pane } => {
                            self.workspace.scenes.iter().any(|scene| scene.pane == pane)
                        }
                        WorkspaceLayout::SideBySide { left, right, .. } => {
                            left != right
                                && [left, right].iter().all(|pane| {
                                    self.workspace
                                        .scenes
                                        .iter()
                                        .any(|scene| scene.pane == *pane)
                                })
                        }
                    };
                    if valid {
                        if matches!(layout, WorkspaceLayout::SideBySide { .. }) {
                            self.workspace.saved_split = Some(layout);
                        } else if matches!(
                            self.workspace.layout,
                            WorkspaceLayout::SideBySide { .. }
                        ) {
                            self.workspace.saved_split = Some(self.workspace.layout);
                        }
                        self.workspace.layout = layout;
                        for scene in &mut self.workspace.scenes {
                            scene.preserve_on_transfer_undo = true;
                        }
                    }
                    Ok(())
                }
                WorkspaceCommand::RequestRenameScene { scene } => {
                    if let Some(target) = self
                        .workspace
                        .scenes
                        .iter()
                        .find(|target| target.key == scene)
                    {
                        self.workspace.rename = Some((scene, target.name.clone()));
                        self.ui.workspace_modal_open = true;
                        self.workspace.rename_focus_pending = true;
                    }
                    Ok(())
                }
                WorkspaceCommand::RenameScene { scene, name } => {
                    let name: String = name.trim().chars().take(128).collect();
                    if !name.is_empty() {
                        if let Some(target) = self
                            .workspace
                            .scenes
                            .iter_mut()
                            .find(|target| target.key == scene)
                        {
                            target.name = name;
                            target.preserve_on_transfer_undo = true;
                        }
                    }
                    Ok(())
                }
                WorkspaceCommand::CloseScene { scene } => {
                    self.request_scene_close(scene);
                    Ok(())
                }
                WorkspaceCommand::Transfer {
                    source,
                    destination,
                    layers,
                } => self.transfer_layers(source, destination, layers),
                WorkspaceCommand::Undo { scene } => {
                    self.navigate_workspace_history(scene, HistoryDirection::Undo)
                }
                WorkspaceCommand::Redo { scene } => {
                    self.navigate_workspace_history(scene, HistoryDirection::Redo)
                }
            };
            if let Err(error) = result {
                if let Some(scene) = self.workspace.scene(self.workspace.active_id()) {
                    self.workspace_status(scene.key, error);
                }
            }
        }
        if had_commands {
            ctx.request_repaint();
        }
    }

    fn create_workspace_scene(
        &mut self,
        anchor: SceneKey,
        key: SceneKey,
        pane: PaneId,
        side: SplitSide,
    ) -> Result<(), String> {
        if self.workspace.scenes.len() >= 2 {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-two-scene-limit")));
        }
        let Some(anchor_pane) = self
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == anchor)
            .map(|scene| scene.pane)
        else {
            return Ok(());
        };
        let name = self.ui.locale.tr_with(
            crate::i18n::message_id!("workspace-scene-name"),
            &[("number", &key.id.get().to_string())],
        );
        let scene = self.make_scene_session(key, pane, name)?;
        let target = scene.target();
        let (left, right) = match side {
            SplitSide::Left => (pane, anchor_pane),
            SplitSide::Right => (anchor_pane, pane),
        };
        let layout = WorkspaceLayout::side_by_side(left, right, 0.5)
            .map_err(|_| "The new scene needs a separate viewport.".to_owned())?;
        self.workspace.scenes.push(scene);
        self.workspace.layout = layout;
        self.workspace.saved_split = Some(self.workspace.layout);
        self.workspace.input.request_activation(target);
        Ok(())
    }

    fn request_scene_close(&mut self, key: SceneKey) {
        let needs_guard = self.scene_context(key).is_some_and(|scene| {
            scene.document.has_unsaved_mesh_edits()
                || scene.document.edit_mode.is_dirty()
                || scene.document.edit_mode.is_busy()
                || scene.sculpt_has_live_work()
        });
        if needs_guard {
            self.workspace.close_request = Some(CloseRequest::Scene(key));
            self.ui.close_guard_open = true;
        } else {
            self.close_workspace_scene(key);
        }
    }

    /// A closing pane may already have painted an offscreen image this frame.
    /// Keep its texture alive until the next frame starts, after submission.
    pub(in crate::app) fn retain_scene_frame_until_next_pass(&mut self, key: SceneKey) {
        if let Some(texture) = self
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .and_then(|scene| scene.render.rendered.as_ref())
            .map(|frame| frame.texture.clone())
        {
            self.workspace.retired_frame_textures.push(texture);
        }
    }

    fn close_workspace_scene(&mut self, key: SceneKey) {
        let Some(index) = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == key)
        else {
            return;
        };
        // Reserve replacement identities before retiring the last live document.
        let replacement = if self.workspace.scenes.len() == 1 {
            let Ok(new_key) = self.workspace.ids.allocate_scene() else {
                return;
            };
            let Ok(new_pane) = self.workspace.ids.allocate_pane_id() else {
                return;
            };
            Some((new_key, new_pane))
        } else {
            None
        };
        let cancelled_import = self.loader.has_work_for(key);
        self.workspace.history.borrow_mut().close_scope(Some(key));
        self.retain_scene_frame_until_next_pass(key);
        let old = self.workspace.scenes.remove(index);
        if let Some((new_key, new_pane)) = replacement {
            self.seed_scene_brush(new_key);
            if let Some(viewport) = old.render.live_viewport.as_ref() {
                if let Ok(mut viewport) = viewport.lock() {
                    viewport.clear();
                }
            }
            let name = self.ui.locale.tr_with(
                crate::i18n::message_id!("workspace-scene-name"),
                &[("number", &new_key.id.get().to_string())],
            );
            self.workspace.scenes.push(SceneSession::new(
                new_key,
                new_pane,
                name,
                old.render.live_viewport.clone(),
                &self.workspace.history,
            ));
        }
        if let Some(survivor) = self.workspace.scenes.first() {
            self.workspace.input = InputArbiter::new(survivor.target());
            self.workspace.layout = WorkspaceLayout::single(survivor.pane);
        }
        self.workspace.saved_split = None;
        self.workspace.layer_drag = None;
        self.workspace.scene_tab_rects.clear();
        self.workspace.scene_create_rect = None;
        self.loader.reap_retired(&self.workspace.keys());
        if cancelled_import {
            let message = self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-import-cancelled"));
            self.workspace_status(self.workspace.input.active().scene, message);
        }
        if self
            .ui
            .pending_replace_open
            .as_ref()
            .is_some_and(|pending| pending.scene_key == key)
        {
            self.ui.pending_replace_open = None;
        }
    }

    pub(super) fn intercept_workspace_close(&mut self, ctx: &egui::Context) {
        if self.ui.close_confirmed || !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        let mut needs_guard = false;
        for key in self.workspace.keys() {
            if let Some(scene) = self.scene_context(key) {
                needs_guard |= scene.document.has_unsaved_mesh_edits()
                    || scene.document.edit_mode.is_dirty()
                    || scene.document.edit_mode.is_busy()
                    || scene.sculpt_has_live_work();
            }
        }
        if needs_guard {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.workspace.close_request = Some(CloseRequest::Window);
            self.ui.close_guard_open = true;
        }
    }

    pub(super) fn show_workspace_close_guard(&mut self, ctx: &egui::Context) {
        let Some(request) = self.workspace.close_request.clone() else {
            return;
        };
        let keys = match request {
            CloseRequest::Window => self.workspace.keys(),
            CloseRequest::Scene(key) => vec![key],
        };
        let names = self
            .workspace
            .scenes
            .iter()
            .filter(|scene| keys.contains(&scene.key))
            .map(|scene| scene.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let title = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-close-title"));
        let headline = self.ui.locale.tr_with(
            crate::i18n::message_id!("workspace-close-question"),
            &[("scenes", &names)],
        );
        let detail = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-close-detail"));
        let discard = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-discard-close"));
        let response = show_guard_dialog(
            ctx,
            &self.ui.locale,
            GuardDialogSpec {
                id: "workspace-close-guard",
                title: &title,
                headline: &headline,
                note: None,
                detail: &detail,
                destructive_label: &discard,
            },
        );
        if response.action == Some(GuardDialogAction::Save) {
            self.workspace.save_before_close_pending = true;
        }
        let action = response.action.or_else(|| {
            self.workspace
                .save_before_close_pending
                .then_some(GuardDialogAction::Save)
        });
        match action {
            Some(GuardDialogAction::Cancel) => {
                self.workspace.save_before_close_pending = false;
                self.workspace.close_request = None;
                self.ui.close_guard_open = false;
            }
            Some(GuardDialogAction::Save) => {
                for key in keys {
                    let Some(mut scene) = self.scene_context(key) else {
                        continue;
                    };
                    if scene.document.edit_mode.is_busy() || scene.sculpt_has_live_work() {
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                        return;
                    }
                    if matches!(
                        scene.save_edited_layers_flow(),
                        SaveEditedLayersOutcome::Aborted
                    ) {
                        self.workspace.save_before_close_pending = false;
                        return;
                    }
                }
                self.finish_workspace_close(request, ctx);
            }
            Some(GuardDialogAction::Destructive) => self.finish_workspace_close(request, ctx),
            None => {}
        }
    }

    fn finish_workspace_close(&mut self, request: CloseRequest, ctx: &egui::Context) {
        self.workspace.close_request = None;
        self.workspace.save_before_close_pending = false;
        self.ui.close_guard_open = false;
        match request {
            CloseRequest::Scene(key) => self.close_workspace_scene(key),
            CloseRequest::Window => {
                self.ui.close_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl SceneContext<'_> {
    pub(super) fn queue_new_scene(&mut self, side: SplitSide) {
        if self.scene_summaries.len() >= 2 {
            return;
        }
        let Ok(scene) = self.ids.allocate_scene() else {
            return;
        };
        let Ok(pane) = self.ids.allocate_pane_id() else {
            return;
        };
        self.commands.push_back(WorkspaceCommand::CreateScene {
            anchor: self.scene_key,
            scene,
            pane,
            side,
        });
    }

    pub(super) fn queue_transfer(&mut self, layer: SceneMeshId, destination: Option<SceneKey>) {
        self.queue_layers_transfer(LayerIds::one(layer), destination);
    }

    fn queue_layers_transfer(&mut self, layers: LayerIds, destination: Option<SceneKey>) {
        let destination = if let Some(scene) = destination {
            TransferDestination::Existing(scene)
        } else {
            if self.scene_summaries.len() >= 2 {
                return;
            }
            let Ok(scene) = self.ids.allocate_scene() else {
                return;
            };
            let Ok(pane) = self.ids.allocate_pane_id() else {
                return;
            };
            TransferDestination::CreateBeside {
                scene,
                pane,
                side: SplitSide::Right,
            }
        };
        self.commands.push_back(WorkspaceCommand::Transfer {
            source: self.scene_key,
            destination,
            layers,
        });
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn show_scenes_menu(&mut self, ui: &mut egui::Ui) {
        let label = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-scenes"));
        ui.menu_button(label, |ui| {
            for (side, key) in [
                (
                    SplitSide::Left,
                    crate::i18n::message_id!("workspace-new-left"),
                ),
                (
                    SplitSide::Right,
                    crate::i18n::message_id!("workspace-new-right"),
                ),
            ] {
                if ui
                    .add_enabled(
                        self.scene_summaries.len() < 2,
                        egui::Button::new(self.ui.locale.tr(key)),
                    )
                    .clicked()
                {
                    self.queue_new_scene(side);
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .button(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-one-view")),
                )
                .clicked()
            {
                self.commands
                    .push_back(WorkspaceCommand::SetLayout(WorkspaceLayout::single(
                        self.pane_id,
                    )));
                ui.close();
            }
            if self.scene_summaries.len() == 2
                && ui
                    .button(
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-two-views")),
                    )
                    .clicked()
            {
                let left = self.scene_summaries[0].pane;
                let right = self.scene_summaries[1].pane;
                self.commands
                    .push_back(WorkspaceCommand::SetLayout(self.saved_split.unwrap_or(
                        WorkspaceLayout::SideBySide {
                            left,
                            right,
                            ratio: 0.5,
                        },
                    )));
                ui.close();
            }
            if ui
                .add_enabled(
                    self.document.focused_layer_id.is_some(),
                    egui::Button::new(
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-move-layer")),
                    ),
                )
                .clicked()
            {
                if let Some(layer) = self.document.focused_layer_id {
                    let other = self
                        .scene_summaries
                        .iter()
                        .find(|scene| scene.key != self.scene_key)
                        .map(|scene| scene.key);
                    self.queue_transfer(layer, other);
                }
                ui.close();
            }
            ui.separator();
            let all_layers = self
                .document
                .scene
                .as_deref()
                .map(|scene| {
                    scene
                        .meshes()
                        .iter()
                        .map(occluview_core::SceneMesh::id)
                        .collect()
                })
                .and_then(|layers| LayerIds::new(layers).ok());
            if ui
                .add_enabled(
                    all_layers.is_some(),
                    egui::Button::new(
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-move-all")),
                    ),
                )
                .clicked()
            {
                if let Some(layers) = all_layers {
                    let other = self
                        .scene_summaries
                        .iter()
                        .find(|scene| scene.key != self.scene_key)
                        .map(|scene| scene.key);
                    self.queue_layers_transfer(layers, other);
                }
                ui.close();
            }
            if ui
                .button(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-rename")),
                )
                .clicked()
            {
                self.commands
                    .push_back(WorkspaceCommand::RequestRenameScene {
                        scene: self.scene_key,
                    });
                ui.close();
            }
            if ui
                .button(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-close")),
                )
                .clicked()
            {
                self.commands.push_back(WorkspaceCommand::CloseScene {
                    scene: self.scene_key,
                });
                ui.close();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use crate::app::app_test_support::{named_scene, test_app};

    fn add_second(app: &mut OccluViewApp) -> SceneKey {
        app.active_context()
            .expect("live scene")
            .queue_new_scene(SplitSide::Right);
        app.apply_workspace_commands(&egui::Context::default());
        app.workspace.scenes[1].key
    }

    #[test]
    fn scene_limit_keeps_both_documents_and_layout_intact() {
        let mut app = test_app("two-scenes-limit");
        let first = app.workspace.scenes[0].key;
        app.scene_context(first)
            .expect("first")
            .set_scene(named_scene("first", 0.0), true);
        let second = add_second(&mut app);
        app.scene_context(second)
            .expect("second")
            .set_scene(named_scene("second", 5.0), true);
        let keys = app.workspace.keys();
        let layout = app.workspace.layout;
        let third = app.workspace.ids.allocate_scene().expect("identity");
        let pane = app.workspace.ids.allocate_pane_id().expect("pane");
        assert!(app
            .create_workspace_scene(second, third, pane, SplitSide::Left)
            .is_err());
        assert_eq!(app.workspace.keys(), keys);
        assert_eq!(app.workspace.layout, layout);
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("first")
                .meshes()
                .len(),
            1
        );
        assert_eq!(
            app.workspace.scenes[1]
                .document
                .scene
                .as_ref()
                .expect("second")
                .meshes()
                .len(),
            1
        );
    }

    #[test]
    fn one_view_keeps_selected_scene_and_restores_split_geometry() {
        let mut app = test_app("scene-layout-switch");
        let second = add_second(&mut app);
        let pane = app.workspace.scene(second.id).expect("second").pane;
        let split = app.workspace.layout.with_ratio(0.38);
        app.workspace.layout = split;
        app.workspace
            .commands
            .push_back(WorkspaceCommand::SetLayout(WorkspaceLayout::single(pane)));
        app.apply_workspace_commands(&egui::Context::default());
        assert_eq!(app.workspace.active_id(), second.id);
        assert_eq!(app.workspace.layout, WorkspaceLayout::single(pane));
        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(app.workspace.saved_split, Some(split));
        app.workspace
            .commands
            .push_back(WorkspaceCommand::SetLayout(
                app.workspace.saved_split.expect("saved split"),
            ));
        app.apply_workspace_commands(&egui::Context::default());
        assert_eq!(app.workspace.layout, split);
    }

    #[test]
    fn closing_last_scene_retires_its_identity_and_leaves_fresh_empty_document() {
        let mut app = test_app("close-last-scene");
        let old = app.workspace.scenes[0].key;
        app.close_workspace_scene(old);
        assert_eq!(app.workspace.scenes.len(), 1);
        let fresh = app.workspace.scenes[0].key;
        assert_ne!(old.id, fresh.id);
        assert_ne!(old.epoch, fresh.epoch);
        assert!(app.scene_context(old).is_none());
        assert!(app.workspace.scenes[0].document.scene.is_none());
        assert_eq!(app.workspace.input.active().scene, fresh);
    }

    #[test]
    fn closing_pane_keeps_its_painted_texture_alive_until_the_next_frame() {
        let ctx = egui::Context::default();
        let mut app = test_app("retired-pane-texture");
        let second = add_second(&mut app);
        let texture = ctx.load_texture(
            "closing-pane",
            egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let id = texture.id();
        app.workspace.scenes[1].render.rendered = Some(crate::app::state_render::RenderedFrame {
            texture,
            pixels: vec![255; 16],
            size_px: [2, 2],
        });
        let closing = ctx.run_ui(egui::RawInput::default(), |_ui| {
            app.close_workspace_scene(second);
        });
        assert!(!closing.textures_delta.free.contains(&id));
        closing.drop_without_applying_deltas();
        app.workspace.retired_frame_textures.clear();
        let next = ctx.run_ui(egui::RawInput::default(), |_ui| {});
        assert!(next.textures_delta.free.contains(&id));
        next.drop_without_applying_deltas();
    }

    #[test]
    fn closing_hidden_dirty_scene_preserves_it_until_explicit_decision() {
        let mut app = test_app("hidden-scene-close");
        let first = app.workspace.scenes[0].key;
        let scene = named_scene("dirty", 0.0);
        let layer = scene.meshes()[0].id();
        {
            let mut owner = app.scene_context(first).expect("first");
            owner.set_scene(scene, true);
            owner.document.mark_mesh_edits_unsaved(layer);
        }
        let second = add_second(&mut app);
        let pane = app.workspace.scene(second.id).expect("second").pane;
        app.workspace.layout = WorkspaceLayout::single(pane);
        app.request_scene_close(first);
        assert!(
            matches!(app.workspace.close_request, Some(CloseRequest::Scene(key)) if key == first)
        );
        assert!(app.ui.close_guard_open);
        assert_eq!(app.workspace.scenes.len(), 2);
        assert!(app
            .scene_context(first)
            .expect("retained")
            .document
            .has_unsaved_mesh_edits());
    }

    #[test]
    fn right_pane_tools_do_not_reserve_an_invisible_second_layers_panel() {
        let mut app = test_app("shared-layers-tool-footprint");
        let second = add_second(&mut app);
        let model = named_scene("right", 0.0);
        app.scene_context(second)
            .expect("right scene")
            .set_scene(model.clone(), true);
        let workspace = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let right = egui::Rect::from_min_max(egui::pos2(604.0, 30.0), workspace.max);
        let pointer = egui::pos2(680.0, 100.0);
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(workspace),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            },
            |_ui| {
                let mut scene = app.scene_context(second).expect("right scene");
                scene.workspace_rect = Some(workspace);
                assert!(!scene.layers_panel_rect(&ctx, right).contains(pointer));
                assert!(scene.pointer_on_bare_viewport(&ctx, right, pointer));
                assert!(
                    scene
                        .viewport_pointer(&ctx, right, &model, false)
                        .over_viewport
                );
                assert!(scene.scene_contact_bar_rect(&ctx, right).top() < 60.0);
            },
        );
        output.drop_without_applying_deltas();
    }
}
