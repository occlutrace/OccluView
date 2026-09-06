//! Parent-popup wireframes of the language selector in every embedded locale.
//! Generated files live under `target/i18n-shots/` for human review only.

use super::*;

fn shot_screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 900.0))
}

fn settings_frame(
    ctx: &egui::Context,
    locale: &LocaleManager,
    events: Vec<egui::Event>,
) -> anyhow::Result<(egui::FullOutput, egui::Rect)> {
    let mut trigger_rect = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(shot_screen()),
            events,
            ..Default::default()
        },
        |ui| {
            egui::Panel::top("settings-language-shot-toolbar")
                .exact_size(30.0)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let trigger = show_settings_toolbar_toggle(ui, true, locale);
                        trigger_rect = Some(trigger.rect);
                        let _ = show_settings_popup(
                            &trigger,
                            &Settings::default(),
                            locale,
                            &UpdateCheckStatus::Idle,
                            None,
                            None,
                        );
                    });
                });
        },
    );
    output.textures_delta.clear();
    Ok((
        output,
        trigger_rect.ok_or_else(|| anyhow::anyhow!("settings toolbar trigger should render"))?,
    ))
}

fn click(
    ctx: &egui::Context,
    locale: &LocaleManager,
    position: egui::Pos2,
) -> anyhow::Result<egui::FullOutput> {
    let _ = settings_frame(
        ctx,
        locale,
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    )?;
    Ok(settings_frame(
        ctx,
        locale,
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    )?
    .0)
}

fn text_center(output: &egui::FullOutput, text: &str) -> anyhow::Result<egui::Pos2> {
    output
        .shapes
        .iter()
        .find_map(|shaped| match &shaped.shape {
            egui::epaint::Shape::Text(shape) if shape.galley.text() == text => {
                Some(shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("language selector header should render {text:?}"))
}

#[test]
fn language_selector_wireframes_for_visual_review() -> anyhow::Result<()> {
    use crate::i18n::catalog::EMBEDDED_TAGS;
    use crate::i18n::preference::UiLanguagePreference;

    for tag in EMBEDDED_TAGS {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut manager = LocaleManager::for_tests();
        if *tag != "en" {
            manager.set_preference(UiLanguagePreference::Explicit(tag));
        }

        let (_, trigger) = settings_frame(&ctx, &manager, Vec::new())?;
        let closed = click(&ctx, &manager, trigger.center())?;
        crate::i18n::shots::save_shot_with_texts(
            &format!("settings-language-closed-{tag}"),
            &ctx,
            closed,
            480,
            900,
        );

        let (visible, _) = settings_frame(&ctx, &manager, Vec::new())?;
        let header = format!(
            "{} · {}",
            manager.text("settings-language-label"),
            selected_language_summary(&manager)
        );
        let open = click(&ctx, &manager, text_center(&visible, &header)?)?;
        assert!(egui::Popup::is_id_open(&ctx, settings_popup_id()));
        crate::i18n::shots::save_shot_with_texts(
            &format!("settings-language-open-{tag}"),
            &ctx,
            open,
            480,
            900,
        );
    }
    Ok(())
}
