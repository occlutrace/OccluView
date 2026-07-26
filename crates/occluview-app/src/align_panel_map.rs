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
use occluview_align::{DeviationStats, Orientation, RampMode};

use crate::align_panel::AlignPanelAction;
use crate::align_worker::AlignSettings;
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
    action = action.or(scale(ui, settings, enabled));
    if let Some(stats) = stats {
        numbers(ui, stats, settings.tolerance_mm);
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

/// The display range, directly under the bar it scales — the arrangement every
/// metrology tool uses, because the bar is the legend for the slider.
fn scale(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                enabled,
                egui::Slider::new(&mut settings.scale_mm, 0.05..=2.0)
                    .suffix(" mm")
                    .text("scale"),
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

/// The numbers behind the colours, including what could not be measured.
fn numbers(ui: &mut egui::Ui, stats: DeviationStats, tolerance_mm: f64) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{:.0}% within {tolerance_mm:.2} mm",
                stats.within_tolerance * 100.0
            ))
            .size(11.0)
            .color(ui_theme::TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("rms {:.3}", stats.rms))
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
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.tolerance_mm, 0.01..=1.0)
                .suffix(" mm")
                .text("tolerance"),
        )
        .drag_stopped();
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.influence_radius_mm, 0.2..=10.0)
                .suffix(" mm")
                .text("max distance"),
        )
        .drag_stopped();

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
    changed |= facing(ui, &mut settings.orientation);
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

/// The surface-orientation rule. An inverted mesh flips the whole signed map,
/// so this is the escape hatch for a scan whose winding disagrees with itself.
fn facing(ui: &mut egui::Ui, orientation: &mut Orientation) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Facing")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        for (value, label, hint) in [
            (Orientation::Match, "as is", "Surfaces face the same way"),
            (
                Orientation::Inverted,
                "flipped",
                "The other scan's winding is inverted",
            ),
            (Orientation::Ignored, "either", "Accept either facing"),
        ] {
            if ui
                .selectable_label(*orientation == value, label)
                .on_hover_text(hint)
                .clicked()
                && *orientation != value
            {
                *orientation = value;
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

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
        for knob in ["tolerance", "max distance", "Stepped bands", "Facing"] {
            let quoted = format!("\"{knob}\"");
            let at = source.find(&quoted).unwrap_or(usize::MAX);
            assert!(at > fold, "{knob} sits in front of the operator");
        }
    }
}
