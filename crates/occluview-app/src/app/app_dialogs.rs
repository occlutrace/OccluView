use super::app_settings_window::settings_popup_id;
use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use super::{
    load_app_logo_color_image, recent_scene_hover, recent_scene_label, status_overlay_rect,
    PathBuf, OPEN_DIALOG_EXTENSIONS,
};
use crate::icons::AppIcon;
use crate::measure_overlay::{toolbar_toggle, ToolbarToggle};
use crate::measure_tool::{self, MeasureMode};
use crate::recent_files::RecentFiles;
use crate::ui_theme;
use eframe::egui;

const RECENT_FILES_POPUP_ID: &str = "recent-files-dropdown-v1";

pub(super) fn recent_files_popup_id() -> egui::Id {
    egui::Id::new(RECENT_FILES_POPUP_ID)
}

pub(super) enum RecentFilesAction {
    Open(Vec<PathBuf>),
    Clear,
}

pub(super) fn show_recent_files_popup(
    trigger: &egui::Response,
    recent_files: &RecentFiles,
) -> Option<RecentFilesAction> {
    let action = egui::Popup::from_toggle_button_response(trigger)
        .id(recent_files_popup_id())
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(220.0);
            for entry in recent_files.entries() {
                if ui
                    .button(recent_scene_label(entry))
                    .on_hover_text(recent_scene_hover(entry))
                    .clicked()
                {
                    return Some(RecentFilesAction::Open(entry.paths().to_vec()));
                }
            }
            ui.separator();
            ui.button("Clear recent")
                .clicked()
                .then_some(RecentFilesAction::Clear)
        })
        .and_then(|response| response.inner);
    if action.is_some() {
        egui::Popup::close_id(&trigger.ctx, recent_files_popup_id());
    }
    action
}

pub(super) fn show_settings_toolbar_toggle(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    toolbar_toggle(
        ui,
        ToolbarToggle::new(
            AppIcon::Settings,
            "Settings",
            enabled,
            egui::Popup::is_id_open(ui.ctx(), settings_popup_id()),
            "Open preferences",
        ),
    )
}

impl OccluViewApp {
    /// Draw the top toolbar and dispatch its actions after layout.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_toolbar(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        if self.close_guard_open
            || self.pending_replace_open.is_some()
            || self.app_error.is_some()
            || self.information_dialog.is_open()
        {
            egui::Popup::close_id(&ctx, settings_popup_id());
        }
        // The only wired shortcut; its tooltip hint is therefore real.
        let open_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        let mut do_open = ctx.input_mut(|input| input.consume_shortcut(&open_shortcut));
        let mut do_add = false;
        let mut recent_to_open: Option<Vec<PathBuf>> = None;
        let mut clear_recent = false;
        let mut toggle_cut_view = false;
        let mut toggle_measure: Option<MeasureMode> = None;
        let mut toggle_align = false;

        egui::Panel::top("toolbar")
            .exact_size(ui_theme::MENUBAR_HEIGHT_PX)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(247, 248, 250))
                    .stroke(egui::Stroke::new(1.0_f32, ui_theme::hairline()))
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(root_ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;

                    let open_hint = format!(
                        "Open 3D files ({})",
                        ui.ctx().format_shortcut(&open_shortcut)
                    );
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(AppIcon::Open, "Open", true, false, &open_hint),
                    )
                    .clicked()
                    {
                        do_open = true;
                    }
                    // Recent files use a chevron popup attached to Open.
                    ui.add_enabled_ui(!self.recent_files.is_empty(), |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::click());
                        crate::icons::paint(
                            ui.painter(),
                            rect,
                            AppIcon::ChevronDown,
                            if response.hovered() {
                                ui_theme::TEXT
                            } else {
                                ui_theme::TEXT_WEAK
                            },
                        );
                        let response = response.on_hover_text("Recent files");
                        if let Some(action) = show_recent_files_popup(&response, &self.recent_files)
                        {
                            match action {
                                RecentFilesAction::Open(paths) => recent_to_open = Some(paths),
                                RecentFilesAction::Clear => clear_recent = true,
                            }
                        }
                    });
                    ui.add_space(4.0);
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Add,
                            "Add",
                            self.scene.is_some(),
                            false,
                            "Add more files to the current scene",
                        ),
                    )
                    .clicked()
                    {
                        do_add = true;
                    }

                    toolbar_divider(ui);

                    let can_cut = self.can_render_cut_view();
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Cut,
                            "Cut view",
                            can_cut,
                            self.cut_view.is_active(),
                            if can_cut {
                                "Slice the model along a plane"
                            } else {
                                "Cut view needs a visible layer"
                            },
                        ),
                    )
                    .clicked()
                    {
                        toggle_cut_view = true;
                    }

                    toolbar_divider(ui);

                    let edit_session_active = self.edit_mode.has_active_session();
                    let has_pickable_layer = self.has_measurable_layer();
                    let can_measure =
                        measure_tool::measure_menu_enabled(has_pickable_layer, edit_session_active);
                    let entries = [
                        (
                            AppIcon::Ruler,
                            MeasureMode::Ruler,
                            "Ruler",
                            "Measure a distance: click two points on the model",
                        ),
                        (
                            AppIcon::Thickness,
                            MeasureMode::Thickness,
                            "Thickness",
                            "Probe the local wall thickness: click a point on the shell",
                        ),
                    ];
                    for (icon, mode, label, hint) in entries {
                        let tooltip = if edit_session_active {
                            "Finish or cancel the mesh edit session first"
                        } else if !has_pickable_layer {
                            "Measuring needs a visible mesh layer"
                        } else {
                            hint
                        };
                        let active = self.measure.mode() == Some(mode);
                        if toolbar_toggle(
                            ui,
                            ToolbarToggle::new(icon, label, can_measure, active, tooltip),
                        )
                        .clicked()
                        {
                            toggle_measure = Some(mode);
                        }
                    }

                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Align,
                            "Align",
                            can_measure,
                            self.align_active(),
                            "Bring two scans together: click a point on each",
                        ),
                    )
                    .clicked()
                    {
                        toggle_align = true;
                    }

                    let can_edit_mesh = self.scene.is_some()
                        && self
                            .scene
                            .as_ref()
                            .is_some_and(|s| s.meshes().iter().any(|m| !m.mesh.is_point_cloud()));
                    let edit_active = self.edit_mode.has_active_session();
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::EditMesh,
                            "Edit",
                            can_edit_mesh,
                            edit_active,
                            if edit_active {
                                "Mesh editor is open"
                            } else {
                                "Edit mesh: selection and sculpting"
                            },
                        ),
                    )
                    .clicked()
                    {
                        // Pressing it while the editor is already open is the
                        // toggle's business, not this button's: opening a second
                        // session over a live one would discard the first one's
                        // selection.
                        if let (false, Some(scene)) = (edit_active, self.scene.clone()) {
                            for entry in scene.meshes() {
                                if !entry.mesh.is_point_cloud() && entry.visible {
                                    let _ = self.edit_mode.begin_face_selection(entry, &scene);
                                    break;
                                }
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = show_settings_toolbar_toggle(ui, !self.close_guard_open);
                        if response.clicked() {
                            self.information_dialog = InformationDialog::None;
                        }
                        self.show_settings_popup(&response);
                    });
                });
            });

        if toggle_align {
            if self.align_active() {
                // Turning the tool off is a close, and a close reverts. Done,
                // inside the window, is what keeps an alignment.
                self.cancel_align_session(&ctx);
            } else {
                self.arm_align_tool(&ctx);
            }
        }
        if toggle_cut_view {
            if self.cut_view.is_active() {
                self.cut_view.disable();
            } else {
                // The viewport-owning tools are mutually exclusive: entering
                // the cut view stands the measurement tool down cleanly.
                self.measure.disarm();
                self.cut_view.enable();
            }
            self.needs_render = true;
        }
        // Arming a measurement or the cut view closes Align, the same way
        // arming Align closes them. Two tools cannot share the primary click.
        if (toggle_measure.is_some() || toggle_cut_view) && self.align_active() {
            self.cancel_align_session(&ctx);
        }
        if let Some(clicked) = toggle_measure {
            let (next, disable_cut) = measure_tool::apply_menu_toggle(
                self.measure.mode(),
                self.cut_view.is_active(),
                clicked,
            );
            if disable_cut {
                self.cut_view.disable();
                self.needs_render = true;
            }
            match next {
                Some(mode) => self.measure.arm(mode),
                None => self.measure.disarm(),
            }
            ctx.request_repaint();
        }

        if do_open {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("3D files", OPEN_DIALOG_EXTENSIONS)
                .pick_files()
            {
                self.replace_paths(&paths, "open");
            }
        }
        if do_add {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("3D files", OPEN_DIALOG_EXTENSIONS)
                .pick_files()
            {
                self.append_paths(&paths, "add");
            }
        }
        if clear_recent {
            self.recent_files.clear();
            self.save_recent_files();
        }
        if let Some(paths) = recent_to_open {
            self.replace_paths(&paths, "recent");
        }
    }

    pub(super) fn app_logo_texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.app_logo.is_none() {
            if let Some(color_image) = load_app_logo_color_image() {
                self.app_logo = Some(ctx.load_texture(
                    "occluview-app-logo",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        self.app_logo.as_ref()
    }

    pub(super) fn show_status_overlay(&self, ui: &mut egui::Ui, viewport_rect: egui::Rect) {
        if self.status_message.is_none() {
            return;
        }
        let rect = status_overlay_rect(viewport_rect);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_rgba_unmultiplied(248, 250, 252, 214))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(26, 32, 44, 30),
                ))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    if let Some(message) = &self.status_message {
                        ui.label(message);
                    }
                });
        });
    }

    /// Cancel a window close before eframe can act on it without a visible UI pass.
    pub(super) fn intercept_unsaved_close(&mut self, ctx: &egui::Context) {
        intercept_unsaved_close_request(
            ctx,
            self.has_unsaved_mesh_edits(),
            self.close_confirmed,
            &mut self.close_guard_open,
        );
    }

    /// Ask how to resolve an intercepted close with unsaved mesh edits.
    /// "Save…" exports each edited layer before closing; the destructive path
    /// re-issues the close only after explicit consent.
    pub(super) fn show_unsaved_close_guard(&mut self, ctx: &egui::Context) {
        if !self.close_guard_open {
            return;
        }
        let edited_count = self.unsaved_edit_layer_ids.len().max(1);
        let mut do_save = false;
        let headline = if edited_count == 1 {
            "1 edited layer has not been saved to disk."
        } else {
            "Edited layers have not been saved to disk."
        };
        let note =
            (edited_count > 1).then(|| format!("{edited_count} edited layers are affected."));
        let response = show_guard_dialog(
            ctx,
            GuardDialogSpec {
                id: "unsaved-mesh-edits-guard",
                title: "Unsaved mesh edits",
                headline,
                note: note.as_deref(),
                detail: "Save exports each edited layer (PLY, STL, or OBJ) and then closes.",
                destructive_label: "Close without saving",
            },
        );
        match response.action {
            Some(GuardDialogAction::Save) => do_save = true,
            Some(GuardDialogAction::Destructive) => {
                self.close_confirmed = true;
                self.close_guard_open = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(GuardDialogAction::Cancel) => self.close_guard_open = false,
            None => {}
        }
        if do_save {
            match self.save_edited_layers_flow() {
                super::app_mesh_export::SaveEditedLayersOutcome::AllSaved
                | super::app_mesh_export::SaveEditedLayersOutcome::NothingToSave => {
                    self.close_confirmed = true;
                    self.close_guard_open = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // A cancelled dialog or failed write keeps the app open —
                // never close on top of edits the operator believes saved.
                super::app_mesh_export::SaveEditedLayersOutcome::Aborted => {}
            }
        }
    }

    /// Guard an incoming REPLACE open (parked in `pending_replace_open`) while a
    /// live edit session is dirty or unsaved edits exist. Mirrors the
    /// close-guard wording: "Save…" writes each edited layer then opens,
    pub(super) fn guard_pending_replace_open(&mut self, ctx: &egui::Context) {
        if self.pending_replace_open.is_none() {
            return;
        }
        // Never stack over the close guard; it takes precedence (the app is
        // trying to exit). The parked open waits until that resolves.
        if self.close_guard_open {
            return;
        }
        let session_layer = self.active_session_layer_label();
        let edited_count = self.unsaved_edit_layer_ids.len();
        let mut do_save = false;
        let mut do_discard = false;
        let mut do_cancel = false;
        let headline = if let Some(layer) = &session_layer {
            format!("An edit session is active on {layer}.")
        } else if edited_count <= 1 {
            "1 edited layer has unsaved changes.".to_string()
        } else {
            format!("{edited_count} edited layers have unsaved changes.")
        };
        let response = show_guard_dialog(
            ctx,
            GuardDialogSpec {
                id: "edit-in-progress-guard",
                title: "Edit in progress",
                headline: &headline,
                note: None,
                detail: "Opening a scene closes the session and discards edits not saved to disk.",
                destructive_label: "Discard and open",
            },
        );
        match response.action {
            Some(GuardDialogAction::Save) => do_save = true,
            Some(GuardDialogAction::Destructive) => do_discard = true,
            Some(GuardDialogAction::Cancel) => do_cancel = true,
            None => {}
        }

        if do_cancel {
            // Drop the parked open; keep the current scene and session.
            self.pending_replace_open = None;
            return;
        }
        if do_discard {
            if let Some(pending) = self.pending_replace_open.take() {
                self.replace_paths_confirmed(&pending.paths, pending.source);
            }
            return;
        }
        if do_save {
            match self.save_edited_layers_flow() {
                super::app_mesh_export::SaveEditedLayersOutcome::AllSaved
                | super::app_mesh_export::SaveEditedLayersOutcome::NothingToSave => {
                    if let Some(pending) = self.pending_replace_open.take() {
                        self.replace_paths_confirmed(&pending.paths, pending.source);
                    }
                }
                // A cancelled export dialog or a failed write keeps the open
                // parked so the operator can retry — never open on top of edits
                // they believe are saved.
                super::app_mesh_export::SaveEditedLayersOutcome::Aborted => {}
            }
        }
    }

    /// Human label for the layer a live edit session targets, for the open
    /// guard message. `None` when no session is active (the guard fired only on
    /// unsaved edits left by a closed session) or the layer has since left the
    /// scene.
    fn active_session_layer_label(&self) -> Option<String> {
        let id = self.edit_mode.session_layer_id()?;
        let scene = self.scene.as_ref()?;
        let index = scene.meshes().iter().position(|entry| entry.id() == id)?;
        Some(crate::layers_overlay::layer_label(
            &self.current_paths,
            &scene.meshes()[index],
            index,
        ))
    }

    pub(super) fn show_error_dialog(&mut self, ctx: &egui::Context) {
        let Some(error) = self.app_error.clone() else {
            return;
        };
        let mut open = true;
        let mut close_clicked = false;
        egui::Window::new(error.title.as_str())
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_size([460.0, 260.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    crate::icons::paint(ui.painter(), icon_rect, AppIcon::Error, ui_theme::DANGER);
                    ui.label(
                        egui::RichText::new(error.summary.as_str())
                            .strong()
                            .size(13.5),
                    );
                });
                ui.add_space(8.0);
                let mut details = error.details.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut details)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                    if ui.button("Copy Details").clicked() {
                        ui.ctx().copy_text(error.details.clone());
                    }
                });
            });
        if !open || close_clicked {
            self.app_error = None;
        }
    }
}

fn intercept_unsaved_close_request(
    ctx: &egui::Context,
    has_unsaved_mesh_edits: bool,
    close_confirmed: bool,
    close_guard_open: &mut bool,
) {
    if ctx.input(|input| input.viewport().close_requested())
        && has_unsaved_mesh_edits
        && !close_confirmed
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        *close_guard_open = true;
        ctx.request_repaint();
    }
}

struct GuardDialogSpec<'a> {
    id: &'static str,
    title: &'a str,
    headline: &'a str,
    note: Option<&'a str>,
    detail: &'a str,
    destructive_label: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuardDialogAction {
    Save,
    Destructive,
    Cancel,
}

struct GuardDialogResponse {
    action: Option<GuardDialogAction>,
}

fn show_guard_dialog(ctx: &egui::Context, spec: GuardDialogSpec<'_>) -> GuardDialogResponse {
    const CONTENT_WIDTH: f32 = 416.0;
    let mut open = true;
    let mut action = None;
    egui::Window::new(spec.title)
        .id(egui::Id::new(spec.id))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .constrain_to(ctx.content_rect().shrink(8.0))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(CONTENT_WIDTH);
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), icon_rect, AppIcon::Warn, ui_theme::WARNING);
                ui.label(egui::RichText::new(spec.headline).strong());
            });
            if let Some(note) = spec.note {
                ui.label(note);
            }
            ui.label(egui::RichText::new(spec.detail).weak().size(11.0));
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.add(dialog_primary_button("Save…")).clicked() {
                        action = Some(GuardDialogAction::Save);
                    }
                    if ui.button(spec.destructive_label).clicked() {
                        action = Some(GuardDialogAction::Destructive);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(GuardDialogAction::Cancel);
                    }
                },
            );
        });
    if !open && action.is_none() {
        action = Some(GuardDialogAction::Cancel);
    }
    GuardDialogResponse { action }
}

/// Slim vertical hairline between toolbar groups.
fn toolbar_divider(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 18.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        egui::Rangef::new(rect.top(), rect.bottom()),
        egui::Stroke::new(1.0_f32, ui_theme::hairline()),
    );
    ui.add_space(6.0);
}

/// Primary dialog action.
fn dialog_primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(label)
            .strong()
            .color(egui::Color32::WHITE),
    )
    .fill(ui_theme::ACCENT)
    .corner_radius(ui_theme::RADIUS_CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_window_close_is_cancelled_before_ui_can_run() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        let root_viewport = input.viewports.get_mut(&egui::ViewportId::ROOT);
        assert!(root_viewport.is_some(), "root viewport exists");
        let Some(root_viewport) = root_viewport else {
            return;
        };
        root_viewport.events.push(egui::ViewportEvent::Close);
        let mut close_guard_open = false;

        let output = ctx.run_logic(&input, |ctx| {
            intercept_unsaved_close_request(ctx, true, false, &mut close_guard_open);
        });

        assert!(close_guard_open, "unsaved close must open the guard");
        assert!(
            output
                .viewport_commands
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|commands| { commands.contains(&egui::ViewportCommand::CancelClose) }),
            "logic-only close must be cancelled before eframe exits"
        );
    }

    #[test]
    fn production_guard_dialog_stays_content_sized() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            let _ = show_guard_dialog(
                ui.ctx(),
                GuardDialogSpec {
                    id: "guard-size-contract",
                    title: "Unsaved mesh edits",
                    headline: "Edited layers have not been saved to disk.",
                    note: Some("3 edited layers are affected."),
                    detail: "Save exports each edited layer and then closes.",
                    destructive_label: "Close without saving",
                },
            );
        })
        .drop_without_applying_deltas();

        let Some(rect) =
            ctx.memory(|memory| memory.area_rect(egui::Id::new("guard-size-contract")))
        else {
            return Err(anyhow::anyhow!("the production guard should render"));
        };
        assert!(rect.width() <= 460.0, "guard width was {}", rect.width());
        assert!(rect.height() <= 150.0, "guard height was {}", rect.height());
        Ok(())
    }
}
