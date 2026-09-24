//! Settings controls and their UI actions.

use crate::app_settings::{Settings, ThemePreference, UnitDisplay, ViewportBackground};
use crate::i18n::catalog::EMBEDDED_TAGS;
use crate::i18n::preference::UiLanguagePreference;
use crate::i18n::{endonym, LocaleManager};
use crate::icons::AppIcon;
use crate::ui_theme;
use crate::update_notice::UpdateCheckStatus;
use eframe::egui;

pub(super) const PANEL_MARGIN: i8 = 12;
/// A control row. Tight enough that the whole panel fits without scrolling on a
/// normal screen, tall enough that a checkbox is still an easy target.
pub(super) const ROW_HEIGHT: f32 = 26.0;
/// Height of the two footer buttons, which are the panel's largest controls.
const FOOTER_BUTTON_HEIGHT: f32 = 26.0;
pub(super) const SETTINGS_PANEL_ID: &str = "settings-popover-v2";

pub(super) fn settings_popup_id() -> egui::Id {
    egui::Id::new(SETTINGS_PANEL_ID)
}

pub(super) fn show_settings_toolbar_toggle(
    ui: &mut egui::Ui,
    enabled: bool,
    locale: &LocaleManager,
) -> egui::Response {
    crate::measure_overlay::toolbar_toggle(
        ui,
        crate::measure_overlay::ToolbarToggle::new(
            AppIcon::Settings,
            &locale.tr("toolbar-settings-label"),
            enabled,
            egui::Popup::is_id_open(ui.ctx(), settings_popup_id()),
            &locale.tr("toolbar-settings-hint"),
        ),
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SettingsAction {
    SetRememberExportDir(bool),
    SetUpdateCheckOnStart(bool),
    SetFrameSceneOnOpen(bool),
    SetDoubleClickFocus(bool),
    SetOrbitSensitivity(f32),
    SetZoomSensitivity(f32),
    SetViewportBackground(ViewportBackground),
    SetShowCutGhost(bool),
    SetUnitDisplay(UnitDisplay),
    SetTheme(ThemePreference),
    SetUiScale {
        value: f32,
        commit: bool,
    },
    SetRememberSculptBrush(bool),
    SetExplicitLanguage(&'static str),
    UseSystemLanguage,
    CheckForUpdates,
    OpenAbout,
    /// Open the keyboard and mouse reference.
    OpenShortcuts,
}

/// Scrollable Settings popup with fixed width and screen-bounded height.
#[allow(clippy::too_many_lines)]
#[expect(
    clippy::too_many_arguments,
    reason = "the UI boundary needs settings, locale, and persistence status"
)]
pub(super) fn show_settings_popup(
    trigger: &egui::Response,
    settings: &Settings,
    locale: &LocaleManager,
    update_status: &UpdateCheckStatus,
    save_error: Option<&str>,
    language_save_error: Option<&str>,
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
            panel_header(ui, locale);
            ui.add_space(4.0);

            // Keep the popup inside the remaining screen height.
            let scroll_budget = ui
                .ctx()
                .input(|input| input.raw.screen_rect)
                .map_or(600.0, |rect| rect.height() - trigger.rect.bottom() - 130.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_budget.max(200.0))
                .show(ui, |ui| {
                    ui.set_width(286.0);

                    section_label(ui, &locale.tr("settings-section-appearance"));
                    segmented_row(
                        ui,
                        locale,
                        &locale.tr("settings-theme"),
                        settings.theme,
                        &ThemePreference::OPTIONS,
                        |option, locale| locale.tr(theme_key(option)),
                        &mut action,
                        SettingsAction::SetTheme,
                    );
                    slider_f32_row_until_release(
                        ui,
                        &locale.tr("settings-scale"),
                        settings.ui_scale,
                        0.85..=1.5,
                        "×",
                        &locale.tr("settings-scale-hint"),
                        &mut action,
                    );
                    language_section(ui, locale, &mut action);
                    if language_save_error.is_some() {
                        ui.label(
                            egui::RichText::new(locale.text("settings-language-save-error"))
                                .size(10.5)
                                .color(ui_theme::danger()),
                        );
                    }

                    section_break(ui);
                    section_label(ui, &locale.tr("settings-section-files"));
                    let mut remember = settings.remember_export_dir;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut remember, locale.tr("settings-remember-export"))
                                .on_hover_text(locale.tr("settings-remember-export-hint"))
                                .changed()
                            {
                                action = Some(SettingsAction::SetRememberExportDir(remember));
                            }
                        },
                    );
                    section_break(ui);
                    section_label(ui, &locale.tr("settings-section-scene"));
                    segmented_row(
                        ui,
                        locale,
                        &locale.tr("settings-background"),
                        settings.viewport_background,
                        &ViewportBackground::OPTIONS,
                        |option, locale| locale.tr(background_key(option)),
                        &mut action,
                        SettingsAction::SetViewportBackground,
                    );
                    let mut ghost = settings.show_cut_ghost;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut ghost, locale.tr("settings-ghost"))
                                .on_hover_text(locale.tr("settings-ghost-hint"))
                                .changed()
                            {
                                action = Some(SettingsAction::SetShowCutGhost(ghost));
                            }
                        },
                    );
                    segmented_row(
                        ui,
                        locale,
                        &locale.tr("settings-measurements"),
                        settings.unit_display,
                        &UnitDisplay::OPTIONS,
                        |option, _locale| option.label().to_owned(),
                        &mut action,
                        SettingsAction::SetUnitDisplay,
                    );
                    let mut frame_on_open = settings.frame_scene_on_open;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut frame_on_open, locale.tr("settings-frame-on-open"))
                                .on_hover_text(locale.tr("settings-frame-on-open-hint"))
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
                                .checkbox(
                                    &mut double_click_focus,
                                    locale.tr("settings-double-click"),
                                )
                                .on_hover_text(locale.tr("settings-double-click-hint"))
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetDoubleClickFocus(double_click_focus));
                            }
                        },
                    );
                    slider_f32_row(
                        ui,
                        &locale.tr("settings-orbit"),
                        settings.orbit_sensitivity,
                        0.25..=4.0,
                        "×",
                        &locale.tr("settings-orbit-hint"),
                        &mut action,
                        SettingsAction::SetOrbitSensitivity,
                    );
                    slider_f32_row(
                        ui,
                        &locale.tr("settings-zoom"),
                        settings.zoom_sensitivity,
                        0.25..=4.0,
                        "×",
                        &locale.tr("settings-zoom-hint"),
                        &mut action,
                        SettingsAction::SetZoomSensitivity,
                    );

                    section_break(ui);
                    section_label(ui, &locale.tr("settings-section-mesh"));
                    let mut remember_brush = settings.remember_sculpt_brush;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut remember_brush, locale.tr("settings-remember-brush"))
                                .on_hover_text(locale.tr("settings-remember-brush-hint"))
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetRememberSculptBrush(remember_brush));
                            }
                        },
                    );

                    section_break(ui);
                    section_label(ui, &locale.tr("settings-section-updates"));
                    let mut check_on_start = settings.update_check_on_start;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .checkbox(&mut check_on_start, locale.tr("settings-check-auto"))
                                .changed()
                            {
                                action =
                                    Some(SettingsAction::SetUpdateCheckOnStart(check_on_start));
                            }
                        },
                    );
                    update_row(ui, update_status, locale, &mut action);
                });

            if save_error.is_some() {
                ui.label(
                    egui::RichText::new(locale.tr("settings-save-error"))
                        .size(10.5)
                        .color(ui_theme::danger()),
                )
                .on_hover_text(locale.tr("settings-save-error-hint"));
            }

            ui.add_space(2.0);
            ui.separator();
            ui.add_space(2.0);
            // Both references sit on one row of two equal, full-height buttons
            // rather than as two bare text lines. They are the only ways into
            // the shortcut list and the about box, and a real button is both
            // easier to hit and shorter to read than a link-shaped label.
            let button_width = (ui.available_width() - 6.0) * 0.5;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if ui
                    .add_sized(
                        [button_width, FOOTER_BUTTON_HEIGHT],
                        egui::Button::new(locale.tr("settings-shortcuts")),
                    )
                    .on_hover_text(locale.tr("settings-shortcuts-hint"))
                    .clicked()
                {
                    action = Some(SettingsAction::OpenShortcuts);
                }
                if ui
                    .add_sized(
                        [button_width, FOOTER_BUTTON_HEIGHT],
                        egui::Button::new(locale.tr("settings-about")),
                    )
                    .clicked()
                {
                    action = Some(SettingsAction::OpenAbout);
                }
            });
            action
        })
        .and_then(|response| response.inner)
}

fn panel_header(ui: &mut egui::Ui, locale: &LocaleManager) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), icon_rect, AppIcon::Settings, ui_theme::text());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(locale.tr("settings-header"))
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

fn section_break(ui: &mut egui::Ui) {
    ui.add_space(1.0);
    ui.separator();
    ui.add_space(1.0);
}

const NUMERIC_LABEL_WIDTH: f32 = 96.0;
const NUMERIC_VALUE_WIDTH: f32 = 48.0;
const NUMERIC_SLIDER_MIN_WIDTH: f32 = 72.0;

/// Labels and readouts stay aligned while the slider takes remaining width.
fn numeric_slider_width(available_width: f32, item_spacing: f32) -> f32 {
    (available_width - NUMERIC_LABEL_WIDTH - NUMERIC_VALUE_WIDTH - item_spacing * 2.0)
        .max(NUMERIC_SLIDER_MIN_WIDTH)
}

/// Numeric slider row with immediate preview.
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

/// Preview UI scale during drag; persist after release so widgets stay stable.
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

fn background_key(option: ViewportBackground) -> &'static str {
    match option {
        ViewportBackground::Gray => "settings-bg-gray",
        ViewportBackground::White => "settings-bg-white",
        ViewportBackground::Dark => "settings-bg-dark",
    }
}

fn theme_key(option: ThemePreference) -> &'static str {
    match option {
        ThemePreference::Light => "settings-theme-light",
        ThemePreference::Dark => "settings-theme-dark",
    }
}

/// Label plus right-aligned enum selector.
#[allow(clippy::too_many_arguments)]
fn segmented_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    locale: &LocaleManager,
    label: &str,
    current: T,
    options: &[T],
    label_of: impl Fn(T, &LocaleManager) -> String,
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
                        .selectable_label(current == *option, label_of(*option, locale))
                        .clicked()
                    {
                        *action = Some(make(*option));
                    }
                }
            });
        },
    );
}

const LANGUAGE_SELECTOR_ID: &str = "settings-language-selector-v2";
// Internal radio sentinel for the System row: never a catalog tag, so it can
// never collide with an entry of `EMBEDDED_TAGS`.
const SYSTEM_LANGUAGE_OPTION: &str = "__system_language__";

/// Inline selector. It must not open another popup.
fn language_section(
    ui: &mut egui::Ui,
    locale: &LocaleManager,
    action: &mut Option<SettingsAction>,
) {
    let header = format!(
        "{} · {}",
        locale.text("settings-language-label"),
        selected_language_summary(locale)
    );
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = ROW_HEIGHT;
        ui.visuals_mut().collapsing_header_frame = true;
        egui::CollapsingHeader::new(header)
            .id_salt(LANGUAGE_SELECTOR_ID)
            .show(ui, |ui| {
                let mut selected = match locale.snapshot().preference {
                    UiLanguagePreference::Auto => SYSTEM_LANGUAGE_OPTION,
                    UiLanguagePreference::Explicit(tag) => tag,
                };
                if ui
                    .radio_value(
                        &mut selected,
                        SYSTEM_LANGUAGE_OPTION,
                        system_language_choice_label(locale),
                    )
                    .clicked()
                {
                    *action = Some(SettingsAction::UseSystemLanguage);
                    ui.close();
                    ui.ctx().request_repaint();
                }
                for tag in EMBEDDED_TAGS {
                    if ui.radio_value(&mut selected, tag, endonym(tag)).clicked() {
                        *action = Some(SettingsAction::SetExplicitLanguage(tag));
                        ui.close();
                        ui.ctx().request_repaint();
                    }
                }
                if let Some(tag) = active_language_fallback_tag(locale) {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(
                            locale.tr_with("settings-language-catalog-fallback", &[("tag", tag)]),
                        )
                        .size(10.5)
                        .color(ui_theme::text_muted()),
                    );
                }
            });
    });
}

fn selected_language_summary(locale: &LocaleManager) -> String {
    match &locale.snapshot().preference {
        UiLanguagePreference::Auto => locale.text("settings-language-auto"),
        UiLanguagePreference::Explicit(tag) if *tag == locale.snapshot().render_tag => {
            endonym(tag).to_owned()
        }
        UiLanguagePreference::Explicit(_) => endonym(locale.snapshot().render_tag).to_owned(),
    }
}

fn system_language_choice_label(locale: &LocaleManager) -> String {
    locale.tr_with(
        "settings-language-auto-current",
        &[("language", endonym(locale.system_render_tag()))],
    )
}

fn active_language_fallback_tag(locale: &LocaleManager) -> Option<&'static str> {
    let snapshot = locale.snapshot();
    (snapshot.active_tag != snapshot.render_tag).then_some(snapshot.active_tag)
}

fn update_row(
    ui: &mut egui::Ui,
    status: &UpdateCheckStatus,
    locale: &LocaleManager,
    action: &mut Option<SettingsAction>,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let enabled = !matches!(
                status,
                UpdateCheckStatus::Disabled | UpdateCheckStatus::Checking
            );
            if ui
                .add_enabled(enabled, egui::Button::new(locale.tr("settings-check-now")))
                .on_disabled_hover_text(match status {
                    UpdateCheckStatus::Disabled => locale.tr("settings-check-disabled-hint"),
                    _ => locale.tr("settings-check-busy-hint"),
                })
                .clicked()
            {
                *action = Some(SettingsAction::CheckForUpdates);
            }
            ui.add_space(5.0);
            let (text, color, detail) = update_status_text(status, locale);
            if !text.is_empty() {
                let response = ui.label(egui::RichText::new(text).size(10.5).color(color));
                if let Some(detail) = detail {
                    response.on_hover_text(detail);
                }
            }
        },
    );
}

fn update_status_text<'a>(
    status: &'a UpdateCheckStatus,
    locale: &LocaleManager,
) -> (String, egui::Color32, Option<&'a str>) {
    match status {
        UpdateCheckStatus::Idle => (String::new(), ui_theme::text_weak(), None),
        UpdateCheckStatus::Disabled => (
            locale.tr("settings-update-disabled"),
            ui_theme::text_muted(),
            None,
        ),
        UpdateCheckStatus::Checking => (
            locale.tr("settings-update-checking"),
            ui_theme::text_weak(),
            None,
        ),
        UpdateCheckStatus::Current => (
            locale.tr("settings-update-current"),
            ui_theme::text_weak(),
            None,
        ),
        UpdateCheckStatus::Available(version) => (
            locale.tr("update-available-title"),
            ui_theme::text(),
            Some(version.as_str()),
        ),
        UpdateCheckStatus::Skipped(version) => (
            locale.tr("settings-update-skipped"),
            ui_theme::text_muted(),
            Some(version.as_str()),
        ),
        UpdateCheckStatus::Failed(error) => (
            locale.tr("settings-update-failed"),
            ui_theme::danger(),
            Some(error.as_str()),
        ),
    }
}
#[cfg(test)]
#[path = "app_settings_panel_shots.rs"]
mod app_settings_panel_shots;
