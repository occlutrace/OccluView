//! The settings popover surface: sections, rows, and the action vocabulary
//! the app applies on top of its state. Pure UI — the app-side dispatch lives
//! in `app_settings_window`, the modal About/third-party windows too.

use crate::app_settings::{
    FallbackExportFormat, Settings, ThemePreference, UnitDisplay, ViewportBackground,
};
use crate::icons::AppIcon;
use crate::ui_theme;
use crate::update_notice::UpdateCheckStatus;
use eframe::egui;

pub(super) const PANEL_MARGIN: i8 = 12;
pub(super) const ROW_HEIGHT: f32 = 30.0;
pub(super) const SETTINGS_PANEL_ID: &str = "settings-popover-v2";

pub(super) fn settings_popup_id() -> egui::Id {
    egui::Id::new(SETTINGS_PANEL_ID)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SettingsAction {
    SetExportFormat(FallbackExportFormat),
    SetRememberExportDir(bool),
    SetUpdateCheckOnStart(bool),
    SetFrameSceneOnOpen(bool),
    SetDoubleClickFocus(bool),
    SetOrbitSensitivity(f32),
    SetZoomSensitivity(f32),
    SetRecentFilesLimit(usize),
    SetViewportBackground(ViewportBackground),
    SetShowCutGhost(bool),
    SetUnitDisplay(UnitDisplay),
    SetTheme(ThemePreference),
    SetUiScale { value: f32, commit: bool },
    SetRememberSculptBrush(bool),
    CheckForUpdates,
    OpenAbout,
}

/// The Settings popover: one scrollable body of labeled sections under a fixed
/// header. Width is fixed (the safe-content contract test pins it at 312
/// points); height yields to the screen. One long body: every section is a
/// flat run of label + rows, so splitting it would just relocate the lines.
#[allow(clippy::too_many_lines)]
pub(super) fn show_settings_popup(
    trigger: &egui::Response,
    settings: &Settings,
    update_status: &UpdateCheckStatus,
    save_error: Option<&str>,
) -> Option<SettingsAction> {
    egui::Popup::from_toggle_button_response(trigger)
        .id(settings_popup_id())
        .align(egui::RectAlign::BOTTOM_END)
        .align_alternatives(&[])
        .gap(4.0)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(312.0)
        .frame(
            egui::Frame::new()
                .fill(ui_theme::panel_fill())
                .stroke(egui::Stroke::new(1.0_f32, ui_theme::panel_stroke()))
                .corner_radius(6)
                .shadow(ui_theme::panel_shadow())
                .inner_margin(egui::Margin::same(PANEL_MARGIN)),
        )
        .show(|ui| {
            let mut action = None;
            ui.set_width(286.0);
            panel_header(ui);
            ui.add_space(7.0);

            // The popover grew from three fields into full preferences; beyond
            // a handful of sections it scrolls instead of escaping the screen.
            // The budget is what sits between the trigger and the screen edge,
            // minus the fixed header/footer chrome around the scroll body.
            let scroll_budget = ui
                .ctx()
                .input(|input| input.raw.screen_rect)
                .map_or(600.0, |rect| rect.height() - trigger.rect.bottom() - 130.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_budget.max(200.0))
                .show(ui, |ui| {
                    ui.set_width(286.0);

                    section_label(ui, "Files");
                    export_format_row(ui, settings, &mut action);

                    let mut remember = settings.remember_export_dir;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut remember, "Remember export folder")
                                .on_hover_text("Use the same folder after restarting OccluView")
                                .changed()
                            {
                                action = Some(SettingsAction::SetRememberExportDir(remember));
                            }
                        },
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Scene & camera");
                    let mut frame_on_open = settings.frame_scene_on_open;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut frame_on_open, "Frame a scene when it opens")
                                .on_hover_text(
                                    "Reset the camera to the home view when a new file \
                                     replaces the scene, instead of keeping the current one",
                                )
                                .changed()
                            {
                                action = Some(SettingsAction::SetFrameSceneOnOpen(frame_on_open));
                            }
                        },
                    );
                    let mut double_click_focus = settings.double_click_resets_camera;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut double_click_focus, "Double-click refocuses view")
                                .on_hover_text(
                                    "A double primary click re-centers the camera on the \
                                     picked point",
                                )
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetDoubleClickFocus(double_click_focus));
                            }
                        },
                    );
                    slider_f32_row(
                        ui,
                        "Orbit speed",
                        settings.orbit_sensitivity,
                        0.25..=4.0,
                        "×",
                        "How fast the view orbits while the right mouse button drags",
                        &mut action,
                        SettingsAction::SetOrbitSensitivity,
                    );
                    slider_f32_row(
                        ui,
                        "Zoom speed",
                        settings.zoom_sensitivity,
                        0.25..=4.0,
                        "×",
                        "How much each scroll notch zooms",
                        &mut action,
                        SettingsAction::SetZoomSensitivity,
                    );
                    slider_usize_row(
                        ui,
                        "Recent scenes",
                        settings.recent_files_limit,
                        4..=20,
                        "Entries kept in the Open chevron",
                        &mut action,
                        SettingsAction::SetRecentFilesLimit,
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Viewport");
                    segmented_row(
                        ui,
                        "Background",
                        settings.viewport_background,
                        &ViewportBackground::OPTIONS,
                        ViewportBackground::label,
                        &mut action,
                        SettingsAction::SetViewportBackground,
                    );
                    let mut ghost = settings.show_cut_ghost;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut ghost, "Ghost the cut-away side")
                                .on_hover_text(
                                    "During a cut view, show the removed side as a \
                                     translucent ghost",
                                )
                                .changed()
                            {
                                action = Some(SettingsAction::SetShowCutGhost(ghost));
                            }
                        },
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Units");
                    segmented_row(
                        ui,
                        "Measurements",
                        settings.unit_display,
                        &UnitDisplay::OPTIONS,
                        UnitDisplay::label,
                        &mut action,
                        SettingsAction::SetUnitDisplay,
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Appearance");
                    segmented_row(
                        ui,
                        "Theme",
                        settings.theme,
                        &ThemePreference::OPTIONS,
                        ThemePreference::label,
                        &mut action,
                        SettingsAction::SetTheme,
                    );
                    slider_f32_row_until_release(
                        ui,
                        "UI scale",
                        settings.ui_scale,
                        0.85..=1.5,
                        "×",
                        "Scales every element; 1.0 keeps the platform default",
                        &mut action,
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Mesh Editing");
                    let mut remember_brush = settings.remember_sculpt_brush;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut remember_brush, "Remember sculpt brush")
                                .on_hover_text(
                                    "Keep the size and intensity sliders between sessions \
                                     instead of resetting them",
                                )
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetRememberSculptBrush(remember_brush));
                            }
                        },
                    );

                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(5.0);
                    section_label(ui, "Updates");
                    let mut check_on_start = settings.update_check_on_start;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut check_on_start, "Check automatically at startup")
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetUpdateCheckOnStart(check_on_start));
                            }
                        },
                    );
                    update_row(ui, update_status, &mut action);
                });

            if save_error.is_some() {
                ui.label(
                    egui::RichText::new("Preferences could not be saved. Retrying…")
                        .size(10.5)
                        .color(ui_theme::danger()),
                )
                .on_hover_text("The settings file is currently unavailable");
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(3.0);
            if ui
                .add(egui::Button::new("About OccluView").frame(false))
                .clicked()
            {
                action = Some(SettingsAction::OpenAbout);
            }
            action
        })
        .and_then(|response| response.inner)
}

fn panel_header(ui: &mut egui::Ui) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), icon_rect, AppIcon::Settings, ui_theme::text());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Settings")
                    .size(14.0)
                    .strong()
                    .color(ui_theme::text()),
            );
        },
    );
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(10.5)
            .strong()
            .color(ui_theme::text_muted()),
    );
}

fn export_format_row(ui: &mut egui::Ui, settings: &Settings, action: &mut Option<SettingsAction>) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label("Export format")
                .on_hover_text("Used when the source format cannot be exported");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                for format in FallbackExportFormat::OPTIONS.into_iter().rev() {
                    if ui
                        .selectable_label(settings.fallback_export_format == format, format.label())
                        .clicked()
                    {
                        *action = Some(SettingsAction::SetExportFormat(format));
                    }
                }
            });
        },
    );
}

const NUMERIC_LABEL_WIDTH: f32 = 96.0;
const NUMERIC_VALUE_WIDTH: f32 = 48.0;
const NUMERIC_SLIDER_MIN_WIDTH: f32 = 72.0;

/// The slider gets the flexible middle column; labels and readouts stay on a
/// stable grid so rows do not jump when a value changes.
fn numeric_slider_width(available_width: f32, item_spacing: f32) -> f32 {
    (available_width - NUMERIC_LABEL_WIDTH - NUMERIC_VALUE_WIDTH - item_spacing * 2.0)
        .max(NUMERIC_SLIDER_MIN_WIDTH)
}

/// One label + slider + readable numeric value. The action fires on every
/// slider tick so the viewport answers immediately.
#[allow(clippy::too_many_arguments)]
fn slider_f32_row(
    ui: &mut egui::Ui,
    label: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    tooltip: &str,
    action: &mut Option<SettingsAction>,
    make: fn(f32) -> SettingsAction,
) {
    slider_f32_row_inner(
        ui,
        label,
        value,
        range,
        suffix,
        tooltip,
        action,
        false,
        |value, _commit| make(value),
    );
}

/// UI Scale is special: changing egui's global zoom changes every widget's
/// geometry. Preview the value while dragging, but only let the app persist it
/// after the pointer is released so the slider cannot move under the pointer.
#[allow(clippy::too_many_arguments)]
fn slider_f32_row_until_release(
    ui: &mut egui::Ui,
    label: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    tooltip: &str,
    action: &mut Option<SettingsAction>,
) {
    slider_f32_row_inner(
        ui,
        label,
        value,
        range,
        suffix,
        tooltip,
        action,
        true,
        |value, commit| SettingsAction::SetUiScale { value, commit },
    );
}

#[allow(clippy::too_many_arguments)]
fn slider_f32_row_inner(
    ui: &mut egui::Ui,
    label: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    tooltip: &str,
    action: &mut Option<SettingsAction>,
    defer_pointer_commit: bool,
    make: impl Fn(f32, bool) -> SettingsAction,
) {
    let row_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(row_width, ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let spacing = ui.spacing().item_spacing.x;
            let label_response = ui.add_sized(
                [NUMERIC_LABEL_WIDTH, 20.0],
                egui::Label::new(egui::RichText::new(label).size(11.5)).truncate(),
            );
            label_response.on_hover_text(tooltip);

            let mut edit = value;
            let slider_response = ui.add_sized(
                [numeric_slider_width(row_width, spacing), 20.0],
                egui::Slider::new(&mut edit, range)
                    .show_value(false)
                    .step_by(0.05)
                    .trailing_fill(true),
            );
            let changed = slider_response.changed();
            let pointer_down = slider_response.is_pointer_button_down_on();
            let drag_stopped = slider_response.drag_stopped();
            slider_response.on_hover_text(tooltip);
            if changed || (defer_pointer_commit && drag_stopped) {
                let commit = !defer_pointer_commit || !pointer_down || drag_stopped;
                *action = Some(make(edit, commit));
            }

            ui.add_sized(
                [NUMERIC_VALUE_WIDTH, 20.0],
                egui::Label::new(
                    egui::RichText::new(format!("{edit:.2}{suffix}"))
                        .size(11.0)
                        .color(ui_theme::text_muted()),
                )
                .halign(egui::Align::RIGHT)
                .truncate(),
            );
        },
    );
}

/// Whole-number variant of [`slider_f32_row`] (no suffix, unit steps).
#[allow(clippy::too_many_arguments)]
fn slider_usize_row(
    ui: &mut egui::Ui,
    label: &str,
    value: usize,
    range: std::ops::RangeInclusive<usize>,
    tooltip: &str,
    action: &mut Option<SettingsAction>,
    make: fn(usize) -> SettingsAction,
) {
    let row_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(row_width, ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let spacing = ui.spacing().item_spacing.x;
            let label_response = ui.add_sized(
                [NUMERIC_LABEL_WIDTH, 20.0],
                egui::Label::new(egui::RichText::new(label).size(11.5)).truncate(),
            );
            label_response.on_hover_text(tooltip);

            let mut edit = value;
            let slider_response = ui.add_sized(
                [numeric_slider_width(row_width, spacing), 20.0],
                egui::Slider::new(&mut edit, range)
                    .show_value(false)
                    .trailing_fill(true),
            );
            let changed = slider_response.changed();
            slider_response.on_hover_text(tooltip);
            if changed {
                *action = Some(make(edit));
            }

            ui.add_sized(
                [NUMERIC_VALUE_WIDTH, 20.0],
                egui::Label::new(
                    egui::RichText::new(edit.to_string())
                        .size(11.0)
                        .color(ui_theme::text_muted()),
                )
                .halign(egui::Align::RIGHT),
            );
        },
    );
}

/// One label + right-aligned segment switch over an enum's fixed options.
#[allow(clippy::too_many_arguments)]
fn segmented_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    current: T,
    options: &[T],
    label_of: fn(T) -> &'static str,
    action: &mut Option<SettingsAction>,
    make: fn(T) -> SettingsAction,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                for option in options.iter().rev() {
                    if ui
                        .selectable_label(current == *option, label_of(*option))
                        .clicked()
                    {
                        *action = Some(make(*option));
                    }
                }
            });
        },
    );
}

fn update_row(ui: &mut egui::Ui, status: &UpdateCheckStatus, action: &mut Option<SettingsAction>) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let enabled = !matches!(
                status,
                UpdateCheckStatus::Disabled | UpdateCheckStatus::Checking
            );
            if ui
                .add_enabled(enabled, egui::Button::new("Check now"))
                .on_disabled_hover_text(match status {
                    UpdateCheckStatus::Disabled => "Update checks are disabled by the environment",
                    _ => "An update check is already running",
                })
                .clicked()
            {
                *action = Some(SettingsAction::CheckForUpdates);
            }
            ui.add_space(5.0);
            let (text, color, detail) = update_status_text(status);
            if !text.is_empty() {
                let response = ui.label(egui::RichText::new(text).size(10.5).color(color));
                if let Some(detail) = detail {
                    response.on_hover_text(detail);
                }
            }
        },
    );
}

fn update_status_text(status: &UpdateCheckStatus) -> (&str, egui::Color32, Option<&str>) {
    match status {
        UpdateCheckStatus::Idle => ("", ui_theme::text_weak(), None),
        UpdateCheckStatus::Disabled => ("Disabled by environment", ui_theme::text_muted(), None),
        UpdateCheckStatus::Checking => ("Checking…", ui_theme::text_weak(), None),
        UpdateCheckStatus::Current => ("Up to date", ui_theme::text_weak(), None),
        UpdateCheckStatus::Available(version) => {
            ("Update available", ui_theme::text(), Some(version.as_str()))
        }
        UpdateCheckStatus::Skipped(version) => (
            "Version skipped",
            ui_theme::text_muted(),
            Some(version.as_str()),
        ),
        UpdateCheckStatus::Failed(error) => {
            ("Couldn’t check", ui_theme::danger(), Some(error.as_str()))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn numeric_preferences_use_slider_tracks_instead_of_stepper_boxes() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_settings_panel.rs"));

        assert!(
            source.contains("egui::Slider::new"),
            "numeric preferences should expose a continuous slider"
        );
        assert!(
            !source.contains("egui::DragValue::new"),
            "numeric preferences should not fall back to compact stepper boxes"
        );
    }
}
