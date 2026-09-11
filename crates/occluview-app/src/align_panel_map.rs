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

    settings.scale_mm = settings
        .scale_mm
        .clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM);
    align_overlay::paint_legend(ui, *settings, locale);
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

/// Set the maximum absolute deviation shown by the heatmap.
///
/// Zero is always the blue origin. The upper stop is intentionally bounded at
/// 0.10 mm, so values beyond the selected range are hot red instead of opening
/// an unbounded clinical-scale control that hides small discrepancies.
fn range(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    settings.scale_mm = settings
        .scale_mm
        .clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM);
    settings.auto_scale = false;
    let response = ui.add_enabled(
        enabled,
        egui::Slider::new(
            &mut settings.scale_mm,
            WORKING_SCALE_MIN_MM..=WORKING_MAX_MM,
        )
        .suffix(" mm")
        .fixed_decimals(2)
        .text(locale.tr("align-map-max").as_str()),
    );
    // Keep a drag cheap until the operator releases it, but do not leave a
    // keyboard edit visually stale: egui reports arrow-key changes without a
    // drag lifecycle. The cached deviation map makes the resulting action a
    // recolour, not another distance search.
    (response.drag_stopped() || (response.changed() && !response.dragged()))
        .then_some(AlignPanelAction::Measure)
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
    fn the_working_panel_has_one_bounded_absolute_range() {
        let source = production();
        assert!(source.contains("range(ui, settings, enabled, locale)"));
        assert!(source.contains("WORKING_SCALE_MIN_MM..=WORKING_MAX_MM"));
        assert!(source.contains(".clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM)"));
        assert!(
            !source.contains("CLINICAL_CEILING_MM")
                && !source.contains("CLINICAL_RANGES")
                && !source.contains("align-map-min")
                && !source.contains("align-map-auto")
        );
    }

    #[test]
    fn keyboard_range_edits_recolour_without_remeasuring_during_a_drag() {
        let source = production();
        assert!(
            source
                .contains("response.drag_stopped() || (response.changed() && !response.dragged())"),
            "keyboard edits must repaint the existing map"
        );
        assert!(
            source.contains("let response = ui.add_enabled("),
            "the slider response must be inspected instead of dropping keyboard changes"
        );
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
