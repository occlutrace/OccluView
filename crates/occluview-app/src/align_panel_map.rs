//! The Hitmap block of the Align Scans window.
//!
//! Split from the window itself because it answers a different question. That
//! module is about getting two scans onto each other; this one is about reading
//! how far apart they ended up, and it is the part an operator stares at.
//!
//! Everything above the fold is what a metrology tool puts in front of someone:
//! the colour bar, one slider that scales it, and the numbers the colours stand
//! for. Everything else only matters when something looks wrong, and lives
//! behind "More settings".

use eframe::egui;
use occluview_align::{DeviationStats, RampMode};

use crate::align_panel::AlignPanelAction;
use crate::align_worker::{AlignSettings, CLINICAL_CEILING_MM, CLINICAL_MAX_MM, CLINICAL_MIN_MM};
use crate::mesh_editor_icons::{self, EditorIcon};
use crate::{align_overlay, ui_theme};

/// Show the Hitmap block; returns what the operator asked for.
pub(crate) fn show(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    stats: Option<DeviationStats>,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = toggle(ui, settings);
    if !settings.show_deviation {
        return action;
    }

    align_overlay::paint_legend(ui, *settings);
    action = action.or(presets(ui, settings, enabled));
    action = action.or(range(ui, settings, enabled));
    if let Some(stats) = stats {
        numbers(ui, stats, settings.tolerance_mm);
        saturation(ui, stats, *settings);
    }
    ui.collapsing("More settings", |ui| {
        if details(ui, settings) {
            action = Some(AlignPanelAction::Measure);
        }
    });
    action
}

/// The one control that decides whether the map is on screen at all.
fn toggle(ui: &mut egui::Ui, settings: &mut AlignSettings) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover())
            .0;
        mesh_editor_icons::paint(
            ui.painter(),
            glyph,
            EditorIcon::Heatmap,
            if settings.show_deviation {
                ui_theme::ACCENT
            } else {
                ui_theme::TEXT_MUTED
            },
            settings.show_deviation,
        );
        let mut shown = settings.show_deviation;
        if ui
            .checkbox(&mut shown, "Hitmap")
            .on_hover_text("Colour one scan by how far it sits from the other")
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

/// The standard ranges, one click each.
///
/// Dentistry works to a fifth of a millimetre, and a tool that makes an
/// operator dial that in by hand every session is a tool that will be read at
/// whatever range it happened to be left at. **0.20 mm** is the clinical
/// default and the one the window opens on; the other two are the tighter and
/// looser bands the same work uses.
fn presets(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
) -> Option<AlignPanelAction> {
    /// Maximum distance, in millimetres, and the nominal band that goes with it.
    const RANGES: [(f64, f64); 3] = [
        (0.10, 0.005),
        (CLINICAL_MAX_MM, CLINICAL_MIN_MM),
        (0.50, 0.02),
    ];

    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("range")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        let width = (ui.available_width() - ui.spacing().item_spacing.x * 3.0) / 3.0;
        for (max_mm, min_mm) in RANGES {
            let active = (settings.scale_mm - max_mm).abs() < f64::EPSILON
                && (settings.tolerance_mm - min_mm).abs() < f64::EPSILON;
            if crate::align_panel::chip(ui, width, None, &format!("{max_mm:.2}"), enabled, active)
                .on_hover_text(format!(
                    "Everything under {min_mm:.3} mm reads as agreement, {max_mm:.2} mm saturates"
                ))
                .clicked()
            {
                settings.scale_mm = max_mm;
                settings.tolerance_mm = min_mm;
                settings.auto_scale = false;
                action = Some(AlignPanelAction::Measure);
            }
        }
    });
    action
}

/// The two numbers exocad exposes: the minimum and the maximum distance.
///
/// Directly under the bar they define — the arrangement every metrology tool
/// uses, because the bar is the legend for the sliders. They are not decoration
/// either: **min** is the nominal band, so everything closer than it is painted
/// one colour, and **max** is where the ramp saturates.
fn range(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = None;
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut settings.tolerance_mm, 0.005..=0.10)
                .suffix(" mm")
                .fixed_decimals(3)
                .text("min"),
        )
        .drag_stopped()
    {
        action = Some(AlignPanelAction::Measure);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                enabled,
                egui::Slider::new(&mut settings.scale_mm, 0.05..=CLINICAL_CEILING_MM)
                    .suffix(" mm")
                    .fixed_decimals(2)
                    .text("max"),
            )
            .drag_stopped()
        {
            // Their range now, not the tool's.
            settings.auto_scale = false;
            action = Some(AlignPanelAction::Measure);
        }
        if !settings.auto_scale
            && ui
                .small_button("auto")
                .on_hover_text("Fit the range to the measurement again")
                .clicked()
        {
            settings.auto_scale = true;
            action = Some(AlignPanelAction::Measure);
        }
    });
    action
}

/// Say so when the range is far too small for what was measured.
///
/// This is the state behind the map an operator called a thermal camera: a
/// 0.20 mm range over a pair sitting 1.4 mm apart pins every vertex to an end
/// stop, and the arch comes out as a red and blue mosaic with no structure in
/// it. The colours are not wrong — the range is — and nothing on screen said so.
fn saturation(ui: &mut egui::Ui, stats: DeviationStats, settings: AlignSettings) {
    if let Some(text) = saturation_advice(stats, settings) {
        ui.label(egui::RichText::new(text).size(10.5).color(ui_theme::TEXT));
    }
}

/// Which sentence the saturation warning is, or none if the range is fine.
///
/// Split from the drawing above so the decision can be run in a test. Left
/// inside the `egui` call it was only reachable by a test that read this file's
/// own source text as a string — which passes on a logic change and fails on a
/// rename, the exact opposite of what a test is for.
fn saturation_advice(stats: DeviationStats, settings: AlignSettings) -> Option<String> {
    let summary = stats.summary.filter(|summary| summary.p95.is_finite())?;
    if summary.p95 <= settings.scale_mm {
        return None;
    }
    // The advice has to be the advice that helps, and past the clinical ceiling
    // a wider range is not it: a map of two meshes millimetres apart is a
    // picture of an alignment that has not happened, and widening the range
    // only makes a prettier picture of the same thing.
    Some(if summary.p95 > CLINICAL_CEILING_MM {
        format!(
            "These meshes are about {:.1} mm apart — align them before reading the map",
            summary.p95
        )
    } else {
        format!(
            "Most of this is past {:.2} mm, so the colours are pinned to the ends — widen the range",
            settings.scale_mm
        )
    })
}

/// The numbers behind the colours, including what could not be measured.
///
/// When there was not enough measured surface to characterise, this says so
/// instead of printing a figure. A "0.000" in the one field a clinician reads
/// is indistinguishable from two surfaces that coincide perfectly.
fn numbers(ui: &mut egui::Ui, stats: DeviationStats, tolerance_mm: f64) {
    let Some(summary) = stats.summary else {
        ui.label(
            egui::RichText::new(format!(
                "Not enough surface to measure — {} of {} vertices reached the other scan",
                stats.measured,
                stats.measured.saturating_add(stats.skipped)
            ))
            .size(11.0)
            .color(ui_theme::TEXT),
        );
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{:.0}% within {tolerance_mm:.2} mm",
                summary.within_tolerance * 100.0
            ))
            .size(11.0)
            .color(ui_theme::TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("rms {:.3}", summary.rms))
                    .size(11.0)
                    .color(ui_theme::TEXT_MUTED),
            );
        });
    });
    if stats.skipped > 0 {
        ui.label(
            egui::RichText::new(format!("{} vertices had nothing to measure", stats.skipped))
                .size(10.0)
                .color(ui_theme::TEXT_MUTED),
        );
    }
}

/// The knobs that only matter when something looks wrong.
fn details(ui: &mut egui::Ui, settings: &mut AlignSettings) -> bool {
    let mut changed = false;
    let mut banded = settings.bands.is_some();
    if ui
        .checkbox(&mut banded, "Stepped bands")
        .on_hover_text("Step the ramp instead of blending it")
        .changed()
    {
        settings.bands = banded.then_some(10);
        changed = true;
    }
    changed |= colours(ui, &mut settings.ramp_mode);
    changed
}

/// Which colour scheme the map paints with.
fn colours(ui: &mut egui::Ui, mode: &mut RampMode) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Colours")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        for (value, label, hint) in [
            (
                RampMode::Magnitude,
                "distance",
                "Cool where the scans agree, hot where they do not",
            ),
            (
                RampMode::Signed,
                "signed",
                "Blue below the surface, green nominal, red above",
            ),
        ] {
            if ui
                .selectable_label(*mode == value, label)
                .on_hover_text(hint)
                .clicked()
                && *mode != value
            {
                *mode = value;
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use crate::align_worker::AlignSettings;
    use occluview_align::{DeviationStats, DeviationSummary};

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = include_str!("align_panel_map.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// The measurement is only honest if the block says how much of the scan it
    /// could not measure.
    #[test]
    fn the_numbers_report_unmeasured_vertices() {
        assert!(production().contains("had nothing to measure"));
    }

    /// The operator asked for this name specifically. "Show distance" was a
    /// description of a checkbox; Hitmap is what they call the thing.
    #[test]
    fn the_map_is_called_what_the_operator_calls_it() {
        assert!(production().contains("\"Hitmap\""));
    }

    /// The two numbers that define the bar are exocad's minimum and maximum
    /// distance, and both belong in front of the operator: min is the nominal
    /// band and max is where the ramp saturates, so a map cannot be read
    /// without them.
    #[test]
    fn the_minimum_and_maximum_distance_are_in_front_of_the_operator() {
        let show = production()
            .split_once("pub(crate) fn show(")
            .and_then(|(_, rest)| rest.split_once("\n/// The one control"))
            .map(|(body, _)| body)
            .expect("the block's own body");
        let fold = show
            .find("ui.collapsing(\"More settings\"")
            .expect("a fold to put the diagnostics behind");
        let drawn = show
            .find("range(ui, settings, enabled)")
            .expect("the range controls");
        assert!(drawn < fold, "the range controls are behind the fold");
    }

    /// The map an operator called a thermal camera was a correct map over a
    /// range seven times too small. Nothing on screen said so.
    ///
    /// And the advice has to be the advice that helps: past the clinical
    /// ceiling a wider range is not it, because the picture is of an alignment
    /// that has not happened yet.
    #[test]
    fn a_range_too_small_for_the_measurement_says_so() {
        let at = |p95: f64, scale_mm: f64| {
            super::saturation_advice(
                DeviationStats {
                    measured: 1_000,
                    skipped: 0,
                    summary: Some(DeviationSummary {
                        within_tolerance: 0.0,
                        mean_abs: p95 / 2.0,
                        rms: p95 / 2.0,
                        median: p95 / 2.0,
                        p95,
                        max_abs: p95,
                    }),
                },
                AlignSettings {
                    scale_mm,
                    ..AlignSettings::default()
                },
            )
        };

        assert!(
            at(0.05, 0.20).is_none(),
            "a range that already covers the measurement needs no warning"
        );

        let pinned = at(0.40, 0.20).expect("a measurement past the range must say so");
        assert!(
            pinned.contains("the colours are pinned to the ends"),
            "got: {pinned}"
        );

        // Past the clinical ceiling the honest advice changes: widening the
        // range only makes a prettier picture of an alignment that has not
        // happened. This is the state the operator called a thermal camera.
        let apart = at(1.40, 0.20).expect("meshes millimetres apart must say so");
        assert!(
            apart.contains("align them before reading the map"),
            "got: {apart}"
        );
        assert!(
            at(f64::NAN, 0.20).is_none(),
            "a non-finite measurement must not produce advice"
        );
    }

    /// A measurement that never happened has no range to advise about.
    #[test]
    fn nothing_measured_produces_no_advice_at_all() {
        let advice = super::saturation_advice(
            DeviationStats {
                measured: 0,
                skipped: 900_000,
                summary: None,
            },
            AlignSettings::default(),
        );
        assert!(advice.is_none());
    }

    /// A reading needs the bar, the scale, and the numbers. Everything else is
    /// a diagnostic, and diagnostics in front of the operator are what turned
    /// this block into the wall of knobs it replaced.
    #[test]
    fn the_rarely_used_knobs_are_only_reachable_through_the_fold() {
        let source = production();
        let fold = source
            .find("ui.collapsing(\"More settings\"")
            .expect("a fold to put the diagnostics behind");
        let call = source
            .find("details(ui, settings)")
            .expect("the diagnostics are drawn by details");
        assert!(fold < call, "the diagnostics must be drawn inside the fold");
        for knob in ["Stepped bands", "Colours", "Facing"] {
            let quoted = format!("\"{knob}\"");
            let at = source.find(&quoted).unwrap_or(usize::MAX);
            assert!(at > fold, "{knob} sits in front of the operator");
        }
    }
}
