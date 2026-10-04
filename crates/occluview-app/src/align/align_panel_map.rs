//! The Heatmap block of the Align Scans window.
//!
//! Separate from the window itself because it answers a different question.
//! That module is about getting two scans onto each other; this one is about
//! reading how far apart they ended up.
//!
//! The working surface is limited to one toggle, one legend, and one absolute
//! deviation range. Fit diagnostics belong in logs, not in the window the
//! operator is using to place two scans.

use eframe::egui;

use crate::align::align_panel::AlignPanelAction;
use crate::align::align_worker::{AlignSettings, WORKING_MAX_MM, WORKING_SCALE_MIN_MM};
use crate::ui::icons::AppIcon;
use crate::{align::align_overlay, ui::ui_theme};

/// Show the Heatmap block; returns what the operator asked for.
pub(crate) fn show(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    fit_ready: bool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    // A persisted checkbox must not resurrect a map for a pose that was never
    // refined in this session. The application state owns this invariant, but
    // this presentation boundary also guards stale settings loaded from disk.
    if !fit_ready {
        settings.show_deviation = false;
    }
    let mut action = toggle(ui, settings, enabled && fit_ready, fit_ready, locale);
    if !fit_ready || !settings.show_deviation {
        return action;
    }

    action = action.or(range(ui, settings, enabled, locale));
    action
}

/// The one control that decides whether the map is on screen at all.
fn toggle(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
    fit_ready: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover())
            .0;
        crate::ui::icons::paint(
            ui.painter(),
            glyph,
            AppIcon::Heatmap,
            if settings.show_deviation {
                ui_theme::accent()
            } else {
                ui_theme::text_muted()
            },
        );
        let mut shown = settings.show_deviation;
        // The checkbox controls the deviation map; its label comes from the
        // `align-map-heatmap` catalog key.
        if ui
            .add_enabled(
                enabled,
                egui::Checkbox::new(
                    &mut shown,
                    locale
                        .tr(crate::i18n::message_id!("align-map-heatmap"))
                        .as_str(),
                ),
            )
            .on_hover_text(locale.tr(crate::i18n::message_id!("align-map-heatmap-hint")))
            .on_disabled_hover_text(if fit_ready {
                locale.tr(crate::i18n::message_id!("align-job-measure"))
            } else {
                locale.tr(crate::i18n::message_id!("align-map-requires-refine"))
            })
            .changed()
        {
            settings.show_deviation = shown;
            action = Some(if shown {
                AlignPanelAction::Measure
            } else {
                AlignPanelAction::HideMap
            });
        }
    });
    action
}

/// Set the cool and hot absolute-deviation limits of the same colour bar.
fn range(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let (min_mm, scale_mm) = settings.display_limits();
    settings.scale_mm = scale_mm.max(0.001);
    settings.auto_scale = false;
    settings.min_display_mm = min_mm.min(settings.scale_mm - 0.001);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(locale.tr(crate::i18n::message_id!("align-map-min")));
        let minimum = ui.add_enabled(
            enabled,
            egui::DragValue::new(&mut settings.min_display_mm)
                .range(WORKING_SCALE_MIN_MM..=settings.scale_mm - 0.001)
                .speed(0.005)
                .fixed_decimals(3)
                .suffix(" mm"),
        );
        crate::ui::accessibility::spin_button(
            &minimum,
            &locale.tr(crate::i18n::message_id!("align-map-min")),
            enabled,
        );
        changed |= minimum.changed();
    });
    align_overlay::paint_legend(ui, *settings, locale);
    ui.horizontal(|ui| {
        ui.label(locale.tr(crate::i18n::message_id!("align-map-max")));
        let maximum = ui.add_enabled(
            enabled,
            egui::DragValue::new(&mut settings.scale_mm)
                .range(settings.min_display_mm + 0.001..=WORKING_MAX_MM)
                .speed(0.005)
                .fixed_decimals(3)
                .suffix(" mm"),
        );
        crate::ui::accessibility::spin_button(
            &maximum,
            &locale.tr(crate::i18n::message_id!("align-map-max")),
            enabled,
        );
        changed |= maximum.changed();
    });
    changed.then_some(AlignPanelAction::Measure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonfinite_heatmap_limits_cannot_panic_or_reach_the_range_widgets() {
        for (scale_mm, min_display_mm) in [
            (f64::NAN, 0.0),
            (0.05, f64::NAN),
            (f64::INFINITY, f64::NEG_INFINITY),
            (f64::NEG_INFINITY, f64::INFINITY),
        ] {
            let ctx = egui::Context::default();
            let mut settings = AlignSettings {
                scale_mm,
                min_display_mm,
                ..AlignSettings::default()
            };
            let _ = heatmap_frame(&ctx, &mut settings, true, Vec::new());
            assert!(settings.scale_mm.is_finite());
            assert!(settings.min_display_mm.is_finite());
            assert!((0.001..=WORKING_MAX_MM).contains(&settings.scale_mm));
            assert!((0.0..=settings.scale_mm - 0.001).contains(&settings.min_display_mm));
        }
    }

    fn heatmap_frame(
        ctx: &egui::Context,
        settings: &mut AlignSettings,
        ready: bool,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let _ = show(
                    ui,
                    settings,
                    ready,
                    false,
                    &crate::i18n::LocaleManager::for_tests(),
                );
            },
        );
        output.textures_delta.clear();
        output
    }

    #[test]
    fn disabled_heatmap_explains_missing_refinement_or_busy_measurement() {
        for (ready, expected) in [(false, "Run Best fit matching first"), (true, "Measuring…")] {
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| {
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
            });
            let mut settings = AlignSettings::default();
            let output = heatmap_frame(&ctx, &mut settings, ready, Vec::new());
            let position = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Heatmap" => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                _ => None,
            });
            assert!(position.is_some(), "the heatmap toggle must be rendered");
            let Some(pos) = position else { return };
            let _ = heatmap_frame(
                &ctx,
                &mut settings,
                ready,
                vec![egui::Event::PointerMoved(pos)],
            );
            let output = heatmap_frame(&ctx, &mut settings, ready, Vec::new());
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape,
                    egui::Shape::Text(text) if text.galley.text().contains(expected)
                )),
                "disabled heatmap must explain its current state: {expected}",
            );
        }
    }
}
