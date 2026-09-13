//! The Heatmap block of the Align Scans window.
//!
//! Split from the window itself because it answers a different question. That
//! module is about getting two scans onto each other; this one is about reading
//! how far apart they ended up, and it is the part an operator stares at.
//!
//! The working surface is deliberately small: one toggle, one legend, and one
//! absolute deviation range. Fit diagnostics belong in logs, not in the window
//! the operator is using to place two scans.

use eframe::egui;

use crate::align_panel::AlignPanelAction;
use crate::align_worker::{AlignSettings, WORKING_MAX_MM, WORKING_SCALE_MIN_MM};
use crate::icons::AppIcon;
use crate::{align_overlay, ui_theme};

/// Show the Heatmap block; returns what the operator asked for.
pub(crate) fn show(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    refined_match_ready: bool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    // A persisted checkbox must not resurrect a map for a pose that was never
    // refined in this session. The application state owns this invariant, but
    // this presentation boundary also guards stale settings loaded from disk.
    if !refined_match_ready {
        settings.show_deviation = false;
    }
    let mut action = toggle(ui, settings, enabled && refined_match_ready, locale);
    if !refined_match_ready || !settings.show_deviation {
        return action;
    }

    settings.scale_mm = settings.scale_mm.clamp(0.001, WORKING_MAX_MM);
    action = action.or(range(ui, settings, enabled, locale));
    action
}

/// The one control that decides whether the map is on screen at all.
fn toggle(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover())
            .0;
        crate::icons::paint(
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
        // Toggle label "Heatmap" (spelling pinned by the test below).
        if ui
            .add_enabled(
                enabled,
                egui::Checkbox::new(&mut shown, locale.tr("align-map-heatmap").as_str()),
            )
            .on_hover_text(if enabled {
                locale.tr("align-map-heatmap-hint")
            } else {
                locale.tr("align-map-requires-refine")
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
    settings.scale_mm = settings.scale_mm.clamp(0.001, WORKING_MAX_MM);
    settings.auto_scale = false;
    settings.min_display_mm = settings
        .min_display_mm
        .clamp(WORKING_SCALE_MIN_MM, settings.scale_mm - 0.001);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(locale.tr("align-map-min"));
        let minimum = ui.add_enabled(
            enabled,
            egui::DragValue::new(&mut settings.min_display_mm)
                .range(WORKING_SCALE_MIN_MM..=settings.scale_mm - 0.001)
                .speed(0.005)
                .fixed_decimals(3)
                .suffix(" mm"),
        );
        changed |= minimum.changed();
    });
    align_overlay::paint_legend(ui, *settings, locale);
    ui.horizontal(|ui| {
        ui.label(locale.tr("align-map-max"));
        let maximum = ui.add_enabled(
            enabled,
            egui::DragValue::new(&mut settings.scale_mm)
                .range(settings.min_display_mm + 0.001..=WORKING_MAX_MM)
                .speed(0.005)
                .fixed_decimals(3)
                .suffix(" mm"),
        );
        changed |= maximum.changed();
    });
    changed.then_some(AlignPanelAction::Measure)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = crate::primary_ui_tests::production_source(include_str!("align_panel_map.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    #[test]
    fn the_map_is_called_what_the_operator_calls_it() {
        let source = production();
        assert!(source.contains("align-map-heatmap"));
        assert!(
            !source.contains("Hitmap") && !source.contains("hitmap"),
            "the misspelling is back in the interface"
        );
    }

    #[test]
    fn the_working_panel_has_no_persistent_diagnostic_wall() {
        let source = production();
        for diagnostic in [
            "numbers(ui",
            "saturation(ui",
            "grey_note(ui",
            "align-map-within",
            "align-map-rms",
            "align-map-grey-",
            "align-map-advice-",
        ] {
            assert!(
                !source.contains(diagnostic),
                "working heatmap panel still renders removed diagnostic {diagnostic}"
            );
        }
    }

    #[test]
    fn the_working_panel_has_editable_cool_and_hot_limits() {
        let source = production();
        assert!(source.contains("range(ui, settings, enabled, locale)"));
        assert!(source.contains("align-map-min"));
        assert!(source.contains("align-map-max"));
        assert!(source.contains("egui::DragValue::new(&mut settings.min_display_mm)"));
        assert!(source.contains("egui::DragValue::new(&mut settings.scale_mm)"));
        assert!(
            !source.contains("CLINICAL_CEILING_MM")
                && !source.contains("CLINICAL_RANGES")
                && !source.contains("align-map-auto")
        );
    }

    #[test]
    fn numeric_range_edits_recolour_the_cached_map() {
        let source = production();
        assert!(source.contains("changed.then_some(AlignPanelAction::Measure)"));
    }

    #[test]
    fn the_map_is_locked_until_a_refined_match_lands() {
        let source = production();
        assert!(source.contains("if !refined_match_ready"));
        assert!(source.contains("settings.show_deviation = false"));
        assert!(source.contains("enabled && refined_match_ready"));
        assert!(source.contains("align-map-requires-refine"));
    }
}
