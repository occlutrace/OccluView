//! Compact preferences popover and the product-information modals.

use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use crate::app_settings::{FallbackExportFormat, Settings};
use crate::icons::AppIcon;
use crate::ui_theme;
use crate::update_notice::UpdateCheckStatus;
use eframe::egui;

const PANEL_MARGIN: i8 = 12;
const ROW_HEIGHT: f32 = 30.0;
const SETTINGS_PANEL_ID: &str = "settings-popover-v2";
const INFORMATION_MODAL_BACKDROP_ALPHA: u8 = 48;

pub(super) fn settings_popup_id() -> egui::Id {
    egui::Id::new(SETTINGS_PANEL_ID)
}

pub(super) fn information_modal(
    ctx: &egui::Context,
    id: egui::Id,
    default_size: egui::Vec2,
) -> egui::Modal {
    let bounds = ctx.content_rect().shrink(16.0);
    egui::Modal::new(id)
        .area(
            egui::Modal::default_area(id)
                .default_size(default_size.min(bounds.size()))
                .constrain_to(bounds),
        )
        .frame(ui_theme::overlay_frame())
        .backdrop_color(egui::Color32::from_black_alpha(
            INFORMATION_MODAL_BACKDROP_ALPHA,
        ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsAction {
    SetExportFormat(FallbackExportFormat),
    SetRememberExportDir(bool),
    SetUpdateCheckOnStart(bool),
    CheckForUpdates,
    OpenAbout,
}

impl OccluViewApp {
    pub(super) fn show_settings_popup(&mut self, trigger: &egui::Response) {
        let Some(action) = show_settings_popup(
            trigger,
            &self.settings,
            self.update_notice.check_status(),
            self.settings_persistence.error(),
        ) else {
            return;
        };

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
            SettingsAction::CheckForUpdates => self.update_notice.request_check(&trigger.ctx),
            SettingsAction::OpenAbout => {
                egui::Popup::close_id(&trigger.ctx, settings_popup_id());
                self.information_dialog = InformationDialog::About;
            }
        }
    }

    pub(super) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        if self.information_dialog != InformationDialog::About {
            return;
        }

        let mut close = false;
        let mut open_third_party = false;
        let mut open_url = None;
        let logo = self.app_logo_texture(ctx).cloned();

        let modal_response = information_modal(
            ctx,
            egui::Id::new("occluview-about-dialog-v2"),
            egui::vec2(320.0, 240.0),
        )
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
                let width = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(1.0);
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
            self.information_dialog = InformationDialog::ThirdPartyNotices;
        } else if close || modal_response.should_close() {
            self.information_dialog = InformationDialog::None;
        }
    }
}

fn show_settings_popup(
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
                .fill(egui::Color32::WHITE)
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
                        action = Some(SettingsAction::SetUpdateCheckOnStart(check_on_start));
                    }
                },
            );
            update_row(ui, update_status, &mut action);

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
            crate::icons::paint(ui.painter(), icon_rect, AppIcon::Settings, ui_theme::TEXT);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Settings")
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
    use crate::app::app_dialogs::{
        recent_files_popup_id, show_recent_files_popup, show_settings_toolbar_toggle,
    };
    use crate::recent_files::RecentFiles;

    fn test_screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 384.0))
    }

    struct ToolbarFrame {
        action: Option<SettingsAction>,
        settings_trigger: egui::Rect,
        recent_trigger: egui::Rect,
        output: egui::FullOutput,
    }

    fn run_toolbar_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> anyhow::Result<ToolbarFrame> {
        let input = egui::RawInput {
            screen_rect: Some(test_screen()),
            safe_area_insets: Some(egui::SafeAreaInsets(egui::Margin::same(4).into())),
            events,
            ..Default::default()
        };
        let mut action = None;
        let mut settings_trigger = None;
        let mut recent_trigger = None;
        let mut recent = RecentFiles::new(1);
        recent.push("case.stl");
        let mut output = ctx.run_ui(input, |ui| {
            egui::Panel::top("settings-test-toolbar")
                .exact_size(30.0)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let settings = show_settings_toolbar_toggle(ui, true);
                        settings_trigger = Some(settings.rect);
                        action = show_settings_popup(
                            &settings,
                            &Settings::default(),
                            &UpdateCheckStatus::Idle,
                            None,
                        );

                        let recent_trigger_response =
                            ui.add(egui::Button::new("Recent").min_size(egui::vec2(64.0, 22.0)));
                        recent_trigger = Some(recent_trigger_response.rect);
                        let _ = show_recent_files_popup(&recent_trigger_response, &recent);
                    });
                });
        });
        output.textures_delta.clear();
        Ok(ToolbarFrame {
            action,
            settings_trigger: settings_trigger.ok_or_else(|| {
                anyhow::anyhow!("the toolbar-like Settings trigger should render")
            })?,
            recent_trigger: recent_trigger
                .ok_or_else(|| anyhow::anyhow!("the toolbar-like Recent trigger should render"))?,
            output,
        })
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(ctx: &egui::Context, position: egui::Pos2) -> anyhow::Result<ToolbarFrame> {
        let _ = run_toolbar_frame(
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                pointer_button(position, true),
            ],
        )?;
        run_toolbar_frame(
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                pointer_button(position, false),
            ],
        )
    }

    fn direct_control_center(output: &egui::FullOutput, label: &str) -> anyhow::Result<egui::Pos2> {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("the production Settings popup should render {label}"))
    }

    fn popup_rect(ctx: &egui::Context, id: egui::Id) -> anyhow::Result<egui::Rect> {
        ctx.memory(|memory| memory.area_rect(id))
            .ok_or_else(|| anyhow::anyhow!("the production popup {id:?} should render"))
    }

    struct ModalFrame {
        should_close: bool,
        backdrop_clicked: bool,
    }

    fn run_modal_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> ModalFrame {
        let input = egui::RawInput {
            screen_rect: Some(test_screen()),
            events,
            ..Default::default()
        };
        let mut should_close = false;
        let mut backdrop_clicked = false;
        ctx.run_ui(input, |ui| {
            let response = egui::Modal::new(egui::Id::new("about-modal-close-contract")).show(
                ui.ctx(),
                |ui| {
                    ui.set_min_size(egui::vec2(160.0, 96.0));
                },
            );
            backdrop_clicked = response.backdrop_response.clicked();
            should_close = response.should_close();
        })
        .drop_without_applying_deltas();
        ModalFrame {
            should_close,
            backdrop_clicked,
        }
    }

    #[test]
    fn settings_segment_selects_direct_stl_without_closing() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let initial = run_toolbar_frame(&ctx, Vec::new())?;
        let _ = click(&ctx, initial.settings_trigger.center())?;
        let visible = run_toolbar_frame(&ctx, Vec::new())?;
        let stl = direct_control_center(&visible.output, "STL")?;

        let response = click(&ctx, stl)?;

        assert_eq!(
            response.action,
            Some(SettingsAction::SetExportFormat(FallbackExportFormat::Stl))
        );
        assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
        Ok(())
    }

    #[test]
    fn settings_toolbar_active_state_follows_popup_memory() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let initial = run_toolbar_frame(&ctx, Vec::new())?;
        assert!(
            (initial.settings_trigger.height() - 22.0).abs() <= 0.01,
            "inactive Settings toolbar height changed"
        );

        let _ = click(&ctx, initial.settings_trigger.center())?;
        assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));

        let active = run_toolbar_frame(&ctx, Vec::new())?;
        assert!(
            (active.settings_trigger.height() - 26.0).abs() <= 0.01,
            "active Settings toolbar height changed"
        );

        let _ = click(&ctx, active.settings_trigger.center())?;
        assert!(!egui::Popup::is_id_open(&ctx, settings_popup_id()));

        let inactive = run_toolbar_frame(&ctx, Vec::new())?;
        assert!(
            (inactive.settings_trigger.height() - 22.0).abs() <= 0.01,
            "inactive Settings toolbar height changed"
        );
        Ok(())
    }

    #[test]
    fn settings_switches_to_recent_popup() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let initial = run_toolbar_frame(&ctx, Vec::new())?;
        let open_settings = click(&ctx, initial.settings_trigger.center())?;
        assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));

        let _ = click(&ctx, open_settings.recent_trigger.center())?;

        assert!(!egui::Popup::is_id_open(&ctx, settings_popup_id()));
        assert!(egui::Popup::is_id_open(&ctx, recent_files_popup_id()));
        Ok(())
    }

    #[test]
    fn settings_dismisses_on_outside_click_and_escape() -> anyhow::Result<()> {
        let click_ctx = egui::Context::default();
        let initial = run_toolbar_frame(&click_ctx, Vec::new())?;
        let _ = click(&click_ctx, initial.settings_trigger.center())?;
        let settings = popup_rect(&click_ctx, settings_popup_id())?;
        let outside = egui::pos2(4.0, test_screen().bottom() - 4.0);
        assert!(!settings.contains(outside));
        let _ = click(&click_ctx, outside)?;
        assert!(!egui::Popup::is_id_open(&click_ctx, settings_popup_id()));

        let escape_ctx = egui::Context::default();
        let initial = run_toolbar_frame(&escape_ctx, Vec::new())?;
        let _ = click(&escape_ctx, initial.settings_trigger.center())?;
        let _ = run_toolbar_frame(
            &escape_ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )?;
        assert!(!egui::Popup::is_id_open(&escape_ctx, settings_popup_id()));
        Ok(())
    }

    #[test]
    fn settings_fits_safe_content_at_312_points() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let initial = run_toolbar_frame(&ctx, Vec::new())?;
        let _ = click(&ctx, initial.settings_trigger.center())?;
        let _ = run_toolbar_frame(&ctx, Vec::new())?;
        let rect = popup_rect(&ctx, settings_popup_id())?;
        let allowed = ctx.content_rect();
        let expected_content = test_screen().shrink(4.0);

        assert!(
            (311.0..=313.0).contains(&rect.width()),
            "width was {}",
            rect.width()
        );
        assert_eq!(
            allowed, expected_content,
            "the test harness must expose the safe content rect used for popup placement"
        );
        assert!(
            allowed.contains_rect(rect),
            "popup {rect:?} escaped {allowed:?}"
        );
        Ok(())
    }

    #[test]
    fn modal_response_closes_on_a_backdrop_click() {
        let ctx = egui::Context::default();
        assert!(!run_modal_frame(&ctx, Vec::new()).should_close);
        assert!(!run_modal_frame(&ctx, Vec::new()).should_close);

        let backdrop = egui::pos2(4.0, 4.0);
        assert!(
            !run_modal_frame(
                &ctx,
                vec![
                    egui::Event::PointerMoved(backdrop),
                    pointer_button(backdrop, true),
                ],
            )
            .should_close
        );
        let release = run_modal_frame(
            &ctx,
            vec![
                egui::Event::PointerMoved(backdrop),
                pointer_button(backdrop, false),
            ],
        );
        assert!(
            release.backdrop_clicked,
            "the raw click should reach the modal backdrop"
        );
        assert!(release.should_close);
    }
}
