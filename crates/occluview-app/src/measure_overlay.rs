//! Painting + chrome for the viewport measurement tools.
//!
//! Draws the world-anchored measurement overlays (ruler segments, thickness
//! probes) and the toolbar toggle buttons. All
//! anchors re-project through the live camera every frame via
//! [`project_world_to_viewport`], so measurements stay glued to the model
//! through orbit/zoom/pan.
//!
//! Depth cue: measurement chrome paints on top of the render without a depth
//! test — the simple robust option. Anchors stay readable from every angle,
//! there is no per-frame ray casting against the scene, and it mirrors the cut
//! disc + section contour, which also paint over the frame.

use eframe::egui;
use glam::Vec3;
use occluview_core::Camera;

use crate::app_settings::{RulerLineAngle, UnitDisplay};
use crate::icons::AppIcon;
use crate::measure_draw::{self, LABEL_LIFT_PX};
use crate::measure_ruler::{self, LinePlacement, RulerSegment};
use crate::measure_tool::{
    format_angle, format_length, MeasureTool, RulerAnchorRef, RulerEndpoint, ThicknessProbe,
    ThicknessReading,
};
use crate::ui_theme;
use crate::viewer::{project_world_to_viewport, viewport_ray};

const RULER_ANCHOR_GRAB_RADIUS_PX: f32 = 10.0;
/// How close to a drawn ruler line the pointer must be for a click to end the
/// pending ruler on it. Tighter than the anchor grab radius, so an end stays a
/// drag handle.
const RULER_LINE_SNAP_PX: f32 = 6.0;
/// A free angle within this of 90 degrees is drawn with the right-angle mark.
const SQUARE_TOLERANCE_DEG: f64 = 0.05;

/// Catalog key naming a [`RulerLineAngle`] choice.
pub(crate) const fn ruler_line_angle_key(angle: RulerLineAngle) -> &'static str {
    match angle {
        RulerLineAngle::Free => "measure-line-angle-free",
        RulerLineAngle::Perpendicular => "measure-line-angle-right",
    }
}

/// Paint every live measurement overlay: completed rulers, the pending anchor
/// with its rubber band to the hover position (or, over a ruler line, the
/// ruler a click would end there, as `line_angle` places it), and the
/// thickness probe.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_measurements(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    tool: &MeasureTool,
    unit: UnitDisplay,
    hover: Option<egui::Pos2>,
    line_angle: RulerLineAngle,
) {
    let segments = tool.ruler_segments();
    let rulers = RulerPainter {
        painter,
        project: |point| project_world_to_viewport(camera, viewport_rect, point).map(|(p, _)| p),
        segments: &segments,
        unit,
    };
    let preview = hover
        .and_then(|pointer| line_target(camera, viewport_rect, tool, pointer, line_angle))
        .and_then(|(base, placement)| tool.line_preview(base, placement));
    for (index, segment) in segments.iter().enumerate() {
        // A ruler ending on this ruler's line, placed or previewed: this
        // label moves to the far side so the two never overlap at the end.
        let crossing = segments
            .iter()
            .chain(&preview)
            .find(|other| other.foot.is_some_and(|foot| foot.base == index))
            .and_then(|other| (rulers.project)(other.a));
        rulers.ruler(segment, crossing);
    }
    if let Some(pending) = tool.pending_anchor() {
        if let Some(anchor) = (rulers.project)(pending) {
            match preview {
                Some(preview) => rulers.on_line(&preview, true),
                None => {
                    if let Some(hover) = hover {
                        measure_draw::rubber_band(painter, anchor, hover);
                    }
                }
            }
            measure_draw::anchor_dot(painter, anchor);
        }
    }
    if let Some(probe) = tool.probe() {
        paint_probe(painter, camera, viewport_rect, probe, unit);
    }
}

pub(crate) fn ruler_anchor_at(
    camera: &Camera,
    viewport_rect: egui::Rect,
    tool: &MeasureTool,
    pointer: egui::Pos2,
) -> Option<RulerAnchorRef> {
    let radius_sq = RULER_ANCHOR_GRAB_RADIUS_PX * RULER_ANCHOR_GRAB_RADIUS_PX;
    let mut closest: Option<(f32, RulerAnchorRef)> = None;
    for (ruler_index, segment) in tool.ruler_segments().iter().enumerate() {
        // An end on another ruler's line is a handle too: it slides along it.
        for (endpoint, point) in [(RulerEndpoint::A, segment.a), (RulerEndpoint::B, segment.b)] {
            let Some((screen, depth)) = project_world_to_viewport(camera, viewport_rect, point)
            else {
                continue;
            };
            if depth <= 0.0 {
                continue;
            }
            let distance_sq = screen.distance_sq(pointer);
            if distance_sq <= radius_sq && closest.is_none_or(|(best, _)| distance_sq < best) {
                closest = Some((
                    distance_sq,
                    RulerAnchorRef {
                        ruler_index,
                        endpoint,
                    },
                ));
            }
        }
    }
    closest.map(|(_, anchor)| anchor)
}

/// Where a click at `pointer` would end the pending ruler: on the nearest
/// drawn ruler line within [`RULER_LINE_SNAP_PX`], at the place along it under
/// the pointer or at the foot of the perpendicular, as `line_angle` says. Only
/// with an anchor pending, and never over an end handle, where a press starts
/// a drag instead. The input path and the hover preview both call this, so the
/// click places exactly what was previewed.
pub(crate) fn line_target(
    camera: &Camera,
    viewport_rect: egui::Rect,
    tool: &MeasureTool,
    pointer: egui::Pos2,
    line_angle: RulerLineAngle,
) -> Option<(usize, LinePlacement)> {
    tool.pending_anchor()?;
    if ruler_anchor_at(camera, viewport_rect, tool, pointer).is_some() {
        return None;
    }
    let base = ruler_line_under(camera, viewport_rect, tool, pointer)?;
    let base_segment = tool.ruler_segment(base)?;
    let placement = line_placement(camera, viewport_rect, &base_segment, pointer, line_angle)?;
    Some((base, placement))
}

/// How a ruler ending on `base` meets it with the pointer at `pointer`: the
/// foot of the perpendicular, or the place on the line nearest the pointer's
/// ray (exact under perspective). `None` when the line is seen end-on and no
/// place along it is under the pointer.
pub(crate) fn line_placement(
    camera: &Camera,
    viewport_rect: egui::Rect,
    base: &RulerSegment,
    pointer: egui::Pos2,
    line_angle: RulerLineAngle,
) -> Option<LinePlacement> {
    match line_angle {
        RulerLineAngle::Perpendicular => Some(LinePlacement::Perpendicular),
        RulerLineAngle::Free => {
            let (origin, direction) = viewport_ray(camera, viewport_rect, pointer)?;
            measure_ruler::line_place_under_ray(base.a, base.b, origin, direction)
                .map(LinePlacement::At)
        }
    }
}

/// The drawn ruler line within [`RULER_LINE_SNAP_PX`] of `pointer`, nearest
/// first.
fn ruler_line_under(
    camera: &Camera,
    viewport_rect: egui::Rect,
    tool: &MeasureTool,
    pointer: egui::Pos2,
) -> Option<usize> {
    let mut closest: Option<(f32, usize)> = None;
    for (index, segment) in tool.ruler_segments().iter().enumerate() {
        let a = project_world_to_viewport(camera, viewport_rect, segment.a);
        let b = project_world_to_viewport(camera, viewport_rect, segment.b);
        let (Some((a, depth_a)), Some((b, depth_b))) = (a, b) else {
            continue;
        };
        if depth_a <= 0.0 || depth_b <= 0.0 {
            continue;
        }
        let distance = distance_to_segment(pointer, a, b);
        if distance <= RULER_LINE_SNAP_PX && closest.is_none_or(|(best, _)| distance < best) {
            closest = Some((distance, index));
        }
    }
    closest.map(|(_, index)| index)
}

fn distance_to_segment(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let along = b - a;
    let length_sq = along.length_sq();
    let t = if length_sq > f32::EPSILON {
        ((point - a).dot(along) / length_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(a + along * t)
}

/// A ruler ending on a line, projected: its two ends and its base's two ends.
#[derive(Clone, Copy)]
struct OnScreen {
    start: egui::Pos2,
    end: egui::Pos2,
    base_a: egui::Pos2,
    base_b: egui::Pos2,
}

/// What every ruler needs to paint itself: the painter, the world-to-screen
/// projection, every resolved ruler (a ruler on a line reads its base) and the
/// display unit.
struct RulerPainter<'a, P> {
    painter: &'a egui::Painter,
    project: P,
    segments: &'a [RulerSegment],
    unit: UnitDisplay,
}

impl<P: Fn(Vec3) -> Option<egui::Pos2>> RulerPainter<'_, P> {
    /// One completed ruler. A free segment: halo underlay + accent line,
    /// endpoint dots, and the `NN.NN mm` chip lifted off the midpoint (beside
    /// it, on the far side from `crossing`, when a ruler ends on its line).
    fn ruler(&self, segment: &RulerSegment, crossing: Option<egui::Pos2>) {
        if segment.foot.is_some() {
            self.on_line(segment, false);
            return;
        }
        let (Some(a), Some(b)) = ((self.project)(segment.a), (self.project)(segment.b)) else {
            return;
        };
        measure_draw::segment(self.painter, a, b);
        measure_draw::anchor_dot(self.painter, a);
        measure_draw::anchor_dot(self.painter, b);
        let label = format_length(segment.distance_mm(), self.unit);
        if crossing.is_some() {
            measure_draw::label_chip_beside(self.painter, a, b, &label, crossing);
            return;
        }
        #[expect(
            clippy::manual_midpoint,
            reason = "preserve established label-pixel output"
        )]
        let mid = egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5 - LABEL_LIFT_PX);
        measure_draw::label_chip(self.painter, mid, &label, ui_theme::text());
    }

    /// A ruler ending on another ruler's line: the segment from the picked
    /// start to its end, a dashed continuation of the base line when the end
    /// falls beyond the base's ends, the angle at the end, and the length
    /// beside the segment. A perpendicular (or a free angle within
    /// [`SQUARE_TOLERANCE_DEG`] of 90) gets the right-angle mark; any other
    /// angle gets an arc over the smaller angle and its value in degrees. A
    /// `preview` draws the segment dashed, as the rubber band does, and its
    /// end as a bare dot; a placed end is a handle.
    fn on_line(&self, ruler: &RulerSegment, preview: bool) {
        let painter = self.painter;
        let project = &self.project;
        let (Some(start), Some(end)) = (project(ruler.a), project(ruler.b)) else {
            return;
        };
        let base = ruler
            .foot
            .and_then(|foot| Some((foot, self.segments.get(foot.base)?)));
        let mut angle_label = None;
        if let Some((foot, base)) = base {
            if let (Some(base_a), Some(base_b)) = (project(base.a), project(base.b)) {
                if foot.t < 0.0 {
                    measure_draw::extension(painter, base_a, end);
                } else if foot.t > 1.0 {
                    measure_draw::extension(painter, base_b, end);
                }
                let screen = OnScreen {
                    start,
                    end,
                    base_a,
                    base_b,
                };
                angle_label = self.angle_at_end(ruler, foot, base, screen);
            }
        }
        if preview {
            measure_draw::rubber_band(painter, start, end);
            measure_draw::accent_dot(painter, end);
        } else {
            measure_draw::segment(painter, start, end);
            measure_draw::anchor_dot(painter, start);
            measure_draw::anchor_dot(painter, end);
        }
        // The length goes on the other side of the segment from the angle.
        measure_draw::label_chip_beside(
            painter,
            start,
            end,
            &format_length(ruler.distance_mm(), self.unit),
            angle_label,
        );
    }

    /// The angle mark where `ruler` meets its base line, on screen at `end`.
    /// Returns where the angle's value was labelled, if it was.
    fn angle_at_end(
        &self,
        ruler: &RulerSegment,
        foot: measure_ruler::RulerFoot,
        base: &RulerSegment,
        screen: OnScreen,
    ) -> Option<egui::Pos2> {
        let OnScreen {
            start,
            end,
            base_a,
            base_b,
        } = screen;
        let painter = self.painter;
        let square = foot.perpendicular
            || foot
                .angle_deg
                .is_some_and(|angle| (angle - 90.0).abs() <= SQUARE_TOLERANCE_DEG);
        let mut along = base_b - base_a;
        if square {
            // Turn the mark toward the base's middle, where the line is drawn.
            if (base_a.lerp(base_b, 0.5) - end).dot(along) < 0.0 {
                along = -along;
            }
            measure_draw::right_angle_mark(painter, end, along, start - end);
            if foot.perpendicular {
                return None;
            }
        } else if (ruler.a - ruler.b).dot(base.b - base.a) < 0.0 {
            // The smaller angle opens toward the half of the line the ruler
            // leans over.
            along = -along;
        }
        let angle = foot.angle_deg?;
        let label_at = if square {
            measure_draw::angle_label_anchor(end, along, start - end)
        } else {
            measure_draw::angle_arc(painter, end, along, start - end)
        }?;
        measure_draw::label_chip(painter, label_at, &format_angle(angle), ui_theme::text());
        Some(label_at)
    }
}

/// The thickness probe: entry marker, the wall chord to the exit (when one
/// exists), and an honest label ("open" when there is no opposite wall).
fn paint_probe(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    probe: &ThicknessProbe,
    unit: UnitDisplay,
) {
    let Some((entry, _)) = project_world_to_viewport(camera, viewport_rect, probe.entry) else {
        return;
    };
    let label_anchor = egui::pos2(entry.x, entry.y - LABEL_LIFT_PX);
    match probe.reading {
        ThicknessReading::Wall { exit, thickness_mm } => {
            if let Some((exit_px, _)) = project_world_to_viewport(camera, viewport_rect, exit) {
                measure_draw::segment(painter, entry, exit_px);
                measure_draw::accent_dot(painter, exit_px);
            }
            measure_draw::anchor_dot(painter, entry);
            measure_draw::label_chip(
                painter,
                label_anchor,
                &format_length(f64::from(thickness_mm), unit),
                ui_theme::text(),
            );
        }
        ThicknessReading::Open => {
            measure_draw::anchor_dot(painter, entry);
            measure_draw::label_chip(
                painter,
                label_anchor,
                "open: no opposite wall",
                ui_theme::text_weak(),
            );
        }
    }
}

pub(crate) struct ToolbarToggle<'a> {
    icon: AppIcon,
    label: &'a str,
    enabled: bool,
    active: bool,
    tooltip: &'a str,
}

impl<'a> ToolbarToggle<'a> {
    pub(crate) const fn new(
        icon: AppIcon,
        label: &'a str,
        enabled: bool,
        active: bool,
        tooltip: &'a str,
    ) -> Self {
        Self {
            icon,
            label,
            enabled,
            active,
            tooltip,
        }
    }
}

/// Compact toolbar toggle with the same active treatment as the tool cells.
pub(crate) fn toolbar_toggle(ui: &mut egui::Ui, control: ToolbarToggle<'_>) -> egui::Response {
    let ToolbarToggle {
        icon,
        label,
        enabled,
        active,
        tooltip,
    } = control;
    let ink = if !enabled {
        ui.visuals().weak_text_color()
    } else if active {
        ui_theme::accent()
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };
    let font = egui::FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), ink);
    let icon_side = 15.0;
    let close_width = if active { 17.0 } else { 0.0 };
    let size = egui::vec2(
        7.0 + icon_side + 5.0 + galley.size().x + 8.0 + close_width,
        if active { 26.0 } else { 22.0 },
    );
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 4.0, ui_theme::accent().gamma_multiply(0.16));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0_f32, ui_theme::accent().gamma_multiply(0.75)),
            egui::StrokeKind::Middle,
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 4.0, ui_theme::accent().gamma_multiply(0.10));
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 7.0 + icon_side * 0.5, rect.center().y),
        egui::Vec2::splat(icon_side),
    );
    crate::icons::paint(painter, icon_rect, icon, ink);
    painter.galley(
        egui::pos2(
            icon_rect.right() + 5.0,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        ink,
    );
    if active {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::vec2(10.0, 10.0),
        );
        crate::icons::paint(painter, close_rect, AppIcon::Close, ink);
    }
    response
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::panic
    )]
    use super::*;
    use crate::measure_tool::MeasureMode;
    use crate::viewer::viewport_ray;
    use glam::Vec3;
    use occluview_core::CameraProjection;

    fn camera() -> Camera {
        Camera {
            target: Vec3::ZERO,
            orbit_pivot: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.4,
            pitch: 0.2,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 80.0,
            fovy: 45.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        }
    }

    /// The same ruler invariant dental CAD software follows: a world anchor
    /// projects onto the SAME model point after any orbit — the pixel through
    /// which it projects always rays back through the anchor.
    #[test]
    fn world_anchor_reprojects_onto_the_same_model_point_across_orbits() {
        let anchor = Vec3::new(3.0, -2.0, 4.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
        let mut cam = camera();
        let mut screens = Vec::new();
        for _ in 0..4 {
            cam.orbit_view_by(0.35, -0.15);
            let (screen, depth) =
                project_world_to_viewport(&cam, rect, anchor).expect("anchor projects");
            assert!(depth > 0.0, "anchor stays in front of the camera");
            let (origin, direction) = viewport_ray(&cam, rect, screen).expect("ray builds");
            let closest = origin + direction * (anchor - origin).dot(direction);
            assert!(
                closest.distance(anchor) < 1.0e-3,
                "projected pixel must ray back through the anchor"
            );
            screens.push(screen);
        }
        assert!(
            screens.windows(2).any(|pair| pair[0] != pair[1]),
            "orbiting must actually move the projection (test is not vacuous)"
        );
    }

    #[test]
    fn ruler_anchor_hit_test_selects_the_nearest_endpoint() {
        let camera = camera();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        tool.place_ruler_point(Vec3::ZERO);
        tool.place_ruler_point(Vec3::new(4.0, 0.0, 0.0));
        let (screen_b, _) = project_world_to_viewport(&camera, rect, Vec3::new(4.0, 0.0, 0.0))
            .expect("endpoint projects");

        assert_eq!(
            ruler_anchor_at(&camera, rect, &tool, screen_b + egui::vec2(2.0, 1.0)),
            Some(RulerAnchorRef {
                ruler_index: 0,
                endpoint: RulerEndpoint::B,
            })
        );
        assert!(ruler_anchor_at(&camera, rect, &tool, egui::pos2(4.0, 4.0)).is_none());
    }

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0))
    }

    fn screen(point: Vec3) -> egui::Pos2 {
        project_world_to_viewport(&camera(), rect(), point)
            .expect("point projects")
            .0
    }

    /// A transverse ruler and a pending incisal anchor, as in the Korkhaus
    /// measurement.
    fn tool_with_pending_over_a_ruler() -> MeasureTool {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        tool.place_ruler_point(Vec3::new(-10.0, 0.0, 0.0));
        tool.place_ruler_point(Vec3::new(10.0, 0.0, 0.0));
        tool.place_ruler_point(Vec3::new(1.0, 12.0, 3.0));
        tool
    }

    #[test]
    fn a_click_near_a_ruler_line_targets_it_only_with_an_anchor_pending() {
        let camera = camera();
        let tool = tool_with_pending_over_a_ruler();
        let on_line = screen(Vec3::new(4.0, 0.0, 0.0));
        let normal = (screen(Vec3::new(10.0, 0.0, 0.0)) - screen(Vec3::new(-10.0, 0.0, 0.0)))
            .normalized()
            .rot90();
        let beside_line = on_line + normal * 4.0;
        assert_eq!(
            line_target(
                &camera,
                rect(),
                &tool,
                beside_line,
                RulerLineAngle::Perpendicular
            ),
            Some((0, LinePlacement::Perpendicular))
        );
        let off_line = on_line + normal * 20.0;
        assert_eq!(
            line_target(&camera, rect(), &tool, off_line, RulerLineAngle::Free),
            None
        );

        let mut completed = tool_with_pending_over_a_ruler();
        completed.clear_measurements();
        completed.place_ruler_point(Vec3::new(-10.0, 0.0, 0.0));
        completed.place_ruler_point(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(
            line_target(&camera, rect(), &completed, on_line, RulerLineAngle::Free),
            None,
            "without a pending anchor a click on the line is an ordinary pick"
        );
    }

    #[test]
    fn an_end_handle_is_a_drag_target_not_a_line_target() {
        let camera = camera();
        let tool = tool_with_pending_over_a_ruler();
        let end = screen(Vec3::new(10.0, 0.0, 0.0));
        assert!(ruler_anchor_at(&camera, rect(), &tool, end).is_some());
        for angle in RulerLineAngle::OPTIONS {
            assert_eq!(line_target(&camera, rect(), &tool, end, angle), None);
        }
    }

    /// With any angle allowed, the end goes to the place on the line under the
    /// pointer, wherever along the line that is, and the click lands there.
    #[test]
    fn with_any_angle_the_end_follows_the_pointer_along_the_line() {
        let camera = camera();
        for x in [-5.0_f32, -1.5, 3.0, 6.0] {
            let mut tool = tool_with_pending_over_a_ruler();
            let pointer = screen(Vec3::new(x, 0.0, 0.0));
            let (base, placement) =
                line_target(&camera, rect(), &tool, pointer, RulerLineAngle::Free)
                    .expect("over the line");
            assert_eq!(base, 0);
            let LinePlacement::At(t) = placement else {
                panic!("any angle places along the line, not at the foot");
            };
            assert!((t - (x + 10.0) / 20.0).abs() < 1.0e-4, "x {x}: t {t}");
            tool.place_on_line(base, placement).expect("placed");
            let placed = tool.ruler_segment(1).expect("second ruler");
            assert!(placed.b.distance(Vec3::new(x, 0.0, 0.0)) < 1.0e-3);
            let foot = placed.foot.expect("on the line");
            assert!(!foot.perpendicular);
            let a = Vec3::new(1.0, 12.0, 3.0) - placed.b;
            let expected = f64::from(a.x.abs() / a.length()).acos().to_degrees();
            let angle = foot.angle_deg.expect("an angle");
            assert!(
                (angle - expected).abs() < 1.0e-2,
                "x {x}: {angle} vs {expected}"
            );
        }
    }

    #[test]
    fn an_end_on_a_line_is_a_drag_handle() {
        let camera = camera();
        let mut tool = tool_with_pending_over_a_ruler();
        tool.place_on_line(0, LinePlacement::Perpendicular)
            .expect("perpendicular placed");
        let segment = tool.ruler_segments()[1];
        assert_eq!(
            ruler_anchor_at(&camera, rect(), &tool, screen(segment.b)),
            Some(RulerAnchorRef {
                ruler_index: 1,
                endpoint: RulerEndpoint::B,
            })
        );
        assert_eq!(
            ruler_anchor_at(&camera, rect(), &tool, screen(segment.a)),
            Some(RulerAnchorRef {
                ruler_index: 1,
                endpoint: RulerEndpoint::A,
            })
        );
    }

    #[test]
    fn painting_every_overlay_state_does_not_panic() {
        // Real test `Ui`: the label chips lay out text, which needs the font
        // atlas a bare `Context::debug_painter` does not have yet.
        egui::__run_test_ui(|ui| {
            let painter = ui.painter();
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
            let mut tool = MeasureTool::default();
            tool.arm(MeasureMode::Ruler);
            tool.place_ruler_point(Vec3::ZERO);
            tool.place_ruler_point(Vec3::new(5.0, 5.0, 0.0));
            tool.place_ruler_point(Vec3::new(1.0, 2.0, 3.0)); // pending anchor
            tool.set_probe(ThicknessProbe {
                entry: Vec3::new(2.0, 0.0, 0.0),
                reading: ThicknessReading::Wall {
                    exit: Vec3::new(1.0, 0.0, 0.0),
                    thickness_mm: 1.0,
                },
            });
            paint_measurements(
                painter,
                &camera(),
                rect,
                &tool,
                UnitDisplay::Millimeters,
                Some(egui::pos2(100.0, 100.0)),
                RulerLineAngle::Free,
            );
            tool.set_probe(ThicknessProbe {
                entry: Vec3::new(2.0, 0.0, 0.0),
                reading: ThicknessReading::Open,
            });
            paint_measurements(
                painter,
                &camera(),
                rect,
                &tool,
                UnitDisplay::Millimeters,
                None,
                RulerLineAngle::Free,
            );
            // A pending anchor hovering a ruler line previews the ruler it
            // would place, in either choice; placed ones whose end falls beyond
            // the base draw the extension, and a free angle of exactly 90 or 0
            // degrees, or a zero-length ruler on the line, paints too.
            let mut on_lines = tool_with_pending_over_a_ruler();
            let hover = screen(Vec3::new(4.0, 0.0, 0.0));
            for angle in RulerLineAngle::OPTIONS {
                paint_measurements(
                    painter,
                    &camera(),
                    rect,
                    &on_lines,
                    UnitDisplay::Millimeters,
                    Some(hover),
                    angle,
                );
            }
            on_lines.place_on_line(0, LinePlacement::Perpendicular);
            for (start, placement) in [
                (Vec3::new(30.0, 5.0, 0.0), LinePlacement::Perpendicular),
                (Vec3::new(10.0, 0.0, 0.0), LinePlacement::Perpendicular),
                (Vec3::new(3.0, 8.0, 0.0), LinePlacement::At(0.2)),
                (Vec3::new(3.0, 8.0, 0.0), LinePlacement::At(1.8)),
                (Vec3::new(2.0, 8.0, 0.0), LinePlacement::At(0.6)),
                (Vec3::new(-20.0, 0.0, 0.0), LinePlacement::At(0.5)),
                (Vec3::new(0.0, 0.0, 0.0), LinePlacement::At(0.5)),
            ] {
                on_lines.place_ruler_point(start);
                on_lines.place_on_line(0, placement);
            }
            paint_measurements(
                painter,
                &camera(),
                rect,
                &on_lines,
                UnitDisplay::Inches,
                None,
                RulerLineAngle::Free,
            );
            // Zero-length ruler (same point twice) labels 0.00 mm, never NaN.
            tool.clear_measurements();
            tool.place_ruler_point(Vec3::X);
            tool.place_ruler_point(Vec3::X);
            paint_measurements(
                painter,
                &camera(),
                rect,
                &tool,
                UnitDisplay::Millimeters,
                None,
                RulerLineAngle::Free,
            );
        });
    }

    #[test]
    fn toolbar_toggle_renders_in_every_state() {
        egui::__run_test_ui(|ui| {
            for icon in [AppIcon::Ruler, AppIcon::Thickness] {
                for enabled in [false, true] {
                    for active in [false, true] {
                        let _ = toolbar_toggle(
                            ui,
                            ToolbarToggle::new(icon, "Label", enabled, active, "tooltip"),
                        );
                    }
                }
            }
        });
    }
}
