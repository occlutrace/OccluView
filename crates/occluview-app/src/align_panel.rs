//! The Align Scans panel.
//!
//! Nothing here names a target, a source, or a role. The pair is established
//! by clicking points in the viewport, so the panel's job is to report what
//! was clicked, offer the two fits, and show the measurement honestly —
//! including what it could not measure.

use eframe::egui;
use occluview_align::{DeviationStats, Orientation};

use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::{align_overlay, ui_theme};

/// What the operator asked the panel for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignPanelAction {
    /// Fit the clicked pairs.
    Align,
    /// Seat the surfaces with ICP.
    Refine,
    /// Remove the last point or pair.
    Back,
    /// Drop the pair and start over.
    Clear,
    /// Close the tool.
    Close,
    /// Re-measure with the current settings.
    Measure,
    /// Stop showing the map.
    HideMap,
}

/// Everything the panel needs to draw itself.
pub(crate) struct AlignPanelView<'a> {
    /// The click model.
    pub(crate) tool: &'a AlignTool,
    /// Live settings, edited in place.
    pub(crate) settings: &'a mut AlignSettings,
    /// The last thing that happened, in a sentence.
    pub(crate) status: Option<&'a str>,
    /// The measurement summary, when there is one.
    pub(crate) stats: Option<DeviationStats>,
    /// Whether a job is in flight.
    pub(crate) busy: bool,
}

/// Draw the panel and return what the operator asked for.
pub(crate) fn show(ctx: &egui::Context, view: AlignPanelView<'_>) -> Option<AlignPanelAction> {
    let mut action = None;
    egui::Window::new("Align Scans")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 56.0))
        .resizable(false)
        .collapsible(false)
        .frame(ui_theme::overlay_frame())
        .show(ctx, |ui| {
            ui.set_min_width(248.0);
            ui.spacing_mut().item_spacing.y = 6.0;

            show_pairs(ui, view.tool);
            ui.separator();
            show_fit_buttons(ui, view.tool, view.busy, &mut action);
            ui.separator();
            show_measurement(ui, view.settings, view.stats, &mut action);

            if let Some(status) = view.status {
                ui.separator();
                ui.label(
                    egui::RichText::new(status)
                        .size(11.0)
                        .color(ui_theme::TEXT_WEAK),
                );
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    action = Some(AlignPanelAction::Close);
                }
            });
        });
    action
}

/// The pair readout: how many correspondences, and what is still expected.
fn show_pairs(ui: &mut egui::Ui, tool: &AlignTool) {
    let placed = tool.pairs().len();
    let prompt = if tool.moving_layer().is_none() {
        "Click a point on the scan that should move".to_owned()
    } else if tool.pending().is_some() {
        "Click the matching spot on the other scan".to_owned()
    } else if placed == 0 {
        "Click a point on each scan to pair them".to_owned()
    } else {
        format!("{placed} pair{} placed", if placed == 1 { "" } else { "s" })
    };
    ui.label(egui::RichText::new(prompt).size(12.0).color(ui_theme::TEXT));
}

/// The two fits and the two ways to take a click back.
fn show_fit_buttons(
    ui: &mut egui::Ui,
    tool: &AlignTool,
    busy: bool,
    action: &mut Option<AlignPanelAction>,
) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(tool.can_align() && !busy, egui::Button::new("Align"))
            .on_hover_text("Fit the clicked pairs")
            .clicked()
        {
            *action = Some(AlignPanelAction::Align);
        }
        if ui
            .add_enabled(tool.can_measure() && !busy, egui::Button::new("Refine"))
            .on_hover_text("Seat the surfaces against each other")
            .clicked()
        {
            *action = Some(AlignPanelAction::Refine);
        }
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("Back"))
            .on_hover_text("Remove the last point")
            .clicked()
        {
            *action = Some(AlignPanelAction::Back);
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Clear"))
            .on_hover_text("Drop the pair and start over")
            .clicked()
        {
            *action = Some(AlignPanelAction::Clear);
        }
    });
}

/// The deviation controls, legend, and statistics.
fn show_measurement(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    stats: Option<DeviationStats>,
    action: &mut Option<AlignPanelAction>,
) {
    let mut changed = false;
    if ui
        .checkbox(&mut settings.show_deviation, "Show deviation")
        .changed()
    {
        *action = Some(if settings.show_deviation {
            AlignPanelAction::Measure
        } else {
            AlignPanelAction::HideMap
        });
    }
    if !settings.show_deviation {
        return;
    }

    changed |= ui
        .add(
            egui::Slider::new(&mut settings.scale_mm, 0.1..=2.0)
                .text("scale ±mm")
                .fixed_decimals(2),
        )
        .drag_stopped();
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.tolerance_mm, 0.01..=1.0)
                .text("tolerance mm")
                .fixed_decimals(2),
        )
        .drag_stopped();
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.influence_radius_mm, 0.2..=10.0)
                .text("max distance mm")
                .fixed_decimals(1),
        )
        .drag_stopped();

    let mut banded = settings.bands.is_some();
    if ui
        .checkbox(&mut banded, "Bands")
        .on_hover_text("Step the ramp instead of blending it — the tolerance edge reads sharply")
        .changed()
    {
        settings.bands = banded.then_some(10);
        changed = true;
    }

    changed |= show_orientation(ui, &mut settings.orientation);

    align_overlay::paint_legend(ui, *settings);

    if let Some(stats) = stats {
        show_stats(ui, stats, settings.tolerance_mm);
    }

    if changed {
        *action = Some(AlignPanelAction::Measure);
    }
}

/// The surface-orientation rule. An inverted fixed mesh flips the whole map's
/// sign, so this is not a curiosity — it is the escape hatch for a scan whose
/// winding disagrees with itself.
fn show_orientation(ui: &mut egui::Ui, orientation: &mut Orientation) -> bool {
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
                "The fixed scan's winding is inverted",
            ),
            (Orientation::Ignored, "unsigned", "Report distance only"),
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

/// The numbers behind the colours, including what could not be measured.
fn show_stats(ui: &mut egui::Ui, stats: DeviationStats, tolerance_mm: f64) {
    let row = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(11.0)
                    .color(ui_theme::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(value).size(11.0).color(ui_theme::TEXT));
            });
        });
    };
    row(
        ui,
        &format!("within {tolerance_mm:.2} mm"),
        format!("{:.1}%", stats.within_tolerance * 100.0),
    );
    row(ui, "mean", format!("{:.3} mm", stats.mean_abs));
    row(ui, "rms", format!("{:.3} mm", stats.rms));
    row(ui, "p95", format!("{:.3} mm", stats.p95));
    if stats.skipped > 0 {
        row(ui, "no data", format!("{} vertices", stats.skipped));
    }
}

#[cfg(test)]
mod tests {
    /// The whole point of this tool is that there is no object picker. If a
    /// control ever names a target or a role, the simplification is gone.
    #[test]
    fn no_control_in_the_panel_names_a_target_a_source_or_a_role() {
        let source = include_str!("align_panel.rs");
        let production = source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before);
        // Only what the operator can read. A doc comment may well discuss the
        // roles this tool deliberately does not have.
        for literal in production.split('"').skip(1).step_by(2) {
            let lowered = literal.to_lowercase();
            for banned in ["target", "source object", "primary object", "role"] {
                assert!(
                    !lowered.contains(banned),
                    "a panel control says {literal:?}, which names {banned}"
                );
            }
        }
    }

    /// The measurement is only honest if the panel says how much of the scan it
    /// could not measure.
    #[test]
    fn the_statistics_report_unmeasured_vertices() {
        let source = include_str!("align_panel.rs");
        assert!(
            source.contains("\"no data\""),
            "the stats block must report vertices with nothing to measure against"
        );
    }
}
