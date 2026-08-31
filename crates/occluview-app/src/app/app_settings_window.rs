//! Compact preferences popover and the separate About dialog.

use super::OccluViewApp;
use crate::app_settings::{FallbackExportFormat, Settings};
use crate::icons::AppIcon;
use crate::ui_theme;
use crate::update_notice::UpdateCheckStatus;
use eframe::egui;

const PANEL_CONTENT_WIDTH: f32 = 320.0;
const PANEL_MARGIN: i8 = 12;
const ROW_HEIGHT: f32 = 30.0;
const SETTINGS_PANEL_ID: &str = "settings-popover-v2";

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsAction {
    SetExportFormat(FallbackExportFormat),
    SetRememberExportDir(bool),
    SetUpdateCheckOnStart(bool),
    CheckForUpdates,
    OpenAbout,
}

struct SettingsPanelResponse {
    actions: Vec<SettingsAction>,
    dismissed: bool,
}

impl OccluViewApp {
    pub(super) fn show_settings_popover(&mut self, ctx: &egui::Context, anchor: egui::Rect) {
        if !self.settings_window.open {
            return;
        }

        let response = show_settings_panel(
            ctx,
            anchor,
            &self.settings,
            self.update_notice.check_status(),
            self.settings_persistence.error(),
        );

        for action in response.actions {
            match action {
                SettingsAction::SetExportFormat(format) => {
                    self.settings.fallback_export_format = format;
                    self.settings_persistence.mark_dirty();
                }
                SettingsAction::SetRememberExportDir(remember) => {
                    self.settings
                        .set_remember_export_dir(remember, self.last_export_dir.as_deref());
                    self.settings_persistence.mark_dirty();
                }
                SettingsAction::SetUpdateCheckOnStart(enabled) => {
                    self.settings.update_check_on_start = enabled;
                    self.settings_persistence.mark_dirty();
                }
                SettingsAction::CheckForUpdates => self.update_notice.request_check(ctx),
                SettingsAction::OpenAbout => {
                    self.settings_window.open = false;
                    self.settings_window.about_open = true;
                }
            }
        }

        if response.dismissed {
            self.settings_window.open = false;
        }
    }

    pub(super) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_window.about_open {
            return;
        }

        let other_modal = self.third_party_window_open
            || self.close_guard_open
            || self.pending_replace_open.is_some()
            || self.app_error.is_some();
        let mut close = !other_modal && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let mut open_third_party = false;
        let mut open_url = None;
        let logo = self.app_logo_texture(ctx).cloned();
        let mut open = true;

        egui::Window::new("About OccluView")
            .id(egui::Id::new("occluview-about-dialog-v2"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .constrain_to(ctx.content_rect().shrink(8.0))
            .open(&mut open)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .auto_sized()
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(304.0);
                ui.vertical_centered(|ui| {
                    if let Some(logo) = &logo {
                        ui.add(egui::Image::new((logo.id(), egui::vec2(48.0, 48.0))));
                    }
                    ui.label(
                        egui::RichText::new("OccluView")
                            .size(19.0)
                            .strong()
                            .color(ui_theme::TEXT),
                    );
                    ui.label(
                        egui::RichText::new("3D viewer for dental scans")
                            .size(12.0)
                            .color(ui_theme::TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .color(ui_theme::TEXT_MUTED),
                    );
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let width =
                        ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(1.0);
                    if about_link(ui, width, AppIcon::Globe, "Website") {
                        open_url = Some("https://occlutrace.ai");
                    }
                    if about_link(ui, width, AppIcon::Github, "Source") {
                        open_url = Some("https://github.com/occlutrace/OccluView");
                    }
                });
                let licenses_width = ui.available_width();
                if about_link(
                    ui,
                    licenses_width,
                    AppIcon::Licenses,
                    "Third-party licenses",
                ) {
                    open_third_party = true;
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Apache License 2.0")
                            .size(10.5)
                            .color(ui_theme::TEXT_MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });

        if let Some(url) = open_url {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        if open_third_party {
            self.third_party_window_open = true;
        }
        if close || !open {
            self.settings_window.about_open = false;
        }
    }
}

fn show_settings_panel(
    ctx: &egui::Context,
    anchor: egui::Rect,
    settings: &Settings,
    update_status: &UpdateCheckStatus,
    save_error: Option<&str>,
) -> SettingsPanelResponse {
    let mut actions = Vec::new();
    let screen = ctx.content_rect().shrink(8.0);
    let position = anchor.right_bottom() + egui::vec2(0.0, 4.0);
    let area = egui::Area::new(egui::Id::new(SETTINGS_PANEL_ID))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(position)
        .constrain_to(screen)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(egui::Color32::from_rgb(250, 251, 252))
                .stroke(egui::Stroke::new(1.0_f32, ui_theme::panel_stroke()))
                .corner_radius(8)
                .shadow(ui_theme::panel_shadow())
                .inner_margin(egui::Margin::same(PANEL_MARGIN))
                .show(ui, |ui| {
                    ui.set_width(PANEL_CONTENT_WIDTH);
                    panel_header(ui);
                    ui.add_space(7.0);
                    section_label(ui, "Files");
                    export_format_row(ui, settings, &mut actions);

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
                                actions.push(SettingsAction::SetRememberExportDir(remember));
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
                                actions.push(SettingsAction::SetUpdateCheckOnStart(check_on_start));
                            }
                        },
                    );
                    update_row(ui, update_status, &mut actions);

                    if save_error.is_some() {
                        ui.label(
                            egui::RichText::new("Preferences could not be saved. Retrying…")
                                .size(10.5)
                                .color(ui_theme::DANGER),
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
                        actions.push(SettingsAction::OpenAbout);
                    }
                });
        });

    let rect = area.response.rect;
    let popup_open = egui::Popup::is_any_open(ctx);
    let escape = !popup_open && ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let outside_press = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|position| !rect.contains(position) && !anchor.contains(position))
    });
    let dismissed = escape || (outside_press && actions.is_empty() && !popup_open);

    SettingsPanelResponse { actions, dismissed }
}

fn panel_header(ui: &mut egui::Ui) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), icon_rect, AppIcon::Settings, ui_theme::TEXT);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Preferences")
                    .size(14.0)
                    .strong()
                    .color(ui_theme::TEXT),
            );
        },
    );
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(10.5)
            .strong()
            .color(ui_theme::TEXT_MUTED),
    );
}

fn export_format_row(ui: &mut egui::Ui, settings: &Settings, actions: &mut Vec<SettingsAction>) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label("Fallback format")
                .on_hover_text("Used when the source format cannot be exported");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("settings-export-format")
                    .selected_text(settings.fallback_export_format.label())
                    .width(126.0)
                    .show_ui(ui, |ui| {
                        for format in FallbackExportFormat::OPTIONS {
                            if ui
                                .selectable_label(
                                    settings.fallback_export_format == format,
                                    format.label(),
                                )
                                .clicked()
                            {
                                actions.push(SettingsAction::SetExportFormat(format));
                            }
                        }
                    });
            });
        },
    );
}

fn update_row(ui: &mut egui::Ui, status: &UpdateCheckStatus, actions: &mut Vec<SettingsAction>) {
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
                actions.push(SettingsAction::CheckForUpdates);
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
        UpdateCheckStatus::Idle => ("", ui_theme::TEXT_WEAK, None),
        UpdateCheckStatus::Disabled => ("Disabled by environment", ui_theme::TEXT_MUTED, None),
        UpdateCheckStatus::Checking => ("Checking…", ui_theme::TEXT_WEAK, None),
        UpdateCheckStatus::Current => ("Up to date", ui_theme::TEXT_WEAK, None),
        UpdateCheckStatus::Available(version) => {
            ("Update available", ui_theme::TEXT, Some(version.as_str()))
        }
        UpdateCheckStatus::Skipped(version) => (
            "Version skipped",
            ui_theme::TEXT_MUTED,
            Some(version.as_str()),
        ),
        UpdateCheckStatus::Failed(error) => {
            ("Couldn’t check", ui_theme::DANGER, Some(error.as_str()))
        }
    }
}

fn about_link(ui: &mut egui::Ui, width: f32, icon: AppIcon, label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 27.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui_theme::RADIUS_CONTROL,
            ui_theme::ACCENT.gamma_multiply(0.08),
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 13.0, rect.center().y),
        egui::vec2(14.0, 14.0),
    );
    crate::icons::paint(ui.painter(), icon_rect, icon, ui_theme::TEXT_WEAK);
    ui.painter().text(
        egui::pos2(icon_rect.right() + 7.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        ui_theme::TEXT,
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 384.0))
    }

    fn test_anchor() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(410.0, 4.0), egui::pos2(492.0, 30.0))
    }

    fn run_settings_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> anyhow::Result<(SettingsPanelResponse, egui::FullOutput)> {
        let input = egui::RawInput {
            screen_rect: Some(test_screen()),
            events,
            ..Default::default()
        };
        let mut response = None;
        let mut output = ctx.run_ui(input, |ui| {
            response = Some(show_settings_panel(
                ui.ctx(),
                test_anchor(),
                &Settings::default(),
                &UpdateCheckStatus::Idle,
                None,
            ));
        });
        let response = response
            .ok_or_else(|| anyhow::anyhow!("the production settings panel should render"))?;
        // These tests inspect shapes and widget state but intentionally do not
        // mount an egui renderer. eframe applies these deltas in production.
        output.textures_delta.clear();
        Ok((response, output))
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn rendered_panel_rect(ctx: &egui::Context) -> anyhow::Result<egui::Rect> {
        ctx.memory(|memory| memory.area_rect(egui::Id::new(SETTINGS_PANEL_ID)))
            .ok_or_else(|| anyhow::anyhow!("the production settings panel should render"))
    }

    fn point_outside_panel(panel: egui::Rect) -> anyhow::Result<egui::Pos2> {
        let screen = test_screen();
        let outside = egui::pos2(f32::midpoint(screen.left(), panel.left()), panel.center().y);
        if !screen.contains(outside) || panel.contains(outside) || test_anchor().contains(outside) {
            return Err(anyhow::anyhow!(
                "derived outside point {outside:?} was invalid for panel {panel:?}"
            ));
        }
        Ok(outside)
    }

    #[test]
    fn settings_does_not_dismiss_while_child_combo_popup_is_open() -> anyhow::Result<()> {
        let closed_ctx = egui::Context::default();
        let _ = run_settings_frame(&closed_ctx, Vec::new())?;
        let _ = run_settings_frame(&closed_ctx, Vec::new())?;
        let closed_panel = rendered_panel_rect(&closed_ctx)?;
        let outside = point_outside_panel(closed_panel)?;
        assert!(!closed_panel.contains(outside));

        let (while_child_closed, _) = run_settings_frame(
            &closed_ctx,
            vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, true),
            ],
        )?;
        assert!(
            while_child_closed.dismissed,
            "the production panel must dismiss on the outside press when its ComboBox is closed"
        );

        let open_ctx = egui::Context::default();
        let _ = run_settings_frame(&open_ctx, Vec::new())?;
        let (_, visible_frame) = run_settings_frame(&open_ctx, Vec::new())?;
        let combo = visible_frame
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "PLY" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("the production export-format control should render"))?;

        let _ = run_settings_frame(&open_ctx, vec![egui::Event::PointerMoved(combo)])?;
        let _ = run_settings_frame(
            &open_ctx,
            vec![
                egui::Event::PointerMoved(combo),
                pointer_button(combo, true),
            ],
        )?;
        let (opened, _) = run_settings_frame(
            &open_ctx,
            vec![
                egui::Event::PointerMoved(combo),
                pointer_button(combo, false),
            ],
        )?;
        assert!(!opened.dismissed);
        assert!(
            egui::Popup::is_any_open(&open_ctx),
            "the production export-format ComboBox should be open"
        );
        let (empty_open_frame, _) = run_settings_frame(&open_ctx, Vec::new())?;
        assert!(
            !empty_open_frame.dismissed,
            "an open child must keep Settings alive across an empty frame"
        );
        let open_panel = rendered_panel_rect(&open_ctx)?;
        assert!(
            !open_panel.contains(outside),
            "the same derived point must be outside the rendered open panel"
        );

        let (while_child_open, _) = run_settings_frame(
            &open_ctx,
            vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, true),
            ],
        )?;

        assert!(
            !while_child_open.dismissed,
            "the first outside press belongs to the open child popup"
        );

        let (after_child_release, _) = run_settings_frame(
            &open_ctx,
            vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, false),
            ],
        )?;
        assert!(
            !after_child_release.dismissed,
            "closing the child must not also dismiss its Settings parent"
        );

        let (second_outside_press, _) = run_settings_frame(
            &open_ctx,
            vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, true),
            ],
        )?;
        assert!(
            second_outside_press.dismissed,
            "a later outside click must dismiss Settings once the child is closed"
        );
        Ok(())
    }

    #[test]
    fn production_settings_panel_fits_a_small_viewer_window() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(test_screen()),
            safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32::same(4.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            let _ = show_settings_panel(
                ui.ctx(),
                test_anchor(),
                &Settings::default(),
                &UpdateCheckStatus::Failed("network unavailable".to_string()),
                Some("read-only settings directory"),
            );
        })
        .drop_without_applying_deltas();

        let Some(rect) = ctx.memory(|memory| memory.area_rect(egui::Id::new(SETTINGS_PANEL_ID)))
        else {
            return Err(anyhow::anyhow!("the production panel should render"));
        };
        assert!(
            (330.0..=360.0).contains(&rect.width()),
            "width was {}",
            rect.width()
        );
        assert!(rect.height() <= 290.0, "height was {}", rect.height());
        let allowed = ctx.content_rect().shrink(8.0);
        assert!(
            allowed.contains_rect(rect),
            "panel {rect:?} escaped safe content bounds {allowed:?}"
        );
        Ok(())
    }
}
