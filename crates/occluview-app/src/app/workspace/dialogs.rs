//! Workspace-level dialogs and native file drops.

use eframe::egui::vec2;

use crate::app::app_loading::native_drop_paths;
use crate::app::workspace::commands::WorkspaceCommand;
use crate::app::workspace::id::SceneKey;
use crate::app::{egui, OccluViewApp};

impl OccluViewApp {
    pub(super) fn collect_native_drops(&mut self, ctx: &egui::Context) {
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
    pub(super) fn show_workspace_dialogs(&mut self, ctx: &egui::Context) {
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
        let modal = crate::ui::modal_surface::show_information_modal(
            ctx,
            egui::Id::new("workspace-drop-target-dialog"),
            vec2(380.0, 180.0),
            &cancel,
            |ui| {
                ui.label(
                    egui::RichText::new(&title)
                        .strong()
                        .size(17.0)
                        .color(crate::ui::ui_theme::text()),
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
        let modal = crate::ui::modal_surface::show_information_modal(
            ctx,
            egui::Id::new("workspace-rename-dialog"),
            vec2(380.0, 160.0),
            &cancel,
            |ui| {
                let title_response = ui.label(
                    egui::RichText::new(&title)
                        .strong()
                        .size(17.0)
                        .color(crate::ui::ui_theme::text()),
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
}
