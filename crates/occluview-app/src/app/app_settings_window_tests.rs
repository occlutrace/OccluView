use super::*;
use crate::app::app_dialogs::{recent_files_popup_id, show_recent_files_popup};
use crate::app::app_settings_panel::show_settings_toolbar_toggle;
use crate::app_settings::Settings;
use crate::i18n::os::OsLocaleSource;
use crate::i18n::preference::UiLanguagePreference;
use crate::recent_files::RecentFiles;
use crate::update_notice::UpdateCheckStatus;

struct FixedLocales(&'static [&'static str]);

impl OsLocaleSource for FixedLocales {
    fn preferred_languages(&self) -> Vec<String> {
        self.0.iter().map(|tag| (*tag).to_owned()).collect()
    }
}

fn locale_from_system_languages(tags: &'static [&'static str]) -> crate::i18n::LocaleManager {
    crate::i18n::LocaleManager::startup(None, &FixedLocales(tags)).0
}

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
    let locale = crate::i18n::LocaleManager::for_tests();
    run_toolbar_frame_in(ctx, events, &locale)
}

fn run_toolbar_frame_in(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    locale: &crate::i18n::LocaleManager,
) -> anyhow::Result<ToolbarFrame> {
    run_toolbar_frame_at(ctx, events, locale, test_screen())
}

/// The same frame with the settings the test wants to read.
fn run_toolbar_frame_at(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    locale: &crate::i18n::LocaleManager,
    screen: egui::Rect,
) -> anyhow::Result<ToolbarFrame> {
    run_toolbar_frame_at_with_settings(ctx, events, locale, screen, &Settings::default())
}

fn run_toolbar_frame_at_with_settings(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    locale: &crate::i18n::LocaleManager,
    screen: egui::Rect,
    settings: &Settings,
) -> anyhow::Result<ToolbarFrame> {
    let input = egui::RawInput {
        screen_rect: Some(screen),
        safe_area_insets: Some(egui::SafeAreaInsets(egui::Margin::same(4).into())),
        events,
        ..Default::default()
    };
    let mut action = None;
    let mut settings_trigger = None;
    let mut recent_trigger = None;
    let settings_for_frame = settings.clone();
    let mut recent = RecentFiles::new(1);
    recent.push("case.stl");
    let mut output = ctx.run_ui(input, |ui| {
        egui::Panel::top("settings-test-toolbar")
            .exact_size(30.0)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let settings = show_settings_toolbar_toggle(ui, true, locale);
                    settings_trigger = Some(settings.rect);
                    action = show_settings_popup(
                        &settings,
                        &settings_for_frame,
                        locale,
                        &UpdateCheckStatus::Idle,
                        None,
                        None,
                    );

                    let recent_trigger_response =
                        ui.add(egui::Button::new("Recent").min_size(egui::vec2(64.0, 22.0)));
                    recent_trigger = Some(recent_trigger_response.rect);
                    let _ = show_recent_files_popup(&recent_trigger_response, &recent, locale);
                });
            });
    });
    output.textures_delta.clear();
    Ok(ToolbarFrame {
        action,
        settings_trigger: settings_trigger
            .ok_or_else(|| anyhow::anyhow!("the toolbar-like Settings trigger should render"))?,
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

fn tall_test_screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 1_200.0))
}

fn run_tall_toolbar_frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> anyhow::Result<ToolbarFrame> {
    let locale = crate::i18n::LocaleManager::for_tests();
    run_tall_toolbar_frame_in(ctx, events, &locale)
}

fn run_tall_toolbar_frame_in(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    locale: &crate::i18n::LocaleManager,
) -> anyhow::Result<ToolbarFrame> {
    run_toolbar_frame_at(ctx, events, locale, tall_test_screen())
}

fn click_tall(ctx: &egui::Context, position: egui::Pos2) -> anyhow::Result<ToolbarFrame> {
    let locale = crate::i18n::LocaleManager::for_tests();
    click_tall_in(ctx, &locale, position)
}

fn click_tall_in(
    ctx: &egui::Context,
    locale: &crate::i18n::LocaleManager,
    position: egui::Pos2,
) -> anyhow::Result<ToolbarFrame> {
    let _ = run_tall_toolbar_frame_in(
        ctx,
        vec![
            egui::Event::PointerMoved(position),
            pointer_button(position, true),
        ],
        locale,
    )?;
    run_tall_toolbar_frame_in(
        ctx,
        vec![
            egui::Event::PointerMoved(position),
            pointer_button(position, false),
        ],
        locale,
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

fn direct_control_center_containing(
    output: &egui::FullOutput,
    required_fragments: &[&str],
) -> anyhow::Result<egui::Pos2> {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::epaint::Shape::Text(text)
                if required_fragments
                    .iter()
                    .all(|fragment| text.galley.text().contains(fragment)) =>
            {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        })
        .ok_or_else(|| {
            let rendered_text = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            anyhow::anyhow!(
                "the production Settings popup should render fragments {required_fragments:?}; rendered text: {rendered_text:?}"
            )
        })
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
        let response =
            egui::Modal::new(egui::Id::new("about-modal-close-contract")).show(ui.ctx(), |ui| {
                ui.set_min_size(egui::vec2(160.0, 96.0));
            });
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

/// There is no save-format question any more. The format follows the scan, so
/// the panel states the rule and offers neither a mode switch nor format chips;
/// a chip labelled "STL" was what let an operator pick a colourless format for a
/// colour scan.
#[test]
fn settings_offer_no_save_format_choice() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    let initial = run_toolbar_frame(&ctx, Vec::new())?;
    let _ = click(&ctx, initial.settings_trigger.center())?;

    let panel = run_toolbar_frame(&ctx, Vec::new())?;
    for removed in [
        "Its own format",
        "Chosen format",
        "Fallback export format",
        "When saving a scan",
    ] {
        assert!(
            direct_control_center(&panel.output, removed).is_err(),
            "{removed:?} must not be offered: the format is decided from the scan"
        );
    }
    // The rule itself is still stated, so the operator knows what will happen.
    assert!(
        direct_control_center(&panel.output, "Save format").is_ok(),
        "the panel must still say what Save layer writes"
    );
    Ok(())
}

#[test]
fn language_selector_opens_inside_settings_and_keeps_parent_open() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let initial = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let _ = click_tall(&ctx, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let selector =
        direct_control_center_containing(&visible.output, &["Language", "System language"])?;

    let _opened = click_tall(&ctx, selector)?;
    assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
    let expanded = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let german = direct_control_center(&expanded.output, "Deutsch")?;
    let selected = click_tall(&ctx, german)?;

    assert_eq!(
        selected.action,
        Some(SettingsAction::SetExplicitLanguage("de"))
    );
    assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
    let collapsed = run_tall_toolbar_frame(&ctx, Vec::new())?;
    assert!(direct_control_center(&collapsed.output, "Deutsch").is_err());
    Ok(())
}

#[test]
fn system_language_item_reapplies_even_when_already_selected() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let initial = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let _ = click_tall(&ctx, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let selector =
        direct_control_center_containing(&visible.output, &["Language", "System language"])?;

    let _ = click_tall(&ctx, selector)?;
    let expanded = run_tall_toolbar_frame(&ctx, Vec::new())?;
    let system =
        direct_control_center_containing(&expanded.output, &["System language", "English"])?;
    let selected = click_tall(&ctx, system)?;

    assert_eq!(selected.action, Some(SettingsAction::UseSystemLanguage));
    assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
    Ok(())
}

#[test]
fn language_selector_keeps_its_open_state_across_a_catalog_switch() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let english = crate::i18n::LocaleManager::for_tests();
    let mut russian = crate::i18n::LocaleManager::for_tests();
    russian.set_preference(UiLanguagePreference::Explicit("ru"));

    let initial = run_tall_toolbar_frame_in(&ctx, Vec::new(), &english)?;
    let _ = click_tall_in(&ctx, &english, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame_in(&ctx, Vec::new(), &english)?;
    let selector =
        direct_control_center_containing(&visible.output, &["Language", "System language"])?;
    let _ = click_tall_in(&ctx, &english, selector)?;

    let localized = run_tall_toolbar_frame_in(&ctx, Vec::new(), &russian)?;
    assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
    assert!(direct_control_center(&localized.output, "Deutsch").is_ok());
    Ok(())
}

#[test]
fn system_language_fallback_is_visible_for_an_unavailable_system_catalog() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let manager = locale_from_system_languages(&["ja-JP"]);
    let expected_notice = manager.tr_with("settings-language-catalog-fallback", &[("tag", "ja")]);

    let initial = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let _ = click_tall_in(&ctx, &manager, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let selector =
        direct_control_center_containing(&visible.output, &["Language", "System language"])?;
    let _ = click_tall_in(&ctx, &manager, selector)?;
    let expanded = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;

    assert!(direct_control_center(&expanded.output, &expected_notice).is_ok());
    Ok(())
}

#[test]
fn explicit_language_hides_an_unrelated_system_fallback() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let mut manager = locale_from_system_languages(&["ja-JP"]);
    manager.set_preference(UiLanguagePreference::Explicit("de"));
    let unrelated_notice = manager.tr_with("settings-language-catalog-fallback", &[("tag", "ja")]);

    let initial = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let _ = click_tall_in(&ctx, &manager, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let selector = direct_control_center_containing(&visible.output, &["Sprache", "Deutsch"])?;
    let _ = click_tall_in(&ctx, &manager, selector)?;
    let expanded = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;

    assert!(direct_control_center(&expanded.output, &unrelated_notice).is_err());
    Ok(())
}

#[test]
fn explicit_language_fallback_is_visible_for_an_unavailable_catalog() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let mut manager = locale_from_system_languages(&["de-DE"]);
    manager.set_preference(UiLanguagePreference::Explicit("ja"));
    let expected_notice = manager.tr_with("settings-language-catalog-fallback", &[("tag", "ja")]);

    let initial = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let _ = click_tall_in(&ctx, &manager, initial.settings_trigger.center())?;
    let visible = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
    let selector = direct_control_center_containing(&visible.output, &["Language", "English"])?;
    let _ = click_tall_in(&ctx, &manager, selector)?;
    let expanded = run_tall_toolbar_frame_in(&ctx, Vec::new(), &manager)?;

    assert!(direct_control_center(&expanded.output, &expected_notice).is_ok());
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

/// Saves settings wireframes for human review.
#[test]
fn settings_popup_wireframes_for_visual_review() -> anyhow::Result<()> {
    use crate::i18n::catalog::EMBEDDED_TAGS;
    use crate::i18n::preference::UiLanguagePreference;

    for tag in EMBEDDED_TAGS {
        let ctx = egui::Context::default();
        let mut manager = crate::i18n::LocaleManager::for_tests();
        if *tag != "en" {
            manager.set_preference(UiLanguagePreference::Explicit(tag));
        }
        let initial = run_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
        let center = initial.settings_trigger.center();
        let press = |position| {
            run_toolbar_frame_in(
                &ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    pointer_button(position, true),
                ],
                &manager,
            )
        };
        let release = |position| {
            run_toolbar_frame_in(
                &ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    pointer_button(position, false),
                ],
                &manager,
            )
        };
        let _ = press(center)?;
        let _ = release(center)?;
        // Settle popup layout.
        let _ = run_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
        let frame = run_toolbar_frame_in(&ctx, Vec::new(), &manager)?;
        let path = crate::i18n::shots::save_shot_with_texts(
            &format!("settings-{tag}"),
            &ctx,
            frame.output,
            500,
            384,
        );
        assert!(path.is_file(), "shot missing: {}", path.display());
    }
    Ok(())
}
