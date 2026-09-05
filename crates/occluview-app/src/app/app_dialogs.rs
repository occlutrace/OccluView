use super::app_guard_dialog::{show_guard_dialog, GuardDialogAction, GuardDialogSpec};
use super::app_help::{render_contextual_hint, show_help_toolbar_toggle};
use super::app_recent_popup::RecentFilesAction;
use super::app_settings_panel::{settings_popup_id, show_settings_toolbar_toggle};
use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use super::{load_app_logo_color_image, status_overlay_rect, PathBuf, OPEN_DIALOG_EXTENSIONS};
use crate::icons::AppIcon;
use crate::measure_overlay::{toolbar_toggle, ToolbarToggle};
use crate::measure_tool::{self, MeasureMode};
use crate::ui_theme;
use eframe::egui;

#[cfg(test)]
pub(super) use super::app_recent_popup::recent_files_popup_id;
pub(super) use super::app_recent_popup::show_recent_files_popup;

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
        // The only wired file shortcut; its tooltip hint is therefore real.
        let open_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        // Plain-letter tool hotkeys, mirrored in each button's tooltip. Guarded
        // by the modal check below so a dialog never leaks keystrokes into a
        // tool toggle behind it.
        let cut_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::C);
        let ruler_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::M);
        let thickness_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::T);
        let align_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::A);
        let edit_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::E);
        let mut toggle_cut_view = false;
        let mut toggle_measure: Option<MeasureMode> = None;
        let mut toggle_align = false;
        let mut toggle_edit_mesh = false;
        if !self.modal_dialog_open() && !ctx.egui_wants_keyboard_input() {
            let consume = |ctx: &egui::Context, shortcut: &egui::KeyboardShortcut| {
                ctx.input_mut(|input| input.consume_key(shortcut.modifiers, shortcut.logical_key))
            };
            if consume(&ctx, &cut_shortcut) {
                toggle_cut_view = true;
            }
            if consume(&ctx, &ruler_shortcut) {
                toggle_measure = Some(MeasureMode::Ruler);
            }
            if consume(&ctx, &thickness_shortcut) {
                toggle_measure = Some(MeasureMode::Thickness);
            }
            if consume(&ctx, &align_shortcut) {
                toggle_align = true;
            }
            if consume(&ctx, &edit_shortcut) {
                toggle_edit_mesh = true;
            }
        }
        let mut do_add = false;
        let mut do_open = ctx.input_mut(|input| input.consume_shortcut(&open_shortcut));
        let mut recent_to_open: Option<Vec<PathBuf>> = None;
        let mut clear_recent = false;

        egui::Panel::top("toolbar")
            .exact_size(ui_theme::MENUBAR_HEIGHT_PX)
            .frame(
                egui::Frame::default()
                    .fill(ui_theme::toolbar_fill())
                    .stroke(egui::Stroke::new(1.0_f32, ui_theme::hairline()))
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(root_ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;

                    // Shortcut glyphs interpolate as catalog variables and stay invariant.
                    let open_shortcut_text = ui.ctx().format_shortcut(&open_shortcut);
                    let open_hint = self
                        .locale
                        .tr_with("toolbar-open-hint", &[("shortcut", &open_shortcut_text)]);
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Open,
                            &self.locale.tr("toolbar-open-label"),
                            true,
                            false,
                            &open_hint,
                        ),
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
                                ui_theme::text()
                            } else {
                                ui_theme::text_weak()
                            },
                        );
                        let response =
                            response.on_hover_text(self.locale.tr("toolbar-recent-hint"));
                        if let Some(action) =
                            show_recent_files_popup(&response, &self.recent_files, &self.locale)
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
                            &self.locale.tr("toolbar-add-label"),
                            self.scene.is_some(),
                            false,
                            &self.locale.tr("toolbar-add-hint"),
                        ),
                    )
                    .clicked()
                    {
                        do_add = true;
                    }

                    toolbar_divider(ui);

                    let can_cut = self.can_render_cut_view();
                    let cut_shortcut_text = ui.ctx().format_shortcut(&cut_shortcut);
                    let cut_hint = if can_cut {
                        self.locale
                            .tr_with("toolbar-cut-hint", &[("shortcut", &cut_shortcut_text)])
                    } else {
                        self.locale.tr("toolbar-cut-unavailable")
                    };
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Cut,
                            &self.locale.tr("toolbar-cut-label"),
                            can_cut,
                            self.cut_view.is_active(),
                            &cut_hint,
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
                            self.locale.tr("toolbar-ruler-label"),
                            "toolbar-ruler-hint",
                            ruler_shortcut,
                        ),
                        (
                            AppIcon::Thickness,
                            MeasureMode::Thickness,
                            self.locale.tr("toolbar-thickness-label"),
                            "toolbar-thickness-hint",
                            thickness_shortcut,
                        ),
                    ];
                    for (icon, mode, label, hint_key, shortcut) in entries {
                        let shortcut_text = ui.ctx().format_shortcut(&shortcut);
                        let tooltip = if edit_session_active {
                            self.locale.tr("toolbar-measure-blocked")
                        } else if !has_pickable_layer {
                            self.locale.tr("toolbar-measure-needs-layer")
                        } else {
                            self.locale
                                .tr_with(hint_key, &[("shortcut", &shortcut_text)])
                        };
                        let active = self.measure.mode() == Some(mode);
                        if toolbar_toggle(
                            ui,
                            ToolbarToggle::new(icon, &label, can_measure, active, &tooltip),
                        )
                        .clicked()
                        {
                            toggle_measure = Some(mode);
                        }
                    }

                    let align_shortcut_text = ui.ctx().format_shortcut(&align_shortcut);
                    let align_hint = self
                        .locale
                        .tr_with("toolbar-align-hint", &[("shortcut", &align_shortcut_text)]);
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Align,
                            &self.locale.tr("toolbar-align-label"),
                            can_measure,
                            self.align_active(),
                            &align_hint,
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
                    let edit_shortcut_text = ui.ctx().format_shortcut(&edit_shortcut);
                    let edit_hint = if edit_active {
                        self.locale.tr("toolbar-edit-open")
                    } else {
                        self.locale
                            .tr_with("toolbar-edit-hint", &[("shortcut", &edit_shortcut_text)])
                    };
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::EditMesh,
                            &self.locale.tr("toolbar-edit-label"),
                            can_edit_mesh,
                            edit_active,
                            &edit_hint,
                        ),
                    )
                    .clicked()
                    {
                        toggle_edit_mesh = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let help_response =
                            show_help_toolbar_toggle(ui, !self.close_guard_open, &self.locale);
                        if help_response.clicked() {
                            self.information_dialog = InformationDialog::KeyboardMouse;
                        }
                        let response =
                            show_settings_toolbar_toggle(ui, !self.close_guard_open, &self.locale);
                        if response.clicked() {
                            self.information_dialog = InformationDialog::None;
                        }
                        self.show_settings_popup(&response);
                    });
                });
            });

        // The Edit button and its E hotkey share one entry path. Opening while
        // the editor is already open is the toggle's business, not this
        // button's: a second session over a live one would discard the first
        // one's selection.
        if toggle_edit_mesh {
            let edit_active = self.edit_mode.has_active_session();
            let can_edit_mesh = self.scene.is_some()
                && self
                    .scene
                    .as_ref()
                    .is_some_and(|s| s.meshes().iter().any(|m| !m.mesh.is_point_cloud()));
            if let (false, true, Some(scene)) = (edit_active, can_edit_mesh, self.scene.clone()) {
                for entry in scene.meshes() {
                    if !entry.mesh.is_point_cloud() && entry.visible {
                        let _ = self.edit_mode.begin_face_selection(entry, &scene);
                        break;
                    }
                }
            }
        }

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
            self.open_files_dialog();
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

    /// The native Open dialog, shared by the toolbar Open button, the Ctrl+O
    /// shortcut and the empty-viewport call to action.
    pub(super) fn open_files_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("3D files", OPEN_DIALOG_EXTENSIONS)
            .pick_files()
        {
            self.replace_paths(&paths, "open");
        }
    }

    pub(super) fn show_status_overlay(&self, ui: &mut egui::Ui, viewport_rect: egui::Rect) {
        let pointer_over_viewport = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pointer| viewport_rect.contains(pointer));
        if self.status_message.is_none() && self.active_load.is_none() && !pointer_over_viewport {
            return;
        }
        let rect = status_overlay_rect(viewport_rect);
        let ink = ui_theme::viewport_ink(self.settings.viewport_background.is_dark());
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_width(rect.width());
            ui.horizontal(|ui| {
                // A scene load is invisible otherwise: the row gains a small
                // spinner for its duration, alongside any transient status.
                if self.active_load.is_some() {
                    ui.add(egui::Spinner::new().size(13.0));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.locale.tr("loading-scene"))
                                .color(ink)
                                .size(11.5),
                        )
                        .truncate(),
                    );
                }
                if let Some(message) = &self.status_message {
                    let response = ui.add(
                        egui::Label::new(egui::RichText::new(message).color(ink).size(11.5))
                            .truncate(),
                    );
                    response.on_hover_text(message);
                } else if pointer_over_viewport {
                    render_contextual_hint(
                        ui,
                        rect,
                        self.interaction_hint_context(),
                        ink,
                        &self.locale,
                    );
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
        // Canonical wording pinned by source guards; rendering resolves the
        // `guard-close-*` catalog keys below.
        let headline = if edited_count == 1 {
            self.locale.tr("guard-close-headline-one")
        } else {
            self.locale.tr("guard-close-headline-many")
        };
        let note = (edited_count > 1).then(|| {
            self.locale
                .tr_with("guard-close-note", &[("count", &edited_count.to_string())])
        });
        let response = show_guard_dialog(
            ctx,
            &self.locale,
            GuardDialogSpec {
                id: "unsaved-mesh-edits-guard",
                title: &self.locale.tr("guard-close-title"),
                headline: &headline,
                note: note.as_deref(),
                detail: &self.locale.tr("guard-close-detail"),
                destructive_label: &self.locale.tr("guard-close-destructive"),
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
            self.locale.tr_with(
                "guard-replace-headline-session",
                &[("layer", layer.as_str())],
            )
        } else if edited_count <= 1 {
            self.locale.tr("guard-replace-headline-one")
        } else {
            self.locale.tr_with(
                "guard-replace-headline-many",
                &[("count", &edited_count.to_string())],
            )
        };
        let response = show_guard_dialog(
            ctx,
            &self.locale,
            GuardDialogSpec {
                id: "edit-in-progress-guard",
                title: &self.locale.tr("guard-replace-title"),
                headline: &headline,
                note: None,
                detail: &self.locale.tr("guard-replace-detail"),
                destructive_label: &self.locale.tr("guard-replace-destructive"),
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
                    crate::icons::paint(
                        ui.painter(),
                        icon_rect,
                        AppIcon::Error,
                        ui_theme::danger(),
                    );
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
                    if ui.button(self.locale.tr("error-close")).clicked() {
                        close_clicked = true;
                    }
                    if ui.button(self.locale.tr("error-copy-details")).clicked() {
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
            let locale = crate::i18n::LocaleManager::for_tests();
            let _ = show_guard_dialog(
                ui.ctx(),
                &locale,
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
