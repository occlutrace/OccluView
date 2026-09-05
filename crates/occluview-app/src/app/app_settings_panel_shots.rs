//! Wireframe screenshots of the language selector (closed + open
//! dropdown) in every embedded locale for human visual review
//! (`target/i18n-shots/`, never committed). Rendered standalone at popup
//! width: no scrolling involved. Child test module of the settings panel;
//! split out to respect the 800-line source budget.

/// Wireframe screenshots of the language selector (closed + open
/// dropdown) in every embedded locale for human visual review
/// (`target/i18n-shots/`, never committed). Rendered standalone at
/// popup width: no scrolling involved.
#[test]
fn language_selector_wireframes_for_visual_review() -> anyhow::Result<()> {
    use super::language_section;
    use crate::i18n::catalog::EMBEDDED_TAGS;
    use crate::i18n::preference::UiLanguagePreference;

    for tag in EMBEDDED_TAGS {
        let ctx = egui::Context::default();
        let mut manager = crate::i18n::LocaleManager::for_tests();
        if *tag != "en" {
            manager.set_preference(UiLanguagePreference::Explicit(tag));
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let frame_input = || egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        // Closed state.
        let mut output = ctx.run_ui(frame_input(), |ui| {
            ui.set_width(286.0);
            let mut action = None;
            language_section(ui, &manager, &mut action);
        });
        output.textures_delta.clear();
        crate::i18n::shots::save_shot_with_texts(
            &format!("language-closed-{tag}"),
            &ctx,
            output,
            400,
            300,
        );
        // Locate the closed ComboBox by its selected label, click it.
        let mut output = ctx.run_ui(frame_input(), |ui| {
            ui.set_width(286.0);
            let mut action = None;
            language_section(ui, &manager, &mut action);
        });
        output.textures_delta.clear();
        let selected = match &manager.snapshot().preference {
            UiLanguagePreference::Auto => manager.text("settings-language-auto"),
            UiLanguagePreference::Explicit(option) => crate::i18n::endonym(option).to_owned(),
        };
        let combo = output
            .shapes
            .iter()
            .find_map(|shaped| match &shaped.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == selected => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("combo not found for {tag}"))?;
        let click_at = |position, pressed| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(286.0);
                    let mut action = None;
                    language_section(ui, &manager, &mut action);
                },
            );
            output.textures_delta.clear();
            output
        };
        let _ = click_at(combo, true);
        let mut output = click_at(combo, false);
        output.textures_delta.clear();
        // The release frame flips the popup state; the popup Area
        // itself lays out on the following frames.
        let mut output = ctx.run_ui(frame_input(), |ui| {
            ui.set_width(286.0);
            let mut action = None;
            language_section(ui, &manager, &mut action);
        });
        output.textures_delta.clear();
        crate::i18n::shots::save_shot_with_texts(
            &format!("language-open-{tag}"),
            &ctx,
            output,
            400,
            300,
        );
    }
    Ok(())
}
