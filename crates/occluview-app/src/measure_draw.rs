//! Shared measurement drawing — the "ray" look reused by the main-viewport
//! measure overlay ([`crate::measure_overlay`]) and the Section-panel ruler
//! ([`crate::cut_ruler`]).
//!
//! Both surfaces draw the same primitives so a wall-thickness chord reads
//! identically whether it is painted over the 3D model or inside the section
//! slice: a soft white halo under a hairline accent segment, endpoint dots
//! (a white-haloed anchor and a bare accent exit dot), and a frosted `NN.NN mm`
//! pill. All coordinates are already-projected panel pixels; the caller owns the
//! world<->pixel mapping.

use eframe::egui;

use crate::ui_theme;

/// Endpoint marker sizing (logical px, so DPI- and zoom-sane). Matched across
/// both measuring surfaces so they read as one tool.
pub(crate) const ANCHOR_HALO_PX: f32 = 4.5;
pub(crate) const ANCHOR_DOT_PX: f32 = 2.75;
/// Segment stroke: a hairline accent over a slightly wider white halo.
const SEGMENT_STROKE_PX: f32 = 1.4;
const SEGMENT_HALO_PX: f32 = 3.2;
/// Label text size and how far the pill is lifted off its anchor point.
const LABEL_TEXT_PX: f32 = 11.0;
pub(crate) const LABEL_LIFT_PX: f32 = 14.0;

/// Thin, precise measurement segment: a soft white halo under a hairline accent
/// stroke so the line reads on both dark and light geometry.
pub(crate) fn segment(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2) {
    painter.line_segment(
        [a, b],
        egui::Stroke::new(
            SEGMENT_HALO_PX,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170),
        ),
    );
    painter.line_segment(
        [a, b],
        egui::Stroke::new(SEGMENT_STROKE_PX, ui_theme::ACCENT),
    );
}

/// Endpoint anchor: white halo + accent dot (the primary end of a measurement).
pub(crate) fn anchor_dot(painter: &egui::Painter, pos: egui::Pos2) {
    painter.circle_filled(pos, ANCHOR_HALO_PX, egui::Color32::WHITE);
    painter.circle_filled(pos, ANCHOR_DOT_PX, ui_theme::ACCENT);
}

/// Bare accent dot for the far end of a thickness chord (the ray's exit point).
pub(crate) fn accent_dot(painter: &egui::Painter, pos: egui::Pos2) {
    painter.circle_filled(pos, ANCHOR_DOT_PX, ui_theme::ACCENT);
}

/// High-contrast label chip: frosted pill + hairline ring + ink text, centered
/// on `anchor`.
pub(crate) fn label_chip(
    painter: &egui::Painter,
    anchor: egui::Pos2,
    text: &str,
    ink: egui::Color32,
) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(LABEL_TEXT_PX),
        ink,
    );
    let pad = egui::vec2(4.0, 2.0);
    let bg = egui::Rect::from_center_size(anchor, galley.size() + pad * 2.0);
    painter.rect_filled(bg, 3.0, ui_theme::panel_fill());
    painter.rect_stroke(bg, 3.0, egui::Stroke::new(1.0, ui_theme::hairline()));
    painter.text(
        anchor,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(LABEL_TEXT_PX),
        ink,
    );
}

/// The full thickness "ray": the halo+accent chord from `entry` to `exit`, a
/// white-haloed anchor at `entry`, a bare accent dot at `exit`, and the mm chip
/// lifted above `entry`. This is the effect the owner wants reused verbatim in
/// the Section panel.
pub(crate) fn thickness_ray(
    painter: &egui::Painter,
    entry: egui::Pos2,
    exit: egui::Pos2,
    label: &str,
) {
    segment(painter, entry, exit);
    accent_dot(painter, exit);
    anchor_dot(painter, entry);
    label_chip(
        painter,
        egui::pos2(entry.x, entry.y - LABEL_LIFT_PX),
        label,
        ui_theme::TEXT,
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;

    /// Every primitive lays out and paints without panicking, including at the
    /// extreme coordinates a degenerate projection can produce.
    #[test]
    fn primitives_paint_without_panic_at_extreme_coords() {
        egui::__run_test_ui(|ui| {
            let painter = ui.painter();
            for &(a, b) in &[
                (egui::pos2(10.0, 10.0), egui::pos2(120.0, 90.0)),
                (egui::pos2(0.0, 0.0), egui::pos2(0.0, 0.0)),
                (egui::pos2(-1.0e6, 5.0e5), egui::pos2(1.0e6, -5.0e5)),
                (
                    egui::pos2(f32::MAX, f32::MIN),
                    egui::pos2(f32::MIN, f32::MAX),
                ),
            ] {
                segment(painter, a, b);
                anchor_dot(painter, a);
                accent_dot(painter, b);
                label_chip(painter, a, "1.23 mm", ui_theme::TEXT);
                thickness_ray(painter, a, b, "1.23 mm");
            }
            // Empty and long labels must not panic the galley layout either.
            label_chip(painter, egui::pos2(50.0, 50.0), "", ui_theme::TEXT);
            label_chip(
                painter,
                egui::pos2(50.0, 50.0),
                "open: no opposite wall",
                ui_theme::TEXT_WEAK,
            );
        });
    }
}
