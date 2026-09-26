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
/// Padding between a label's text and its pill edge.
const LABEL_PAD: egui::Vec2 = egui::Vec2::new(4.0, 2.0);
/// Clear space between a segment and a label placed beside it.
const LABEL_SIDE_GAP_PX: f32 = 6.0;
/// Leg length of the right-angle mark at a perpendicular's foot.
const RIGHT_ANGLE_LEG_PX: f32 = 8.0;
/// Radius of the arc that marks the angle where a ruler meets a line.
const ANGLE_ARC_RADIUS_PX: f32 = 18.0;
/// How far past the arc (or the right-angle mark) the angle's label centre
/// sits, along the bisector.
const ANGLE_LABEL_GAP_PX: f32 = 16.0;

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
        egui::Stroke::new(SEGMENT_STROKE_PX, ui_theme::accent()),
    );
}

/// Endpoint anchor: white halo + accent dot (the primary end of a measurement).
pub(crate) fn anchor_dot(painter: &egui::Painter, pos: egui::Pos2) {
    painter.circle_filled(pos, ANCHOR_HALO_PX, egui::Color32::WHITE);
    painter.circle_filled(pos, ANCHOR_DOT_PX, ui_theme::accent());
}

/// Bare accent dot for the far end of a thickness chord (the ray's exit point).
pub(crate) fn accent_dot(painter: &egui::Painter, pos: egui::Pos2) {
    painter.circle_filled(pos, ANCHOR_DOT_PX, ui_theme::accent());
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
    let bg = egui::Rect::from_center_size(anchor, galley.size() + LABEL_PAD * 2.0);
    painter.rect_filled(bg, 3.0, ui_theme::panel_fill());
    painter.rect_stroke(
        bg,
        3.0,
        egui::Stroke::new(1.0_f32, ui_theme::hairline()),
        egui::StrokeKind::Middle,
    );
    painter.text(
        anchor,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(LABEL_TEXT_PX),
        ink,
    );
}

/// Center for a label of `size` beside the midpoint of `a`-`b`, clear of the
/// segment by [`LABEL_SIDE_GAP_PX`] at any segment angle. The label goes on
/// the side away from `away_from` (another stroke meeting the segment); with
/// none, to the right of the segment, or above it when it is horizontal.
fn label_center_beside(
    a: egui::Pos2,
    b: egui::Pos2,
    size: egui::Vec2,
    away_from: Option<egui::Pos2>,
) -> egui::Pos2 {
    let mid = a.lerp(b, 0.5);
    let mut normal =
        unit(b - a).map_or(egui::vec2(0.0, -1.0), |along| egui::vec2(-along.y, along.x));
    let flip = match away_from {
        Some(other) => (other - mid).dot(normal) > 0.0,
        // A horizontal segment has a vertical normal: put the label above.
        None if normal.x.abs() < 1.0e-3 => normal.y > 0.0,
        None => normal.x < 0.0,
    };
    if flip {
        normal = -normal;
    }
    // Distance from the label's center to its edge along the normal.
    let half_extent = normal.x.abs() * size.x * 0.5 + normal.y.abs() * size.y * 0.5;
    mid + normal * (LABEL_SIDE_GAP_PX + half_extent)
}

/// [`label_chip`] placed beside a segment, see [`label_center_beside`].
pub(crate) fn label_chip_beside(
    painter: &egui::Painter,
    a: egui::Pos2,
    b: egui::Pos2,
    text: &str,
    away_from: Option<egui::Pos2>,
) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(LABEL_TEXT_PX),
        ui_theme::text(),
    );
    let size = galley.size() + LABEL_PAD * 2.0;
    label_chip(
        painter,
        label_center_beside(a, b, size, away_from),
        text,
        ui_theme::text(),
    );
}

/// Right-angle mark at `corner` between two on-screen directions. The legs
/// follow the projected directions, so the mark foreshortens with the view
/// the same way the 3D angle does. Skipped when either direction collapses.
pub(crate) fn right_angle_mark(
    painter: &egui::Painter,
    corner: egui::Pos2,
    along: egui::Vec2,
    toward: egui::Vec2,
) {
    let (Some(along), Some(toward)) = (unit(along), unit(toward)) else {
        return;
    };
    let points = vec![
        corner + along * RIGHT_ANGLE_LEG_PX,
        corner + (along + toward) * RIGHT_ANGLE_LEG_PX,
        corner + toward * RIGHT_ANGLE_LEG_PX,
    ];
    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(
            SEGMENT_HALO_PX,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170),
        ),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(SEGMENT_STROKE_PX, ui_theme::accent()),
    ));
}

/// Arc at `corner` over the angle between two on-screen directions, drawn
/// like the right-angle mark. Returns where the angle's label goes: past the
/// arc on its bisector. Skipped (`None`) when either direction collapses.
pub(crate) fn angle_arc(
    painter: &egui::Painter,
    corner: egui::Pos2,
    from: egui::Vec2,
    to: egui::Vec2,
) -> Option<egui::Pos2> {
    const STEPS: u8 = 16;
    let (from, to) = (unit(from)?, unit(to)?);
    let start = from.y.atan2(from.x);
    // Signed turn from `from` to `to`, the short way round.
    let sweep = from.x.mul_add(to.y, -(from.y * to.x)).atan2(from.dot(to));
    let points: Vec<egui::Pos2> = (0..=STEPS)
        .map(|step| {
            let angle = sweep.mul_add(f32::from(step) / f32::from(STEPS), start);
            corner + egui::vec2(angle.cos(), angle.sin()) * ANGLE_ARC_RADIUS_PX
        })
        .collect();
    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(
            SEGMENT_HALO_PX,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170),
        ),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(SEGMENT_STROKE_PX, ui_theme::accent()),
    ));
    angle_label_anchor(corner, from, to)
}

/// Where the label of the angle between two on-screen directions at `corner`
/// goes: on the bisector, past the arc. `None` when either direction
/// collapses. Directions pointing opposite ways bisect along the normal.
pub(crate) fn angle_label_anchor(
    corner: egui::Pos2,
    from: egui::Vec2,
    to: egui::Vec2,
) -> Option<egui::Pos2> {
    let (from, to) = (unit(from)?, unit(to)?);
    let bisector = unit(from + to).unwrap_or_else(|| from.rot90());
    Some(corner + bisector * (ANGLE_ARC_RADIUS_PX + ANGLE_LABEL_GAP_PX))
}

/// `v` scaled to unit length, or `None` when it has no usable direction.
fn unit(v: egui::Vec2) -> Option<egui::Vec2> {
    let length = v.length();
    (length.is_finite() && length > 1.0e-3).then(|| v / length)
}

/// Dashed continuation of a ruler line out to a perpendicular's foot that
/// falls beyond the ruler's ends.
pub(crate) fn extension(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2) {
    dashed(
        painter,
        from,
        to,
        egui::Stroke::new(1.0_f32, ui_theme::accent().gamma_multiply(0.7)),
        4.0,
    );
}

/// The dashed rubber band from a pending anchor: to the pointer, or to the
/// foot of the perpendicular a click would drop.
pub(crate) fn rubber_band(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2) {
    dashed(
        painter,
        from,
        to,
        egui::Stroke::new(1.1_f32, ui_theme::accent()),
        5.0,
    );
}

/// A dashed stroke (dash length `dash`, gap 4 px), clipped to the painter's
/// clip rect first. egui emits one shape per dash along the whole segment, so
/// an end projected far off screen (a deep zoom, a foot far along a line's
/// extension) would otherwise cost a shape for every dash nobody can see, and
/// an end at an extreme coordinate would never finish.
fn dashed(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke, dash: f32) {
    const GAP_PX: f32 = 4.0;
    let visible = painter.clip_rect().expand(dash + GAP_PX);
    if let Some((a, b)) = clip_segment(a, b, visible) {
        painter.extend(egui::Shape::dashed_line(&[a, b], stroke, dash, GAP_PX));
    }
}

/// The part of segment `a`-`b` inside `rect` (Liang-Barsky), or `None` when
/// none of it is, or when the segment is not finite.
fn clip_segment(
    a: egui::Pos2,
    b: egui::Pos2,
    rect: egui::Rect,
) -> Option<(egui::Pos2, egui::Pos2)> {
    let d = b - a;
    if !(a.is_finite() && d.x.is_finite() && d.y.is_finite()) {
        return None;
    }
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-d.x, a.x - rect.min.x),
        (d.x, rect.max.x - a.x),
        (-d.y, a.y - rect.min.y),
        (d.y, rect.max.y - a.y),
    ] {
        if p.abs() <= f32::EPSILON {
            // Parallel to this edge: inside the slab or not at all.
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            enter = enter.max(r);
        } else {
            leave = leave.min(r);
        }
        if enter > leave {
            return None;
        }
    }
    Some((a + d * enter, a + d * leave))
}

/// The full thickness "ray": the halo+accent chord from `entry` to `exit`, a
/// white-haloed anchor at `entry`, a bare accent dot at `exit`, and the mm chip
/// lifted above `entry`. This is the effect the Section panel reuses verbatim.
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
        ui_theme::text(),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::expect_used)]
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
                label_chip(painter, a, "1.23 mm", ui_theme::text());
                thickness_ray(painter, a, b, "1.23 mm");
                label_chip_beside(painter, a, b, "1.23 mm", Some(b));
                right_angle_mark(painter, a, b - a, egui::vec2(0.0, 1.0));
                right_angle_mark(painter, a, egui::Vec2::ZERO, egui::vec2(0.0, 1.0));
                let _ = angle_arc(painter, a, b - a, egui::vec2(0.0, 1.0));
                let _ = angle_arc(painter, a, egui::Vec2::ZERO, egui::vec2(0.0, 1.0));
                extension(painter, a, b);
            }
            // Empty and long labels must not panic the galley layout either.
            label_chip(painter, egui::pos2(50.0, 50.0), "", ui_theme::text());
            label_chip(
                painter,
                egui::pos2(50.0, 50.0),
                "open: no opposite wall",
                ui_theme::text_weak(),
            );
        });
    }

    fn distance_to_segment(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
        let along = b - a;
        let t = ((point - a).dot(along) / along.length_sq()).clamp(0.0, 1.0);
        point.distance(a + along * t)
    }

    /// A label beside a segment clears it at every angle, so a perpendicular's
    /// length never sits on its own line.
    #[test]
    fn a_label_beside_a_segment_clears_it_at_every_angle() {
        let size = egui::vec2(52.0, 16.0);
        let center = egui::pos2(300.0, 200.0);
        for step in 0_u8..24 {
            let angle = std::f32::consts::TAU * f32::from(step) / 24.0;
            let offset = egui::vec2(angle.cos(), angle.sin()) * 40.0;
            let (a, b) = (center - offset, center + offset);
            let label = egui::Rect::from_center_size(label_center_beside(a, b, size, None), size);
            // Nearest point of the label box to the segment: sample its outline.
            let nearest = (0_u8..=40)
                .flat_map(|i| {
                    let f = f32::from(i) / 40.0;
                    [
                        label.lerp_inside(egui::vec2(f, 0.0)),
                        label.lerp_inside(egui::vec2(f, 1.0)),
                        label.lerp_inside(egui::vec2(0.0, f)),
                        label.lerp_inside(egui::vec2(1.0, f)),
                    ]
                })
                .map(|p| distance_to_segment(p, a, b))
                .fold(f32::INFINITY, f32::min);
            assert!(
                nearest >= LABEL_SIDE_GAP_PX - 0.5,
                "step {step}: label {nearest} px from its segment"
            );
        }
    }

    #[test]
    fn a_label_moves_to_the_far_side_of_a_crossing_stroke() {
        let size = egui::vec2(52.0, 16.0);
        let (a, b) = (egui::pos2(100.0, 200.0), egui::pos2(300.0, 200.0));
        let above = egui::pos2(200.0, 120.0);
        let below = egui::pos2(200.0, 280.0);
        assert!(label_center_beside(a, b, size, Some(above)).y > 200.0);
        assert!(label_center_beside(a, b, size, Some(below)).y < 200.0);
        assert!(
            label_center_beside(a, b, size, None).y < 200.0,
            "a horizontal segment labels above by default"
        );
        let vertical = label_center_beside(
            egui::pos2(200.0, 100.0),
            egui::pos2(200.0, 300.0),
            size,
            None,
        );
        assert!(vertical.x > 200.0, "a vertical segment labels to the right");
    }

    /// The arc spans the short way between the two directions at its radius,
    /// and the label sits on the bisector clear of the arc.
    #[test]
    fn the_angle_arc_spans_the_smaller_opening_and_labels_its_bisector() {
        let corner = egui::pos2(200.0, 200.0);
        let along = egui::vec2(1.0, 0.0);
        let up_left = egui::vec2(-1.0, -1.0);
        egui::__run_test_ui(|ui| {
            let painter = ui.painter();
            let label = angle_arc(painter, corner, along, up_left).expect("an arc");
            let offset = label - corner;
            assert!((offset.length() - (ANGLE_ARC_RADIUS_PX + ANGLE_LABEL_GAP_PX)).abs() < 1.0e-3);
            // 135 degrees between them; the bisector points up and right of up.
            let bisector_angle = offset.y.atan2(offset.x).to_degrees();
            assert!((bisector_angle + 67.5).abs() < 1.0e-3, "{bisector_angle}");
            assert!(angle_arc(painter, corner, egui::Vec2::ZERO, along).is_none());
        });
        let opposite = angle_label_anchor(corner, along, -along).expect("a label");
        assert!(
            (opposite - corner).x.abs() < 1.0e-3,
            "a straight angle labels off the line, not on it"
        );
    }

    #[test]
    fn clip_keeps_the_visible_part_of_a_segment() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        let inside = (egui::pos2(10.0, 10.0), egui::pos2(90.0, 50.0));
        assert_eq!(clip_segment(inside.0, inside.1, rect), Some(inside));
        assert_eq!(
            clip_segment(egui::pos2(50.0, 50.0), egui::pos2(1.0e9, 50.0), rect),
            Some((egui::pos2(50.0, 50.0), egui::pos2(100.0, 50.0)))
        );
        assert_eq!(
            clip_segment(egui::pos2(-50.0, 50.0), egui::pos2(150.0, 50.0), rect),
            Some((egui::pos2(0.0, 50.0), egui::pos2(100.0, 50.0)))
        );
        assert_eq!(
            clip_segment(egui::pos2(-50.0, -10.0), egui::pos2(150.0, -10.0), rect),
            None
        );
        assert_eq!(
            clip_segment(egui::pos2(250.0, 0.0), egui::pos2(0.0, 250.0), rect),
            None,
            "a diagonal that misses the corner is outside"
        );
        assert_eq!(
            clip_segment(
                egui::pos2(f32::MAX, f32::MIN),
                egui::pos2(f32::MIN, f32::MAX),
                rect
            ),
            None
        );
        assert_eq!(
            clip_segment(egui::pos2(f32::NAN, 0.0), egui::pos2(1.0, 1.0), rect),
            None
        );
    }
}
