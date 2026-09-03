//! Preferences dispatch and the product-information modals. The popover
//! surface itself (sections, rows, action vocabulary) lives in
//! `app_settings_panel`.

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
            SettingsAction::SetFrameSceneOnOpen(enabled) => {
                self.settings.frame_scene_on_open = enabled;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetDoubleClickFocus(enabled) => {
                self.settings.double_click_resets_camera = enabled;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetOrbitSensitivity(value) => {
                self.settings.orbit_sensitivity = value;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetZoomSensitivity(value) => {
                self.settings.zoom_sensitivity = value;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetRecentFilesLimit(limit) => {
                self.settings.recent_files_limit = limit;
                // Re-trim the live list so an operator shrinking the limit sees
                // the chevron shorten immediately, not after a restart.
                let stored = self.recent_files.serialize();
                self.recent_files = crate::recent_files::RecentFiles::deserialize(
                    self.settings.recent_files_limit(),
                    &stored,
                );
                self.settings_persistence.mark_dirty();
                self.save_recent_files();
            }
            SettingsAction::SetViewportBackground(background) => {
                self.settings.viewport_background = background;
                // The clear color is baked into the prepared scene specs, so
                // both render paths must rebuild before the change is visible.
                self.mark_scene_materials_changed();
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetShowCutGhost(enabled) => {
                self.settings.show_cut_ghost = enabled;
                // Both render paths bake the ghost decision into the frame they
                // draw; force the next one so a stationary cut view answers at
                // once instead of waiting for the next camera move.
                self.needs_render = true;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetUnitDisplay(unit) => {
                self.settings.unit_display = unit;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetTheme(theme) => {
                self.settings.theme = theme;
                self.settings_persistence.mark_dirty();
            }
            SettingsAction::SetUiScale { value, commit } => {
                self.settings.ui_scale = value;
                if commit {
                    self.settings_persistence.mark_dirty();
                }
            }
            SettingsAction::SetRememberSculptBrush(enabled) => {
                self.settings.remember_sculpt_brush = enabled;
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
                        egui::RichText::new("OccluView")
                            .size(19.0)
                            .strong()
                            .color(ui_theme::text()),
                    );
                    ui.label(
                        egui::RichText::new("Mesh Repair · Mesh Editing for dental CAD")
                            .size(12.0)
                            .color(ui_theme::text_weak()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .color(ui_theme::text_muted()),
                    );
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                centered_about_row(ui, ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP, |ui| {
                    ui.spacing_mut().item_spacing.x = ABOUT_ACTION_GAP;
                    if about_link(ui, ABOUT_ACTION_WIDTH, AppIcon::Globe, "Website") {
                        open_url = Some("https://occlutrace.ai");
                    }
                    if about_link(ui, ABOUT_ACTION_WIDTH, AppIcon::Github, "Source") {
                        open_url = Some("https://github.com/occlutrace/OccluView");
                    }
                });
                ui.add_space(2.0);
                centered_about_row(ui, ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP, |ui| {
                    if about_link(
                        ui,
                        ABOUT_ACTION_WIDTH * 2.0 + ABOUT_ACTION_GAP,
                        AppIcon::Licenses,
                        "Third-party licenses",
                    ) {
                        open_third_party = true;
                    }
                });
                ui.add_space(2.0);
                centered_about_row(ui, ABOUT_FOOTER_WIDTH, |ui| {
                    ui.label(
                        egui::RichText::new("Apache License 2.0")
                            .size(10.5)
                            .color(ui_theme::text_muted()),
                    );
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            },
        );

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

const ABOUT_ACTION_WIDTH: f32 = 132.0;
const ABOUT_ACTION_GAP: f32 = 6.0;
const ABOUT_FOOTER_WIDTH: f32 = 146.0;
const ABOUT_ACTION_ROW_HEIGHT: f32 = 27.0;

/// Center a compact row without `horizontal_centered`: that layout fills the
/// available height by design, which made a vertically stacked modal grow on
/// every repaint. The row itself stays top-aligned and only receives the
/// horizontal gutter it needs.
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
mod tests {
    use super::*;
    use crate::app::app_dialogs::{
        recent_files_popup_id, show_recent_files_popup, show_settings_toolbar_toggle,
    };
    use crate::app_settings::Settings;
    use crate::recent_files::RecentFiles;
    use crate::update_notice::UpdateCheckStatus;

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

    fn responsive_information_modal_frame(
        ctx: &egui::Context,
        screen: egui::Rect,
    ) -> anyhow::Result<egui::Rect> {
        let id = egui::Id::new("information-modal-resize-contract");
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                show_information_modal(ui.ctx(), id, egui::vec2(560.0, 420.0), |ui| {
                    ui.set_width(304.0_f32.min(ui.available_width()));
                    ui.set_min_height(180.0_f32.min(ui.available_height()));
                });
            },
        )
        .drop_without_applying_deltas();
        popup_rect(ctx, id)
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
            Some(SettingsAction::SetExportFormat(
                crate::app_settings::FallbackExportFormat::Stl
            ))
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

    #[test]
    fn information_modal_shrinks_to_the_current_content_rect_after_resize() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let large_screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let small_screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 180.0));

        let _ = responsive_information_modal_frame(&ctx, large_screen)?;
        let _ = responsive_information_modal_frame(&ctx, large_screen)?;
        let _ = responsive_information_modal_frame(&ctx, small_screen)?;
        let rect = responsive_information_modal_frame(&ctx, small_screen)?;
        let bounds = small_screen.shrink(16.0);

        assert!(
            bounds.contains_rect(rect),
            "information modal {rect:?} escaped the current content bounds {bounds:?}"
        );
        Ok(())
    }

    #[test]
    fn scrollable_information_modal_stays_near_its_declared_size() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let id = egui::Id::new("information-modal-scroll-size-contract");

        for _ in 0..2 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    show_information_modal(ui.ctx(), id, egui::vec2(560.0, 420.0), |ui| {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show_rows(ui, 14.0, 2_000, |ui, rows| {
                                for row in rows {
                                    ui.label(format!("license line {row}"));
                                }
                            });
                    });
                },
            )
            .drop_without_applying_deltas();
        }

        let rect = popup_rect(&ctx, id)?;
        assert!(
            rect.width() <= 600.0 && rect.height() <= 460.0,
            "scrollable information modal should not expand to the full screen: {rect:?}"
        );
        Ok(())
    }

    #[test]
    fn about_modal_does_not_cycle_through_repeated_sizing_passes() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
        let id = egui::Id::new("information-modal-about-stability-contract");
        let mut rects = Vec::new();

        for _ in 0..12 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    show_information_modal(ui.ctx(), id, egui::vec2(320.0, 240.0), |ui| {
                        ui.set_width(304.0_f32.min(ui.available_width()));
                        ui.vertical_centered(|ui| {
                            ui.label("OccluView");
                            ui.label("Mesh Repair · Mesh Editing for dental CAD");
                            ui.label("Version 1.1.1");
                        });
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);
                        centered_about_row(ui, 270.0, |ui| {
                            ui.label("Website");
                            ui.label("Source");
                        });
                        ui.add_space(2.0);
                        centered_about_row(ui, 270.0, |ui| {
                            ui.label("Third-party licenses");
                        });
                        ui.add_space(2.0);
                        centered_about_row(ui, 146.0, |ui| {
                            ui.label("Apache License 2.0");
                        });
                    });
                },
            )
            .drop_without_applying_deltas();
            rects.push(popup_rect(&ctx, id)?);
        }

        let stable_tail = &rects[6..];
        assert!(
            stable_tail.windows(2).all(|pair| {
                (pair[0].size() - pair[1].size()).length() < 0.1
                    && (pair[0].center() - pair[1].center()).length() < 0.1
            }),
            "About modal kept changing size/position: {stable_tail:?}"
        );
        Ok(())
    }
}
