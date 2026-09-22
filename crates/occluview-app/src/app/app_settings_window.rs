//! Settings action dispatch and product-information modals.

use super::app_settings_panel::{settings_popup_id, show_settings_popup, SettingsAction};
use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use crate::icons::AppIcon;
use crate::ui_theme;
use eframe::egui;

pub(super) use crate::modal_surface::show_information_modal;

impl OccluViewApp {
    pub(super) fn show_settings_popup(&mut self, trigger: &egui::Response) {
        let Some(action) = show_settings_popup(
            trigger,
            &self.persistence.settings,
            &self.ui.locale,
            self.persistence.update_notice.check_status(),
            self.persistence.settings_persistence.error(),
            self.persistence.language_persistence.error(),
        ) else {
            return;
        };

        match action {
            SettingsAction::SetExportFormat(format) => {
                self.persistence.settings.fallback_export_format = format;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetKeepSourceExportFormat(keep) => {
                self.persistence.settings.keep_source_export_format = keep;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetRememberExportDir(remember) => {
                self.persistence
                    .settings
                    .set_remember_export_dir(remember, self.persistence.last_export_dir.as_deref());
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetUpdateCheckOnStart(enabled) => {
                self.persistence.settings.update_check_on_start = enabled;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetFrameSceneOnOpen(enabled) => {
                self.persistence.settings.frame_scene_on_open = enabled;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetDoubleClickFocus(enabled) => {
                self.persistence.settings.double_click_resets_camera = enabled;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetOrbitSensitivity(value) => {
                self.persistence.settings.orbit_sensitivity = value;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetZoomSensitivity(value) => {
                self.persistence.settings.zoom_sensitivity = value;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetViewportBackground(background) => {
                self.persistence.settings.viewport_background = background;
                // Prepared scenes cache the clear color on both paths.
                self.mark_scene_materials_changed();
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetShowCutGhost(enabled) => {
                self.persistence.settings.show_cut_ghost = enabled;
                // Both render paths cache the ghost decision.
                self.render.invalidation.overlay_tools_changed();
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetUnitDisplay(unit) => {
                self.persistence.settings.unit_display = unit;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetTheme(theme) => {
                self.persistence.settings.theme = theme;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::SetUiScale { value, commit } => {
                self.persistence.settings.ui_scale = value;
                if commit {
                    self.persistence.settings_persistence.mark_dirty();
                }
            }
            SettingsAction::SetRememberSculptBrush(enabled) => {
                self.persistence.settings.remember_sculpt_brush = enabled;
                self.persistence.settings_persistence.mark_dirty();
            }
            SettingsAction::CheckForUpdates => {
                self.persistence.update_notice.request_check(&trigger.ctx);
            }
            SettingsAction::SetExplicitLanguage(tag) => {
                self.ui
                    .locale
                    .set_preference(crate::i18n::preference::UiLanguagePreference::Explicit(tag));
                self.language_selection_changed();
            }
            SettingsAction::UseSystemLanguage => {
                self.ui
                    .locale
                    .use_system_language(&crate::i18n::os::SystemLocaleSource);
                self.language_selection_changed();
            }
            SettingsAction::OpenAbout => {
                egui::Popup::close_id(&trigger.ctx, settings_popup_id());
                self.ui.information_dialog = InformationDialog::About;
            }
            SettingsAction::OpenShortcuts => {
                egui::Popup::close_id(&trigger.ctx, settings_popup_id());
                self.ui.information_dialog = InformationDialog::KeyboardMouse;
            }
        }
    }

    fn language_selection_changed(&mut self) {
        self.persistence.language_persistence.mark_dirty();
        // Send the localized native title on the next frame.
        self.ui.native_title_sent = false;
    }

    pub(super) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        if self.ui.information_dialog != InformationDialog::About {
            return;
        }

        let mut close = false;
        let mut open_third_party = false;
        let mut open_url = None;
        let logo = self.app_logo_texture(ctx).cloned();

        let modal_response = show_information_modal(
            ctx,
            egui::Id::new("occluview-about-dialog-v2"),
            egui::vec2(320.0, 240.0),
            |ui| {
                ui.set_width(304.0_f32.min(ui.available_width()));
                ui.vertical_centered(|ui| {
                    if let Some(logo) = &logo {
                        ui.add(egui::Image::new((logo.id(), egui::vec2(48.0, 48.0))));
                    }
                    ui.label(
                        egui::RichText::new(self.ui.locale.text("about-title"))
                            .size(19.0)
                            .strong()
                            .color(ui_theme::text()),
                    );
                    ui.label(
                        egui::RichText::new(self.ui.locale.text("about-tagline"))
                            .size(12.0)
                            .color(ui_theme::text_weak()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(self.ui.locale.text_with(
                            "about-version",
                            Some(&crate::i18n::catalog::args(&[(
                                "version",
                                env!("CARGO_PKG_VERSION"),
                            )])),
                        ))
                        .size(11.0)
                        .color(ui_theme::text_muted()),
                    );
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                // These labels are pinned by the UI contract tests.
                centered_about_row(ui, ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP, |ui| {
                    ui.spacing_mut().item_spacing.x = ABOUT_ACTION_GAP;
                    if about_link(
                        ui,
                        ABOUT_ACTION_WIDTH,
                        AppIcon::Globe,
                        &self.ui.locale.tr("about-website"),
                    ) {
                        open_url = Some("https://occlutrace.ai");
                    }
                    if about_link(
                        ui,
                        ABOUT_ACTION_WIDTH,
                        AppIcon::Github,
                        &self.ui.locale.tr("about-source"),
                    ) {
                        open_url = Some("https://github.com/occlutrace/OccluView");
                    }
                });
                ui.add_space(2.0);
                centered_about_row(ui, ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP, |ui| {
                    if about_link(
                        ui,
                        ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP,
                        AppIcon::Licenses,
                        &self.ui.locale.tr("about-licenses"),
                    ) {
                        open_third_party = true;
                    }
                });
                ui.add_space(2.0);
                centered_about_row(ui, ABOUT_FOOTER_WIDTH, |ui| {
                    ui.label(
                        egui::RichText::new(self.ui.locale.tr("about-license-kind"))
                            .size(10.5)
                            .color(ui_theme::text_muted()),
                    );
                    ui.add_space(10.0);
                    if ui.button(self.ui.locale.tr("help-close")).clicked() {
                        close = true;
                    }
                });
            },
        );

        if let Some(url) = open_url {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        if open_third_party {
            self.ui.information_dialog = InformationDialog::ThirdPartyNotices;
        } else if close || modal_response.should_close() {
            self.ui.information_dialog = InformationDialog::None;
        }
    }
}

const ABOUT_ACTION_WIDTH: f32 = 132.0;
const ABOUT_ACTION_GAP: f32 = 6.0;
const ABOUT_FOOTER_WIDTH: f32 = 146.0;
const ABOUT_ACTION_ROW_HEIGHT: f32 = 27.0;

/// Centers a fixed-width row without expanding the modal vertically.
fn centered_about_row(
    ui: &mut egui::Ui,
    content_width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ABOUT_ACTION_ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let gutter = ((ui.available_width() - content_width) * 0.5).max(0.0);
            ui.add_space(gutter);
            add_contents(ui);
        },
    );
}

fn about_link(ui: &mut egui::Ui, width: f32, icon: AppIcon, label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 27.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui_theme::RADIUS_CONTROL,
            ui_theme::accent().gamma_multiply(0.08),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            ui_theme::RADIUS_CONTROL,
            egui::Stroke::new(1.0_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    let ink = ui_theme::text();
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let icon_side = 14.0;
    let content_width = icon_side + 7.0 + galley.size().x;
    let content_left = rect.center().x - content_width * 0.5;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(content_left + icon_side * 0.5, rect.center().y),
        egui::Vec2::splat(icon_side),
    );
    crate::icons::paint(ui.painter(), icon_rect, icon, ui_theme::text_weak());
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + 7.0,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        ink,
    );
    response.clicked()
}

#[cfg(test)]
#[path = "app_settings_window_tests.rs"]
mod tests;
