use super::app_guard_dialog::{show_guard_dialog, GuardDialogAction, GuardDialogSpec};
use super::app_help::render_contextual_hint;
use super::app_recent_popup::RecentFilesAction;
use super::information_dialog::InformationDialog;
use super::settings::panel::{settings_popup_id, show_settings_toolbar_toggle};
use super::state_tool::ViewportTool;
use super::{load_app_logo_color_image, status_overlay_rect, PathBuf, OPEN_DIALOG_EXTENSIONS};
use super::{AppErrorAction, SceneContext};
use crate::measure::measure_overlay::{toolbar_toggle, toolbar_toggle_width, ToolbarToggle};
use crate::measure::measure_tool::{self, MeasureMode};
use crate::ui::icons::AppIcon;
use crate::ui::interaction_hints::ContextualHint;
use crate::ui::ui_theme;
use eframe::egui;
use occluview_core::CameraAxisView;

pub(super) use super::app_recent_popup::recent_files_popup_id;
pub(super) use super::app_recent_popup::show_recent_files_popup;

const AXIS_VIEW_POPUP_ID: &str = "toolbar-axis-view-popup-v1";

pub(super) fn axis_view_popup_id() -> egui::Id {
    egui::Id::new(AXIS_VIEW_POPUP_ID)
}

pub(super) fn axis_view_trigger_id() -> egui::Id {
    egui::Id::new("toolbar-axis-view-trigger-v1")
}

fn toolbar_axis_view_trigger(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    expanded: bool,
    compact: bool,
    tooltip: &str,
) -> egui::Response {
    let ink = if !enabled {
        ui.visuals().weak_text_color()
    } else if expanded {
        ui_theme::ThemeTokens::from_ui(ui).accent()
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };
    let font = egui::FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, ink);
    let icon_side = 15.0;
    let size = egui::vec2(
        if compact {
            30.0
        } else {
            toolbar_toggle_width(ui, label, expanded)
        },
        if expanded { 26.0 } else { 22.0 },
    );
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(
        rect,
        axis_view_trigger_id(),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if expanded {
        ui.painter().rect_filled(
            rect,
            4.0,
            ui_theme::ThemeTokens::from_ui(ui).accent().gamma_multiply(0.16),
        );
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(
                1.0_f32,
                ui_theme::ThemeTokens::from_ui(ui).accent().gamma_multiply(0.75),
            ),
            egui::StrokeKind::Middle,
        );
    } else if enabled && response.hovered() {
        ui.painter().rect_filled(
            rect,
            4.0,
            ui_theme::ThemeTokens::from_ui(ui).accent().gamma_multiply(0.10),
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 7.0 + icon_side * 0.5, rect.center().y),
        egui::Vec2::splat(icon_side),
    );
    crate::ui::icons::paint(ui.painter(), icon_rect, AppIcon::FitView, ink);
    if !compact {
        ui.painter().galley(
            egui::pos2(
                icon_rect.right() + 5.0,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink,
        );
    }
    if expanded && !compact {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::vec2(10.0, 10.0),
        );
        crate::ui::icons::paint(ui.painter(), close_rect, AppIcon::Close, ink);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
    });
    response
        .ctx
        .accesskit_node_builder(response.id, |node| {
            node.set_has_popup(egui::accesskit::HasPopup::Menu);
            node.set_expanded(expanded);
        });
    response
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

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
                crate::i18n::message_id!("toolbar-view-label"),
                egui::Popup::is_id_open(ui.ctx(), axis_view_popup_id()),
            ),
            (
                crate::i18n::message_id!("toolbar-settings-label"),
                egui::Popup::is_id_open(ui.ctx(), settings_popup_id()),
            ),
        ];
        let controls_width: f32 = controls
            .iter()
            .map(|(key, active)| toolbar_toggle_width(ui, &self.ui.locale.tr(*key), *active))
            .sum();
        // Recent-files chevron, group dividers, and spacing between controls.
        let chrome_width = 18.0 + 4.0 + 3.0 * 13.0 + 14.0 * ui.spacing().item_spacing.x;
        let scenes_width = toolbar_toggle_width(
            ui,
            &self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-scenes")),
            false,
        );
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
        let can_switch_tools = !self.document.edit_mode.is_busy() && !self.tools.sculpt.is_busy();
        let can_cut = self.can_render_cut_view() && can_switch_tools;
        let has_pickable_layer = self.has_measurable_layer();
        let can_measure = measure_tool::measure_menu_enabled(has_pickable_layer, !can_switch_tools);
        let can_edit_mesh = has_pickable_layer;
        let mut toggle_cut_view = false;
        let mut toggle_measure: Option<MeasureMode> = None;
        let mut toggle_align = false;
        let mut toggle_edit_mesh = false;
        let mut snap_axis: Option<CameraAxisView> = None;
        let camera_view_enabled = self.camera_axis_menu_enabled(&ctx);
        if !camera_view_enabled {
            egui::Popup::close_id(&ctx, axis_view_popup_id());
        }
        if !self.ui.modal_dialog_open() && !ctx.egui_wants_keyboard_input() {
            let consume = |ctx: &egui::Context, shortcut: &egui::KeyboardShortcut| {
                ctx.input_mut(|input| input.consume_key(shortcut.modifiers, shortcut.logical_key))
            };
            if consume(&ctx, &cut_shortcut) && can_cut {
                toggle_cut_view = true;
            }
            if consume(&ctx, &ruler_shortcut) && can_measure {
                toggle_measure = Some(MeasureMode::Ruler);
            }
            if consume(&ctx, &thickness_shortcut) && can_measure {
                toggle_measure = Some(MeasureMode::Thickness);
            }
            if consume(&ctx, &align_shortcut) && can_measure {
                toggle_align = true;
            }
            if consume(&ctx, &edit_shortcut) && can_edit_mesh {
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
                    .fill(ui_theme::ThemeTokens::from_ui(root_ui).toolbar_fill())
                    .stroke(egui::Stroke::new(1.0_f32, ui_theme::ThemeTokens::from_ui(root_ui).hairline()))
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
                    let can_show_recent = !self.persistence.recent_files.is_empty()
                        || !self.persistence.may_write_recent;
                    ui.add_enabled_ui(can_show_recent, |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::click());
                        crate::ui::icons::paint(
                            ui.painter(),
                            rect,
                            AppIcon::ChevronDown,
                            if response.hovered() {
                                ui_theme::ThemeTokens::from_ui(ui).text()
                            } else {
                                ui_theme::ThemeTokens::from_ui(ui).text_weak()
                            },
                        );
                        crate::ui::accessibility::button(
                            &response,
                            &self
                                .ui
                                .locale
                                .tr(crate::i18n::message_id!("toolbar-recent-hint")),
                            can_show_recent,
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

                    self.show_scenes_menu(ui, compact);
                    toolbar_divider(ui);

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
                        let tooltip = if !can_switch_tools {
                            self.ui
                                .locale
                                .tr(crate::i18n::message_id!("repair-edit-busy"))
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
                        let settings_response = show_settings_toolbar_toggle(
                            ui,
                            !self.ui.close_guard_open,
                            &self.ui.locale,
                            compact,
                        );
                        if settings_response.clicked() {
                            self.ui.information_dialog = InformationDialog::None;
                        }
                        self.show_settings_popup(&settings_response);

                        let view_label = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("toolbar-view-label"));
                        let view_hint = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("toolbar-view-hint"));
                        let view_was_open =
                            egui::Popup::is_id_open(ui.ctx(), axis_view_popup_id());
                        let view_control_enabled = camera_view_enabled && ui.is_enabled();
                        if !view_control_enabled {
                            egui::Popup::close_id(ui.ctx(), axis_view_popup_id());
                        }
                        let response = toolbar_axis_view_trigger(
                            ui,
                            &view_label,
                            view_control_enabled,
                            view_was_open,
                            compact,
                            &view_hint,
                        );
                        egui::Popup::menu(&response)
                            .id(axis_view_popup_id())
                            .align(egui::RectAlign::BOTTOM_END)
                            .align_alternatives(&[])
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|menu_ui| {
                                menu_ui.set_min_width(160.0);
                                for axis in CameraAxisView::ALL {
                                    let item_label = self.ui.locale.tr_with(
                                        crate::i18n::message_id!("toolbar-view-from-axis"),
                                        &[("axis", axis.label())],
                                    );
                                    if menu_ui
                                        .add_sized(
                                            [
                                                menu_ui.available_width(),
                                                menu_ui.spacing().interact_size.y,
                                            ],
                                            egui::Button::new(item_label),
                                        )
                                        .clicked()
                                    {
                                        snap_axis = Some(axis);
                                    }
                                }
                            });
                        if snap_axis.is_some() {
                            egui::Popup::close_id(&ctx, axis_view_popup_id());
                        }
                        response
                            .ctx
                            .accesskit_node_builder(response.id, |node| {
                                node.set_expanded(egui::Popup::is_id_open(
                                    &ctx,
                                    axis_view_popup_id(),
                                ));
                            });

                        // Help remains in Settings, on F1, and behind the
                        // viewport's own hint line.
                    });
                });
            });

        if let Some(axis) = snap_axis {
            // This is the pre-show gate: Popup::show closes the owner's popup
            // when an item is activated, so a live post-close query would
            // reject the command that popup just produced.
            if camera_view_enabled {
                if let Some(camera) = self.render.camera.as_mut() {
                    camera.snap_to_axis(axis);
                    self.request_camera_repaint(&ctx);
                }
            }
        }

        if toggle_edit_mesh || toggle_align || toggle_cut_view || toggle_measure.is_some() {
            *self.preserve_on_transfer_undo = true;
        }

        // The Edit button and its E hotkey share one entry path. Opening while
        // the editor is already open is the toggle's business, not this
        // button's: a second session over a live one would discard the first
        // one's selection.
        if toggle_edit_mesh {
            let edit_active = self.document.edit_mode.has_active_session();
            let first_layer = self.document.scene.as_ref().and_then(|scene| {
                scene
                    .meshes()
                    .iter()
                    .find(|entry| !entry.mesh.is_point_cloud() && entry.visible)
                    .map(occluview_core::SceneMesh::id)
            });
            if let (false, true, Some(layer_id)) = (edit_active, can_edit_mesh, first_layer) {
                if !self.begin_mesh_edit_session(layer_id)
                    && matches!(
                        self.document.edit_mode.take_session_start_failure(),
                        Some(crate::edit_mode::EditSessionStartFailure::HistoryCapacityUnavailable)
                    )
                {
                    self.scene_ui.status_message = Some(
                        self.ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-history-budget")),
                    );
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
            } else if self.prepare_viewport_tool_entry(ViewportTool::Cut, &ctx) {
                self.tools.cut_view.enable();
            }
            self.render.invalidation.overlay_tools_changed();
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
                Some(mode) => {
                    if self.prepare_viewport_tool_entry(ViewportTool::Measure, &ctx) {
                        self.tools.measure.arm(mode);
                    }
                }
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
            self.persistence.clear_recent_files();
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
                super::mesh_edit::export::SaveEditedLayersOutcome::AllSaved
                | super::mesh_edit::export::SaveEditedLayersOutcome::NothingToSave => {
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
                super::mesh_edit::export::SaveEditedLayersOutcome::Aborted => {}
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
        let mut close_clicked = false;
        let mut retry = false;
        let close_label = self.ui.locale.tr(crate::i18n::message_id!("error-close"));
        let modal = crate::ui::modal_surface::show_information_modal(
            ctx,
            egui::Id::new("occluview-error-dialog"),
            egui::vec2(540.0, 340.0),
            &close_label,
            |ui| {
                ui_theme::dialog_header(ui, &error.title, AppIcon::Error);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&error.summary)
                            .strong()
                            .size(13.5)
                            .color(ui_theme::ThemeTokens::from_context(ctx).text()),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("error-details")
                    .max_height((ctx.content_rect().height() - 240.0).clamp(48.0, 240.0))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&error.details)
                                    .size(12.0)
                                    .color(ui_theme::ThemeTokens::from_context(ctx).text_weak()),
                            )
                            .wrap()
                            .selectable(false),
                        );
                    });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                let mut retry_clicked = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(ui_theme::primary_button(
                            ui_theme::ThemeTokens::from_context(ctx),
                            &close_label,
                        ))
                        .clicked()
                    {
                        close_clicked = true;
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
            },
        );
        if modal.should_close() || close_clicked || retry {
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
    use anyhow::Context as _;

    fn axis_view_app(ctx: &egui::Context) -> crate::app::OccluViewApp {
        let mut app = crate::app::OccluViewApp::new_for_tests(ctx.clone());
        let model = super::super::app_test_support::named_scene("surface", 0.0);
        app.workspace.scenes[0].render.camera = Some(super::super::home_camera_for_scene(&model));
        app.workspace.scenes[0].document.scene = Some(model.into());
        app
    }

    fn toolbar_frame(
        app: &mut crate::app::OccluViewApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        app.ui.popup_open_at_frame_start = egui::Popup::is_any_open(ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                app.active_context()
                    .expect("test scene")
                    .show_toolbar(ui);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn accesskit_node<'a>(
        output: &'a egui::FullOutput,
        label: &str,
    ) -> (egui::accesskit::NodeId, &'a egui::accesskit::Node) {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .and_then(|update| {
                update
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .map(|(id, node)| (*id, node))
            })
            .unwrap_or_else(|| panic!("missing AccessKit node {label:?}"))
    }

    #[allow(clippy::cast_possible_truncation)]
    fn accessible_center(bounds: egui::accesskit::Rect) -> egui::Pos2 {
        egui::pos2(
            bounds.x0.midpoint(bounds.x1) as f32,
            bounds.y0.midpoint(bounds.y1) as f32,
        )
    }

    fn pointer_button(point: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn axis_view_item_label(app: &crate::app::OccluViewApp, axis: CameraAxisView) -> String {
        app.ui.locale.tr_with(
            crate::i18n::message_id!("toolbar-view-from-axis"),
            &[("axis", axis.label())],
        )
    }

    #[test]
    fn axis_view_menu_click_snaps_during_its_active_ui_capture() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = axis_view_app(&ctx);
        let trigger_label = app
            .ui
            .locale
            .tr(crate::i18n::message_id!("toolbar-view-label"));
        let initial = toolbar_frame(&mut app, &ctx, Vec::new());
        let (_, trigger) = accesskit_node(&initial, &trigger_label);
        let trigger_pos = accessible_center(trigger.bounds().expect("View button bounds"));

        toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(trigger_pos),
                pointer_button(trigger_pos, true),
            ],
        );
        let active = app.workspace.input.active();
        assert!(matches!(
            app.workspace.input.primary_pressed(
                active,
                super::super::workspace::input::GestureKind::UiControl,
                None,
            ),
            super::super::workspace::input::PressResult::Begin(_)
        ));
        let menu_opened = toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(trigger_pos),
                pointer_button(trigger_pos, false),
            ],
        );
        assert!(egui::Popup::is_id_open(&ctx, axis_view_popup_id()));
        let _ = app.workspace.input.primary_released();

        let axis = CameraAxisView::PositiveX;
        let item_label = axis_view_item_label(&app, axis);
        let (_, item) = accesskit_node(&menu_opened, &item_label);
        let item_pos = accessible_center(item.bounds().expect("axis button bounds"));
        toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(item_pos),
                pointer_button(item_pos, true),
            ],
        );
        assert!(matches!(
            app.workspace.input.primary_pressed(
                active,
                super::super::workspace::input::GestureKind::UiControl,
                None,
            ),
            super::super::workspace::input::PressResult::Begin(_)
        ));
        toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(item_pos),
                pointer_button(item_pos, false),
            ],
        );
        let _ = app.workspace.input.primary_released();

        let camera = app.workspace.scenes[0]
            .render
            .camera
            .as_ref()
            .expect("axis menu retains the camera");
        assert!(
            camera.view_direction().dot(glam::Vec3::NEG_X) > 0.9999,
            "a menu click captured by the active pane must snap to the +X view"
        );
    }

    #[test]
    fn axis_view_menu_supports_enter_and_space_and_rejects_divider_capture() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = axis_view_app(&ctx);
        toolbar_frame(&mut app, &ctx, Vec::new());
        ctx.memory_mut(|memory| memory.request_focus(axis_view_trigger_id()));
        let opened = toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(egui::Popup::is_id_open(&ctx, axis_view_popup_id()));
        let item_label = axis_view_item_label(&app, CameraAxisView::PositiveY);
        let (node_id, _) = accesskit_node(&opened, &item_label);
        toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Focus,
                    target_node: node_id,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    data: None,
                },
            )],
        );
        assert!(
            app.ui.modal_dialog_open(),
            "the open camera menu remains in the global Escape/modal snapshot"
        );
        assert!(
            app.active_context()
                .expect("test scene")
                .camera_axis_menu_enabled(&ctx),
            "only the menu's own camera command route is exempt"
        );
        toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let camera = app.workspace.scenes[0]
            .render
            .camera
            .as_ref()
            .expect("axis menu retains the camera");
        assert!(
            camera.view_direction().dot(glam::Vec3::NEG_Y) > 0.9999,
            "Space on the focused axis button must snap to the +Y view"
        );

        ctx.memory_mut(|memory| memory.request_focus(axis_view_trigger_id()));
        let opened_again = toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let item_label = axis_view_item_label(&app, CameraAxisView::NegativeX);
        let (node_id, _) = accesskit_node(&opened_again, &item_label);
        toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_node: node_id,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    data: None,
                },
            )],
        );
        let camera = app.workspace.scenes[0]
            .render
            .camera
            .as_ref()
            .expect("axis menu retains the camera");
        assert!(
            camera.view_direction().dot(glam::Vec3::X) > 0.9999,
            "an AccessKit click must activate the selected axis button"
        );

        let active = app.workspace.input.active();
        let _ = app
            .workspace
            .input
            .begin_divider_resize(active, 0.5);
        egui::Popup::open_id(&ctx, axis_view_popup_id());
        let blocked = toolbar_frame(&mut app, &ctx, Vec::new());
        assert!(!egui::Popup::is_id_open(&ctx, axis_view_popup_id()));
        let (_, trigger) = accesskit_node(
            &blocked,
            &app.ui
                .locale
                .tr(crate::i18n::message_id!("toolbar-view-label")),
        );
        assert!(trigger.is_disabled(), "divider capture disables View");

        let _ = app.workspace.input.primary_released();
        app.ui.close_guard_open = true;
        app.workspace.input =
            super::super::workspace::input::InputArbiter::new(active);
        egui::Popup::open_id(&ctx, axis_view_popup_id());
        let modal = toolbar_frame(&mut app, &ctx, Vec::new());
        assert!(!egui::Popup::is_id_open(&ctx, axis_view_popup_id()));
        let (_, trigger) = accesskit_node(
            &modal,
            &app.ui
                .locale
                .tr(crate::i18n::message_id!("toolbar-view-label")),
        );
        assert!(trigger.is_disabled(), "a modal dialog disables View");
    }

    #[test]
    fn disabled_toolbar_commands_cannot_be_armed_by_keyboard_shortcuts() -> anyhow::Result<()> {
        for key in [
            egui::Key::C,
            egui::Key::M,
            egui::Key::T,
            egui::Key::A,
            egui::Key::E,
        ] {
            let ctx = egui::Context::default();
            let mut app = crate::app::OccluViewApp::new_for_tests(ctx.clone());
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    let Some(mut scene) = app.active_context() else {
                        panic!("scene missing");
                    };
                    scene.show_toolbar(ui);
                },
            )
            .drop_without_applying_deltas();
            let scene = app.active_context().context("scene missing")?;
            assert!(
                !scene.tools.cut_view.is_active(),
                "{key:?} must obey the disabled Cut button"
            );
            assert!(
                !scene.tools.measure.is_active(),
                "{key:?} must obey the disabled measurement buttons"
            );
            assert!(
                !scene.align_active(),
                "{key:?} must obey the disabled Align button"
            );
            assert!(!scene.document.edit_mode.has_active_session());
        }
        Ok(())
    }

    #[test]
    fn toolbar_shortcuts_finish_idle_editing_except_for_cut_view() -> anyhow::Result<()> {
        for (key, busy) in [egui::Key::M, egui::Key::T, egui::Key::A, egui::Key::C]
            .into_iter()
            .flat_map(|key| [(key, false), (key, true)])
        {
            let ctx = egui::Context::default();
            let mut app = crate::app::OccluViewApp::new_for_tests(ctx.clone());
            let model =
                std::sync::Arc::new(super::super::app_test_support::named_scene("surface", 0.0));
            app.workspace.scenes[0].document.scene = Some(model.clone());
            assert!(app.workspace.scenes[0]
                .document
                .edit_mode
                .begin_face_selection(&model.meshes()[0], &model));
            if busy {
                assert!(app.workspace.scenes[0]
                    .document
                    .edit_mode
                    .begin_scene_edit(
                        &model,
                        model.meshes()[0].id(),
                        crate::edit_mode::EditModeCommand::MoveLayer,
                    )
                    .is_some());
            }
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    let Some(mut scene) = app.active_context() else {
                        panic!("scene missing");
                    };
                    scene.show_toolbar(ui);
                },
            )
            .drop_without_applying_deltas();
            let scene = app.active_context().context("scene missing")?;
            assert_eq!(
                scene.tools.measure.is_active(),
                !busy && matches!(key, egui::Key::M | egui::Key::T)
            );
            assert_eq!(scene.align_active(), !busy && key == egui::Key::A);
            // Cut View is refused while any editing session is open, and the
            // refusal must leave that session in place rather than apply it.
            assert!(!scene.tools.cut_view.is_active());
            assert_eq!(
                scene.document.edit_mode.has_active_session(),
                busy || key == egui::Key::C
            );
            assert_eq!(scene.document.edit_mode.is_busy(), busy);
        }
        Ok(())
    }

    #[test]
    fn hidden_meshes_do_not_enable_the_edit_toolbar_button() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = crate::app::OccluViewApp::new_for_tests(ctx.clone());
        let mut model = super::super::app_test_support::named_scene("surface", 0.0);
        model.meshes_mut()[0].visible = false;
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(model));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let Some(mut scene) = app.active_context() else {
                panic!("scene missing");
            };
            scene.show_toolbar(ui);
        });
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .context("toolbar output")?;
        let edit = update
            .nodes
            .iter()
            .find_map(|(_, node)| (node.label() == Some("Edit")).then_some(node))
            .context("Edit button missing")?;
        assert!(edit.is_disabled(), "Edit has no visible mesh it can open");
        Ok(())
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
