use super::{
    load_app_logo_color_image, recent_scene_hover, recent_scene_label, status_overlay_rect,
    PathBuf, OPEN_DIALOG_EXTENSIONS,
};
use super::{AboutWindowState, OccluViewApp};
use crate::measure_tool::{self, MeasureMode};
use crate::mesh_editor_icons::MeasureIcon;
use crate::ui_theme;
use eframe::egui;

impl OccluViewApp {
    /// One flat action bar instead of a windows-style File/View/Help menubar:
    /// direct icon+label buttons for the handful of real actions, a recent
    /// dropdown next to Open, the cut-view toggle inline, and version + About
    /// tucked on the right. Every action keeps its tooltip and shortcut.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_toolbar_impl(&mut self, ctx: &egui::Context) {
        // The only wired shortcut; its tooltip hint is therefore real.
        let open_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        let mut do_open = ctx.input_mut(|input| input.consume_shortcut(&open_shortcut));
        let mut do_add = false;
        let mut recent_to_open: Option<Vec<PathBuf>> = None;
        let mut clear_recent = false;
        let mut toggle_cut_view = false;
        let mut toggle_measure: Option<MeasureMode> = None;

        egui::TopBottomPanel::top("toolbar")
            .exact_height(ui_theme::MENUBAR_HEIGHT_PX)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(247, 248, 250))
                    .stroke(egui::Stroke::new(1.0, ui_theme::hairline()))
                    .inner_margin(egui::Margin::symmetric(8.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;

                    let open_hint = format!(
                        "Open 3D files ({})",
                        ui.ctx().format_shortcut(&open_shortcut)
                    );
                    if toolbar_action(ui, "🗁 Open", true, &open_hint) {
                        do_open = true;
                    }
                    // Recent files live in a slim dropdown fused to Open.
                    ui.add_enabled_ui(!self.recent_files.is_empty(), |ui| {
                        let recent = ui.menu_button(egui::RichText::new("⏷").size(11.0), |ui| {
                            ui.set_min_width(220.0);
                            for entry in self.recent_files.entries() {
                                if ui
                                    .button(recent_scene_label(entry))
                                    .on_hover_text(recent_scene_hover(entry))
                                    .clicked()
                                {
                                    recent_to_open = Some(entry.paths().to_vec());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear recent").clicked() {
                                clear_recent = true;
                                ui.close_menu();
                            }
                        });
                        recent.response.on_hover_text("Recent files");
                    });
                    ui.add_space(4.0);
                    if toolbar_action(
                        ui,
                        "🗐 Add",
                        self.scene.is_some(),
                        "Add more files to the current scene",
                    ) {
                        do_add = true;
                    }

                    toolbar_divider(ui);

                    let can_cut = self.can_render_cut_view();
                    let cut_active = self.cut_view.is_active();
                    let cut = ui
                        .add_enabled(
                            can_cut,
                            egui::SelectableLabel::new(
                                cut_active,
                                egui::RichText::new("✂ Cut view").size(12.5),
                            ),
                        )
                        .on_hover_text("Slice the model along a plane")
                        .on_disabled_hover_text("Cut view needs a visible layer");
                    if cut.clicked() {
                        toggle_cut_view = true;
                    }

                    toolbar_divider(ui);

                    // Measure group: two direct toggles (this toolbar is flat
                    // by owner decision — no dropdown menus), each with a
                    // hand-painted vector glyph and a lit active state, exactly
                    // like the sibling Cut view tool.
                    let edit_session_active = self.edit_mode.has_active_session();
                    let has_pickable_layer = self.has_measurable_layer();
                    let can_measure =
                        measure_tool::measure_menu_enabled(has_pickable_layer, edit_session_active);
                    let entries = [
                        (
                            MeasureIcon::Ruler,
                            MeasureMode::Ruler,
                            "Ruler",
                            "Measure a distance: click two points on the model",
                        ),
                        (
                            MeasureIcon::Thickness,
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
                        if crate::measure_overlay::toolbar_toggle(
                            ui,
                            icon,
                            label,
                            can_measure,
                            active,
                            tooltip,
                        ) {
                            toggle_measure = Some(mode);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if toolbar_action(ui, "ℹ", true, "About OccluView") {
                            self.about_window = AboutWindowState::Open;
                        }
                    });
                });
            });

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
            egui::Frame::none()
                .fill(egui::Color32::from_rgba_unmultiplied(248, 250, 252, 214))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(26, 32, 44, 30),
                ))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                .show(ui, |ui| {
                    if let Some(message) = &self.status_message {
                        ui.label(message);
                    }
                });
        });
    }

    /// Intercept window close while unsaved mesh edits exist: cancel the close
    /// and ask. "Save…" walks each edited layer through the export dialog and
    /// closes once everything is on disk; "Close without saving" re-issues the
    /// close with consent given.
    pub(super) fn guard_unsaved_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && self.has_unsaved_mesh_edits
            && !self.close_confirmed
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_guard_open = true;
        }
        if !self.close_guard_open {
            return;
        }
        let edited_count = self.unsaved_edit_layer_ids.len().max(1);
        let mut open = true;
        let mut do_save = false;
        egui::Window::new("Unsaved mesh edits")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(if edited_count == 1 {
                    "1 edited layer has not been saved to disk.".to_string()
                } else {
                    format!("{edited_count} edited layers have not been saved to disk.")
                });
                ui.label(
                    egui::RichText::new(
                        "Save exports each edited layer (PLY, STL, or OBJ) and then closes.",
                    )
                    .weak()
                    .size(11.0),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let save = egui::Button::new(egui::RichText::new("Save…").strong());
                    if ui.add(save).clicked() {
                        do_save = true;
                    }
                    if ui.button("Close without saving").clicked() {
                        self.close_confirmed = true;
                        self.close_guard_open = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.close_guard_open = false;
                    }
                });
            });
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
        if !open {
            self.close_guard_open = false;
        }
    }

    /// Guard an incoming REPLACE open (parked in `pending_replace_open`) while a
    /// live edit session is dirty or unsaved edits exist. Mirrors the
    /// close-guard wording: "Save…" writes each edited layer then opens,
    /// "Discard and open" starts the replace (the session falls away only when
    /// the new scene actually loads), and "Cancel" drops the parked open and
    /// leaves the scene and session untouched. A queued handoff/open is held
    /// here, never silently dropped.
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
        let mut open = true;
        let mut do_save = false;
        let mut do_discard = false;
        let mut do_cancel = false;
        egui::Window::new("Edit in progress")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(layer) = &session_layer {
                    ui.label(format!("An edit session is active on {layer}."));
                } else if edited_count <= 1 {
                    ui.label("1 edited layer has unsaved changes.");
                } else {
                    ui.label(format!(
                        "{edited_count} edited layers have unsaved changes."
                    ));
                }
                ui.label(
                    egui::RichText::new(
                        "Opening a scene closes the session and discards edits not saved to disk.",
                    )
                    .weak()
                    .size(11.0),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let save = egui::Button::new(egui::RichText::new("Save…").strong());
                    if ui.add(save).clicked() {
                        do_save = true;
                    }
                    if ui.button("Discard and open").clicked() {
                        do_discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                });
            });

        if do_cancel || !open {
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

    pub(super) fn show_about_window(&mut self, ctx: &egui::Context) {
        if self.about_window != AboutWindowState::Open {
            return;
        }
        let logo = self.app_logo_texture(ctx).cloned();
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));

        // Dimmed backdrop makes it a centered modal; clicking it dismisses.
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("about_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(88));
                if response.clicked() {
                    close = true;
                }
            });

        egui::Window::new("About OccluView")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.add_space(22.0);
                ui.vertical_centered(|ui| {
                    if let Some(logo) = &logo {
                        ui.add(egui::Image::new((logo.id(), egui::vec2(64.0, 64.0))));
                    }
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("OccluView")
                            .size(21.0)
                            .strong()
                            .color(ui_theme::TEXT),
                    );
                    ui.label(
                        egui::RichText::new("3D viewer for dental scans")
                            .color(ui_theme::TEXT_WEAK),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(concat!(
                            "Version ",
                            env!("CARGO_PKG_VERSION"),
                            " · Apache-2.0"
                        ))
                        .size(11.0)
                        .color(ui_theme::TEXT_MUTED),
                    );
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.hyperlink_to("occlutrace.ai", "https://occlutrace.ai");
                        ui.label(egui::RichText::new("·").color(ui_theme::TEXT_MUTED));
                        ui.hyperlink_to("GitHub", "https://github.com/occlutrace/OccluView");
                    });
                    ui.add_space(16.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    ui.add_space(6.0);
                });
            });

        if close {
            self.about_window = AboutWindowState::Closed;
        }
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
                ui.label(error.summary.as_str());
                ui.add_space(8.0);
                let mut details = error.details.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut details)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
                ui.horizontal(|ui| {
                    if ui.button("Copy Details").clicked() {
                        ui.ctx().copy_text(error.details.clone());
                    }
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
            });
        if !open || close_clicked {
            self.app_error = None;
        }
    }
}

/// One flat toolbar action: icon+label, quiet frame, hover tooltip.
fn toolbar_action(ui: &mut egui::Ui, label: &str, enabled: bool, tooltip: &str) -> bool {
    let button = egui::Button::new(egui::RichText::new(label).size(12.5)).frame(false);
    ui.add_enabled(enabled, button)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
        .clicked()
}

/// Slim vertical hairline between toolbar groups.
fn toolbar_divider(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 16.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        egui::Rangef::new(rect.top(), rect.bottom()),
        egui::Stroke::new(1.0, ui_theme::hairline()),
    );
    ui.add_space(6.0);
}
