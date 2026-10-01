use super::app_guard_dialog::{show_guard_dialog, GuardDialogAction, GuardDialogSpec};
use super::app_help::render_contextual_hint;
use super::app_recent_popup::RecentFilesAction;
use super::app_settings_panel::{settings_popup_id, show_settings_toolbar_toggle};
use super::information_dialog::InformationDialog;
use super::{load_app_logo_color_image, status_overlay_rect, PathBuf, OPEN_DIALOG_EXTENSIONS};
use super::{AppErrorAction, SceneContext};
use crate::measure_overlay::{toolbar_toggle, ToolbarToggle};
use crate::measure_tool::{self, MeasureMode};
use crate::ui::icons::AppIcon;
use crate::ui::interaction_hints::ContextualHint;
use crate::ui::ui_theme;
use eframe::egui;

pub(super) use super::app_recent_popup::recent_files_popup_id;
pub(super) use super::app_recent_popup::show_recent_files_popup;

impl SceneContext<'_> {
    fn toolbar_needs_compact(&self, ui: &egui::Ui) -> bool {
        let controls = [
            (crate::i18n::message_id!("toolbar-open-label"), false),
            (crate::i18n::message_id!("toolbar-add-label"), false),
            (
                crate::i18n::message_id!("toolbar-cut-label"),
                self.tools.cut_view.is_active(),
            ),
            (
                crate::i18n::message_id!("toolbar-ruler-label"),
                self.tools.measure.mode() == Some(MeasureMode::Ruler),
            ),
            (
                crate::i18n::message_id!("toolbar-thickness-label"),
                self.tools.measure.mode() == Some(MeasureMode::Thickness),
            ),
            (
                crate::i18n::message_id!("toolbar-align-label"),
                self.align_active(),
            ),
            (
                crate::i18n::message_id!("toolbar-edit-label"),
                self.document.edit_mode.has_active_session(),
            ),
            (
                crate::i18n::message_id!("toolbar-settings-label"),
                egui::Popup::is_id_open(ui.ctx(), settings_popup_id()),
            ),
        ];
        let controls_width: f32 = controls
            .iter()
            .map(|(key, active)| {
                crate::measure_overlay::toolbar_toggle_width(ui, &self.ui.locale.tr(*key), *active)
            })
            .sum();
        let scenes = ui.painter().layout_no_wrap(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("workspace-scenes")),
            egui::TextStyle::Button.resolve(ui.style()),
            ui_theme::text(),
        );
        // Recent-files chevron, group dividers, and spacing between controls.
        let chrome_width = 18.0 + 4.0 + 3.0 * 13.0 + 14.0 * ui.spacing().item_spacing.x;
        let scenes_width = scenes.size().x + 2.0 * ui.spacing().button_padding.x;
        controls_width + scenes_width + chrome_width > ui.available_width()
    }

    /// Draw the top toolbar and dispatch its actions after layout.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_toolbar(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        if self.ui.close_guard_open
            || self.ui.pending_replace_open.is_some()
            || self.ui.app_error.is_some()
            || self.ui.information_dialog.is_open()
        {
            egui::Popup::close_id(&ctx, settings_popup_id());
        }
        // The only wired file shortcut, which the Open tooltip names.
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
        if !self.ui.modal_dialog_open() && !ctx.egui_wants_keyboard_input() {
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
        // F1 opens the keyboard and mouse reference, the help key desktop apps
        // share; the toolbar has no Help button.
        let mut open_shortcuts = false;
        if !self.ui.modal_dialog_open() && !ctx.egui_wants_keyboard_input() {
            open_shortcuts =
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F1));
        }
        let mut do_add = false;
        // Ctrl+O obeys the same gate as every other shortcut above. Outside it,
        // the native dialog would open on top of an information modal and, with
        // unsaved edits, park an open behind a guard window that the modal
        // layer leaves dimmed and unclickable until the modal is closed.
        let mut do_open = !self.ui.modal_dialog_open()
            && !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_shortcut(&open_shortcut));
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
                if self.ui.command_dialog_open() {
                    ui.disable();
                }
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let compact = self.toolbar_needs_compact(ui);

                    // Shortcut glyphs interpolate as catalog variables and stay invariant.
                    let open_shortcut_text = ui.ctx().format_shortcut(&open_shortcut);
                    let open_hint = self.ui.locale.tr_with(
                        crate::i18n::message_id!("toolbar-open-hint"),
                        &[("shortcut", &open_shortcut_text)],
                    );
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Open,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-open-label")),
                            true,
                            false,
                            &open_hint,
                        )
                        .compact(compact),
                    )
                    .clicked()
                    {
                        do_open = true;
                    }
                    // Recent files use a chevron popup attached to Open.
                    ui.add_enabled_ui(!self.persistence.recent_files.is_empty(), |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::click());
                        crate::ui::icons::paint(
                            ui.painter(),
                            rect,
                            AppIcon::ChevronDown,
                            if response.hovered() {
                                ui_theme::text()
                            } else {
                                ui_theme::text_weak()
                            },
                        );
                        crate::ui::accessibility::button(
                            &response,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-recent-hint")),
                            !self.persistence.recent_files.is_empty(),
                            Some(egui::Popup::is_id_open(ui.ctx(), recent_files_popup_id())),
                        );
                        let response = response.on_hover_text(
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-recent-hint")),
                        );
                        if let Some(action) = show_recent_files_popup(
                            &response,
                            &self.persistence.recent_files,
                            &self.ui.locale,
                        ) {
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
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-add-label")),
                            true,
                            false,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-add-hint")),
                        )
                        .compact(compact),
                    )
                    .clicked()
                    {
                        do_add = true;
                    }

                    toolbar_divider(ui);

                    self.show_scenes_menu(ui);
                    toolbar_divider(ui);

                    let can_cut = self.can_render_cut_view();
                    let cut_shortcut_text = ui.ctx().format_shortcut(&cut_shortcut);
                    let cut_hint = if can_cut {
                        self.ui.locale.tr_with(
                            crate::i18n::message_id!("toolbar-cut-hint"),
                            &[("shortcut", &cut_shortcut_text)],
                        )
                    } else {
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("toolbar-cut-unavailable"))
                    };
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Cut,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-cut-label")),
                            can_cut,
                            self.tools.cut_view.is_active(),
                            &cut_hint,
                        )
                        .compact(compact),
                    )
                    .clicked()
                    {
                        toggle_cut_view = true;
                    }

                    toolbar_divider(ui);

                    let edit_session_active = self.document.edit_mode.has_active_session();
                    let has_pickable_layer = self.has_measurable_layer();
                    let can_measure =
                        measure_tool::measure_menu_enabled(has_pickable_layer, edit_session_active);
                    let entries = [
                        (
                            AppIcon::Ruler,
                            MeasureMode::Ruler,
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-ruler-label")),
                            crate::i18n::message_id!("toolbar-ruler-hint"),
                            ruler_shortcut,
                        ),
                        (
                            AppIcon::Thickness,
                            MeasureMode::Thickness,
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-thickness-label")),
                            crate::i18n::message_id!("toolbar-thickness-hint"),
                            thickness_shortcut,
                        ),
                    ];
                    for (icon, mode, label, hint_key, shortcut) in entries {
                        let shortcut_text = ui.ctx().format_shortcut(&shortcut);
                        let tooltip = if edit_session_active {
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-measure-blocked"))
                        } else if !has_pickable_layer {
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-measure-needs-layer"))
                        } else {
                            self.ui
                                .locale
                                .tr_with(hint_key, &[("shortcut", &shortcut_text)])
                        };
                        let active = self.tools.measure.mode() == Some(mode);
                        if toolbar_toggle(
                            ui,
                            ToolbarToggle::new(icon, &label, can_measure, active, &tooltip)
                                .compact(compact),
                        )
                        .clicked()
                        {
                            toggle_measure = Some(mode);
                        }
                    }

                    let align_shortcut_text = ui.ctx().format_shortcut(&align_shortcut);
                    let align_hint = self.ui.locale.tr_with(
                        crate::i18n::message_id!("toolbar-align-hint"),
                        &[("shortcut", &align_shortcut_text)],
                    );
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::Align,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-align-label")),
                            can_measure,
                            self.align_active(),
                            &align_hint,
                        )
                        .compact(compact),
                    )
                    .clicked()
                    {
                        toggle_align = true;
                    }

                    let can_edit_mesh =
                        self.document.scene.is_some()
                            && self.document.scene.as_ref().is_some_and(|s| {
                                s.meshes().iter().any(|m| !m.mesh.is_point_cloud())
                            });
                    let edit_active = self.document.edit_mode.has_active_session();
                    let edit_shortcut_text = ui.ctx().format_shortcut(&edit_shortcut);
                    let edit_hint = if edit_active {
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("toolbar-edit-open"))
                    } else {
                        self.ui.locale.tr_with(
                            crate::i18n::message_id!("toolbar-edit-hint"),
                            &[("shortcut", &edit_shortcut_text)],
                        )
                    };
                    if toolbar_toggle(
                        ui,
                        ToolbarToggle::new(
                            AppIcon::EditMesh,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-edit-label")),
                            can_edit_mesh,
                            edit_active,
                            &edit_hint,
                        )
                        .compact(compact),
                    )
                    .clicked()
                    {
                        toggle_edit_mesh = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // No Help button here: the keyboard and mouse reference
                        // lives in Settings, on F1, and behind the viewport's
                        // own hint line, so the toolbar width goes to the tools.
                        let response = show_settings_toolbar_toggle(
                            ui,
                            !self.ui.close_guard_open,
                            &self.ui.locale,
                            compact,
                        );
                        if response.clicked() {
                            self.ui.information_dialog = InformationDialog::None;
                        }
                        self.show_settings_popup(&response);
                    });
                });
            });

        if toggle_edit_mesh || toggle_align || toggle_cut_view || toggle_measure.is_some() {
            *self.preserve_on_transfer_undo = true;
        }

        // The Edit button and its E hotkey share one entry path. Opening while
        // the editor is already open is the toggle's business, not this
        // button's: a second session over a live one would discard the first
        // one's selection.
        if toggle_edit_mesh {
            let edit_active = self.document.edit_mode.has_active_session();
            let can_edit_mesh = self.document.scene.is_some()
                && self
                    .document
                    .scene
                    .as_ref()
                    .is_some_and(|s| s.meshes().iter().any(|m| !m.mesh.is_point_cloud()));
            if let (false, true, Some(scene)) =
                (edit_active, can_edit_mesh, self.document.scene.clone())
            {
                for entry in scene.meshes() {
                    if !entry.mesh.is_point_cloud() && entry.visible {
                        self.document.capture_edit_metadata();
                        if !self.document.edit_mode.begin_face_selection(entry, &scene) {
                            self.document.discard_edit_metadata();
                            if matches!(self.document.edit_mode.take_session_start_failure(),
                                Some(crate::edit_mode::EditSessionStartFailure::HistoryCapacityUnavailable)) {
                                self.scene_ui.status_message = Some(self.ui.locale.tr(
                                    crate::i18n::message_id!("workspace-history-budget"),
                                ));
                            }
                        }
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
            if self.tools.cut_view.is_active() {
                self.tools.cut_view.disable();
            } else {
                // The viewport-owning tools are mutually exclusive: entering
                // the cut view stands the measurement tool down cleanly.
                self.tools.measure.disarm();
                self.tools.cut_view.enable();
            }
            self.render.invalidation.overlay_tools_changed();
        }
        // Arming a measurement or the cut view closes Align, the same way
        // arming Align closes them. Two tools cannot share the primary click.
        if (toggle_measure.is_some() || toggle_cut_view) && self.align_active() {
            self.cancel_align_session(&ctx);
        }
        if let Some(clicked) = toggle_measure {
            let (next, disable_cut) = measure_tool::apply_menu_toggle(
                self.tools.measure.mode(),
                self.tools.cut_view.is_active(),
                clicked,
            );
            if disable_cut {
                self.tools.cut_view.disable();
                self.render.invalidation.overlay_tools_changed();
            }
            match next {
                Some(mode) => self.tools.measure.arm(mode),
                None => self.tools.measure.disarm(),
            }
            ctx.request_repaint();
        }

        if open_shortcuts {
            self.ui.information_dialog = InformationDialog::KeyboardMouse;
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
            self.persistence.recent_files.clear();
            self.persistence.save_recent_files();
        }
        if let Some(paths) = recent_to_open {
            self.replace_paths(&paths, "recent");
        }
    }

    pub(super) fn app_logo_texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.ui.app_logo.is_none() {
            if let Some(color_image) = load_app_logo_color_image() {
                self.ui.app_logo = Some(ctx.load_texture(
                    "occluview-app-logo",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        self.ui.app_logo.as_ref()
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
        if self.scene_ui.status_message.is_none()
            && !self.loader.has_work_for(self.scene_key)
            && !pointer_over_viewport
        {
            return;
        }
        let rect = status_overlay_rect(viewport_rect);
        let ink = ui_theme::viewport_ink(self.persistence.settings.viewport_background.is_dark());
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_width(rect.width());
            ui.horizontal(|ui| {
                // A scene load is invisible otherwise: the row gains a small
                // spinner for its duration, alongside any transient status.
                if self.loader.has_work_for(self.scene_key) {
                    ui.add(egui::Spinner::new().size(13.0));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                self.ui.locale.tr(crate::i18n::message_id!("loading-scene")),
                            )
                            .color(ink)
                            .size(11.5),
                        )
                        .truncate(),
                    );
                }
                if let Some(message) = &self.scene_ui.status_message {
                    let response = ui.add(
                        egui::Label::new(egui::RichText::new(message).color(ink).size(11.5))
                            .truncate(),
                    );
                    response.on_hover_text(message);
                } else if pointer_over_viewport {
                    render_contextual_hint(
                        ui,
                        rect,
                        ContextualHint {
                            context: self.interaction_hint_context(),
                            scroll_behavior: self.persistence.settings.scroll_behavior,
                        },
                        ink,
                        &self.ui.locale,
                    );
                }
            });
        });
    }

    /// Guard an incoming replace open (parked in `pending_replace_open`) while a
    /// live edit session is dirty or unsaved edits exist. Mirrors the
    /// close-guard wording: "Save…" writes each edited layer then opens,
    pub(super) fn guard_pending_replace_open(&mut self, ctx: &egui::Context) {
        if self.ui.pending_replace_open.is_none() {
            return;
        }
        // Never stack over the close guard; it takes precedence (the app is
        // trying to exit). The parked open waits until that resolves.
        if self.ui.close_guard_open {
            return;
        }
        let session_layer = self.active_session_layer_label();
        let edited_count = self.document.unsaved_edit_layer_ids.len();
        let mut do_save = false;
        let mut do_discard = false;
        let mut do_cancel = false;
        let headline = if let Some(layer) = &session_layer {
            self.ui.locale.tr_with(
                crate::i18n::message_id!("guard-replace-headline-session"),
                &[("layer", layer.as_str())],
            )
        } else if edited_count <= 1 {
            self.ui
                .locale
                .tr(crate::i18n::message_id!("guard-replace-headline-one"))
        } else {
            self.ui.locale.tr_with(
                crate::i18n::message_id!("guard-replace-headline-many"),
                &[("count", &edited_count.to_string())],
            )
        };
        let busy_note =
            (self.document.edit_mode.is_busy() || self.sculpt_has_live_work()).then(|| {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("edit-session-busy"))
            });
        let response = show_guard_dialog(
            ctx,
            &self.ui.locale,
            GuardDialogSpec {
                id: "edit-in-progress-guard",
                title: &self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("guard-replace-title")),
                headline: &headline,
                note: busy_note.as_deref(),
                detail: &self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("guard-replace-detail")),
                destructive_label: &self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("guard-replace-destructive")),
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
            self.ui.pending_replace_open = None;
            return;
        }
        if self.document.edit_mode.is_busy() || self.sculpt_has_live_work() {
            if do_discard || do_save {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("edit-session-busy")),
                );
            }
            return;
        }
        if do_discard {
            if let Some(pending) = self.ui.pending_replace_open.take() {
                self.replace_paths_confirmed(&pending.paths, pending.source, pending.requested_at);
            }
            return;
        }
        if do_save {
            match self.save_edited_layers_flow() {
                super::app_mesh_export::SaveEditedLayersOutcome::AllSaved
                | super::app_mesh_export::SaveEditedLayersOutcome::NothingToSave => {
                    if let Some(pending) = self.ui.pending_replace_open.take() {
                        self.replace_paths_confirmed(
                            &pending.paths,
                            pending.source,
                            pending.requested_at,
                        );
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
        let id = self.document.edit_mode.session_layer_id()?;
        let scene = self.document.scene.as_ref()?;
        let index = scene.meshes().iter().position(|entry| entry.id() == id)?;
        Some(crate::layers_overlay::layer_label(
            &self.document.current_paths,
            &scene.meshes()[index],
            index,
            &self.ui.locale,
        ))
    }

    pub(super) fn show_error_dialog(&mut self, ctx: &egui::Context) {
        let Some(error) = self.ui.app_error.clone() else {
            return;
        };
        let mut open = true;
        let mut close_clicked = false;
        let mut retry = false;
        // The details field scrolls internally, so the dialog needs no resize handles.
        egui::Window::new(error.title.as_str())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_size([460.0, 260.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    crate::ui::icons::paint(
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
                let details_response = ui.add(
                    egui::TextEdit::multiline(&mut details)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
                crate::ui::accessibility::read_only_text(
                    &details_response,
                    &self.ui.locale.tr(crate::i18n::message_id!("error-details")),
                    &details,
                );
                ui.add_space(4.0);
                let mut retry_clicked = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(self.ui.locale.tr(crate::i18n::message_id!("error-close")))
                        .clicked()
                    {
                        close_clicked = true;
                    }
                    if ui
                        .button(
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("error-copy-details")),
                        )
                        .clicked()
                    {
                        ui.ctx().copy_text(error.details.clone());
                    }
                    if error.action == AppErrorAction::RetryGraphics
                        && ui
                            .button(
                                self.ui
                                    .locale
                                    .tr(crate::i18n::message_id!("error-retry-graphics")),
                            )
                            .clicked()
                    {
                        retry_clicked = true;
                    }
                });
                if retry_clicked {
                    retry = true;
                }
            });
        if !open || close_clicked || retry {
            self.ui.app_error = None;
        }
        if retry {
            self.commands
                .push_back(super::workspace::commands::WorkspaceCommand::RetryGraphics);
            ctx.request_repaint();
        }
    }
}

/// Slim vertical hairline between toolbar groups.
fn toolbar_divider(ui: &mut egui::Ui) {
    ui_theme::vertical_divider(ui, 18.0);
}

#[cfg(test)]
mod tests {
    use super::*;

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
