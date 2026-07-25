//! Painting for Align Scans: the clicked pairs and the deviation legend.
//!
//! Markers re-project through the live camera every frame from each point's
//! **local** coordinates, so they stay welded to their surface after a fit
//! moves a scan underneath them.

use eframe::egui;
use glam::Affine3A;
use occluview_align::RampMode;
use occluview_core::{Camera, Scene, SceneMeshId};

use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::measure_draw;
use crate::ui_theme;
use crate::viewer::project_world_to_viewport;

/// Radius of a numbered pair marker.
const MARKER_RADIUS_PX: f32 = 6.0;

/// Fill behind a marker's number, so a digit stays legible over any scan.
const MARKER_FILL: egui::Color32 = egui::Color32::from_rgb(246, 247, 249);

/// The colour a rejected pair is drawn in. It is shown, not hidden: the
/// operator needs to see which click the fit threw away.
const REJECTED_INK: egui::Color32 = egui::Color32::from_rgb(198, 64, 48);

/// Everything one paint pass needs.
pub(crate) struct PairPaint<'a> {
    /// The live camera the markers re-project through.
    pub(crate) camera: &'a Camera,
    /// The viewport the markers land in.
    pub(crate) viewport_rect: egui::Rect,
    /// The scene, for each layer's current pose.
    pub(crate) scene: &'a Scene,
    /// The click model.
    pub(crate) tool: &'a AlignTool,
    /// Pairs the last fit threw away.
    pub(crate) rejected: &'a [u32],
    /// The cursor, for the rubber band to a half-placed point.
    pub(crate) hover: Option<egui::Pos2>,
}

/// Paint every pair, the half-placed point, and the rubber band to the cursor.
pub(crate) fn paint_pairs(painter: &egui::Painter, view: &PairPaint<'_>) {
    let PairPaint {
        camera,
        viewport_rect,
        scene,
        tool,
        rejected,
        hover,
    } = *view;
    let pose_of = |layer: SceneMeshId| -> Option<Affine3A> {
        scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == layer)
            .map(|entry| entry.transform)
    };
    let project = |layer: SceneMeshId, local: glam::Vec3| -> Option<egui::Pos2> {
        let pose = pose_of(layer)?;
        project_world_to_viewport(camera, viewport_rect, pose.transform_point3(local))
            .map(|(pos, _)| pos)
    };

    for (index, pair) in tool.pairs().iter().enumerate() {
        let Some(moving) = project(pair.moving.layer, pair.moving.local) else {
            continue;
        };
        let Some(fixed) = project(pair.fixed.layer, pair.fixed.local) else {
            continue;
        };
        let outlier = u32::try_from(index).is_ok_and(|slot| rejected.contains(&slot));
        // A rejected pair is drawn in the error colour rather than hidden: the
        // operator needs to see which click the fit threw away.
        let ink = if outlier {
            REJECTED_INK
        } else {
            ui_theme::ACCENT
        };
        painter.line_segment(
            [moving, fixed],
            egui::Stroke::new(1.1, ink.gamma_multiply(0.7)),
        );
        marker(painter, moving, ink, index + 1);
        marker(painter, fixed, ink, index + 1);
    }

    if let Some(pending) = tool.pending() {
        if let Some(anchor) = project(pending.layer, pending.local) {
            if let Some(hover) = hover {
                painter.extend(egui::Shape::dashed_line(
                    &[anchor, hover],
                    egui::Stroke::new(1.1, ui_theme::ACCENT),
                    5.0,
                    4.0,
                ));
            }
            measure_draw::anchor_dot(painter, anchor);
        }
    }
}

/// One numbered marker.
fn marker(painter: &egui::Painter, at: egui::Pos2, ink: egui::Color32, number: usize) {
    painter.circle_filled(at, MARKER_RADIUS_PX, MARKER_FILL);
    painter.circle_stroke(at, MARKER_RADIUS_PX, egui::Stroke::new(1.6, ink));
    painter.text(
        at,
        egui::Align2::CENTER_CENTER,
        number.to_string(),
        egui::FontId::proportional(9.0),
        ink,
    );
}

/// Steps the legend bar is drawn in. Enough that a continuous ramp reads
/// smooth at the bar's width.
const LEGEND_STEPS: usize = 64;

/// The deviation, in millimetres, the legend bar carries at `step` of `steps`.
///
/// The magnitude ramp has **no negative side**: `ramp_color` takes the absolute
/// value, so sweeping its bar from `-scale` mirrors it — the hot end is drawn
/// at *both* ends and the ramp's zero colour lands in the middle, where every
/// metrology legend puts nominal. An operator reading that bar is told blue
/// means nominal, when on the surface blue means zero. The signed ramp really
/// does run `-scale` to `+scale`, and only it is swept that way.
pub(crate) fn legend_value_mm(step: usize, steps: usize, mode: RampMode, scale_mm: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let fraction = step as f64 / (steps.max(2) - 1) as f64;
    match mode {
        RampMode::Magnitude => fraction * scale_mm,
        RampMode::Signed => fraction.mul_add(2.0, -1.0) * scale_mm,
    }
}

/// The number written under each end of the legend bar, in millimetres.
fn legend_bounds(mode: RampMode, scale_mm: f64) -> (String, String) {
    match mode {
        RampMode::Magnitude => ("0.00 mm".to_owned(), format!("{scale_mm:.2} mm")),
        RampMode::Signed => (format!("−{scale_mm:.2} mm"), format!("+{scale_mm:.2} mm")),
    }
}

/// Paint the deviation legend: the colour ramp with the numeric bounds of the
/// scale it was measured against, so a colour on screen can always be read as
/// a number.
pub(crate) fn paint_legend(ui: &mut egui::Ui, settings: AlignSettings) {
    const WIDTH: f32 = 200.0;
    const HEIGHT: f32 = 12.0;
    const STEPS: usize = LEGEND_STEPS;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    for step in 0..STEPS {
        let color = occluview_align::ramp_color(
            legend_value_mm(step, STEPS, settings.ramp_mode, settings.scale_mm),
            &occluview_align::RampSettings {
                scale_mm: settings.scale_mm,
                tolerance_mm: settings.tolerance_mm,
                bands: settings.bands,
                mode: settings.ramp_mode,
            },
        );
        #[allow(clippy::cast_precision_loss)]
        let x0 = rect.left() + rect.width() * (step as f32 / STEPS as f32);
        #[allow(clippy::cast_precision_loss)]
        let x1 = rect.left() + rect.width() * ((step + 1) as f32 / STEPS as f32);
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.top()),
                egui::pos2(x1 + 1.0, rect.bottom()),
            ),
            0.0,
            egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 255),
        );
    }

    ui.horizontal(|ui| {
        let label = |ui: &mut egui::Ui, text: String| {
            ui.label(
                egui::RichText::new(text)
                    .size(10.0)
                    .color(ui_theme::TEXT_MUTED),
            );
        };
        let (low, high) = legend_bounds(settings.ramp_mode, settings.scale_mm);
        label(ui, low);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            label(ui, high);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::{legend_bounds, legend_value_mm, LEGEND_STEPS};
    use occluview_align::{ramp_color, RampMode, RampSettings};

    fn bar(mode: RampMode, scale_mm: f64) -> Vec<[u8; 4]> {
        let ramp = RampSettings {
            scale_mm,
            tolerance_mm: 0.2,
            bands: None,
            mode,
        };
        (0..LEGEND_STEPS)
            .map(|step| ramp_color(legend_value_mm(step, LEGEND_STEPS, mode, scale_mm), &ramp))
            .collect()
    }

    /// The bug this test exists for: the bar used to sweep `-scale` to
    /// `+scale` in every mode, and `ramp_color` takes the absolute value in
    /// magnitude mode. The bar came out red-blue-red — mirrored, with the
    /// ramp's ZERO colour in the middle where a legend puts nominal, and half
    /// of it labelled with a negative magnitude that cannot exist.
    #[test]
    fn the_magnitude_legend_runs_cold_to_hot_and_never_mirrors() {
        let bar = bar(RampMode::Magnitude, 0.5);
        let first = bar[0];
        let last = bar[bar.len() - 1];
        assert!(
            first[2] > 200 && first[0] == 0,
            "the bar must start at the cold end, got {first:?}"
        );
        assert!(
            last[0] > 200 && last[2] < 60,
            "the bar must end at the hot end, got {last:?}"
        );
        assert_ne!(
            first, last,
            "a mirrored bar has the same colour at both ends"
        );
        for pair in bar.windows(2) {
            assert!(
                pair[1][0] >= pair[0][0] && pair[1][2] <= pair[0][2],
                "the bar doubled back: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// The signed ramp genuinely has two sides, and its bar must still show
    /// both: blue below, green at nominal, red above.
    #[test]
    fn the_signed_legend_still_spans_both_sides() {
        let bar = bar(RampMode::Signed, 0.5);
        let first = bar[0];
        let middle = bar[bar.len() / 2];
        let last = bar[bar.len() - 1];
        assert!(first[2] > first[0], "the cold end must be blue: {first:?}");
        assert!(
            middle[1] > middle[0] && middle[1] > middle[2],
            "nominal must sit in the middle: {middle:?}"
        );
        assert!(last[0] > last[2], "the hot end must be red: {last:?}");
    }

    /// A bar the numbers disagree with is worse than no bar: the operator
    /// reads the colour through the label.
    #[test]
    fn the_bounds_name_the_scale_the_bar_was_drawn_over() {
        assert_eq!(
            legend_bounds(RampMode::Magnitude, 0.5),
            ("0.00 mm".to_owned(), "0.50 mm".to_owned()),
            "a magnitude bar starts at nothing, never at a negative distance"
        );
        assert_eq!(
            legend_bounds(RampMode::Signed, 0.5),
            ("−0.50 mm".to_owned(), "+0.50 mm".to_owned())
        );
    }
}
