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
use occluview_core::Camera;

use crate::measure_draw::{self, LABEL_LIFT_PX};
use crate::measure_tool::{
    format_mm, MeasureTool, RulerAnchorRef, RulerEndpoint, RulerMeasurement, ThicknessProbe,
    ThicknessReading,
};
use crate::mesh_editor_icons::{self, MeasureIcon};
use crate::ui_theme;
use crate::viewer::project_world_to_viewport;

const RULER_ANCHOR_GRAB_RADIUS_PX: f32 = 10.0;

/// Paint every live measurement overlay: completed ruler segments, the pending
/// anchor with its rubber band to the hover position, and the thickness probe.
pub(crate) fn paint_measurements(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    tool: &MeasureTool,
    hover: Option<egui::Pos2>,
) {
    for ruler in tool.rulers() {
        paint_ruler(painter, camera, viewport_rect, ruler);
    }
    if let Some(pending) = tool.pending_anchor() {
        if let Some((anchor, _)) = project_world_to_viewport(camera, viewport_rect, pending) {
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
    if let Some(probe) = tool.probe() {
        paint_probe(painter, camera, viewport_rect, probe);
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
    for (ruler_index, ruler) in tool.rulers().iter().enumerate() {
        for (endpoint, point) in [(RulerEndpoint::A, ruler.a), (RulerEndpoint::B, ruler.b)] {
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

/// One completed segment: halo underlay + accent line, endpoint dots, and the
/// `NN.NN mm` chip lifted off the midpoint.
fn paint_ruler(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    ruler: &RulerMeasurement,
) {
    let a = project_world_to_viewport(camera, viewport_rect, ruler.a);
    let b = project_world_to_viewport(camera, viewport_rect, ruler.b);
    let (Some((a, _)), Some((b, _))) = (a, b) else {
        return;
    };
    measure_draw::segment(painter, a, b);
    measure_draw::anchor_dot(painter, a);
    measure_draw::anchor_dot(painter, b);
    let mid = egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5 - LABEL_LIFT_PX);
    measure_draw::label_chip(
        painter,
        mid,
        &format_mm(ruler.distance_mm()),
        ui_theme::TEXT,
    );
}

/// The thickness probe: entry marker, the wall chord to the exit (when one
/// exists), and an honest label ("open" when there is no opposite wall).
fn paint_probe(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    probe: &ThicknessProbe,
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
                &format_mm(f64::from(thickness_mm)),
                ui_theme::TEXT,
            );
        }
        ThicknessReading::Open => {
            measure_draw::anchor_dot(painter, entry);
            measure_draw::label_chip(
                painter,
                label_anchor,
                "open: no opposite wall",
                ui_theme::TEXT_WEAK,
            );
        }
    }
}

/// A compact toolbar toggle: hand-painted vector glyph + caption, with the
/// mesh-editor cell visuals (accent wash + ring while the tool is engaged).
// A toggle genuinely needs its glyph, caption, both state flags, and tooltip;
// bundling them into a struct would only add ceremony (same call shape as
// `mesh_editor_icons::icon_button`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn toolbar_toggle(
    ui: &mut egui::Ui,
    icon: MeasureIcon,
    label: &str,
    enabled: bool,
    active: bool,
    tooltip: &str,
) -> bool {
    let ink = if !enabled {
        ui.visuals().weak_text_color()
    } else if active {
        ui_theme::ACCENT
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
        painter.rect_filled(rect, 4.0, ui_theme::ACCENT.gamma_multiply(0.16));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, ui_theme::ACCENT.gamma_multiply(0.75)),
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 4.0, ui_theme::ACCENT.gamma_multiply(0.10));
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 7.0 + icon_side * 0.5, rect.center().y),
        egui::Vec2::splat(icon_side),
    );
    mesh_editor_icons::paint_measure(painter, icon_rect, icon, ink, active);
    painter.galley(
        egui::pos2(
            icon_rect.right() + 5.0,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        ink,
    );
    if active {
        let x = rect.right() - 8.0;
        let center = egui::pos2(x, rect.center().y);
        let arm = 3.2;
        let stroke = egui::Stroke::new(1.35, ink);
        painter.line_segment(
            [
                center + egui::vec2(-arm, -arm),
                center + egui::vec2(arm, arm),
            ],
            stroke,
        );
        painter.line_segment(
            [
                center + egui::vec2(-arm, arm),
                center + egui::vec2(arm, -arm),
            ],
            stroke,
        );
    }
    response
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
        .clicked()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
    use super::*;
    use crate::measure_tool::MeasureMode;
    use crate::viewer::viewport_ray;
    use glam::Vec3;
    use occluview_core::CameraProjection;

    fn camera() -> Camera {
        Camera {
            target: Vec3::ZERO,
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

    /// The exocad-ruler invariant: a world anchor projects onto the SAME model
    /// point after any orbit — the pixel through which it projects always rays
    /// back through the anchor.
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
                Some(egui::pos2(100.0, 100.0)),
            );
            tool.set_probe(ThicknessProbe {
                entry: Vec3::new(2.0, 0.0, 0.0),
                reading: ThicknessReading::Open,
            });
            paint_measurements(painter, &camera(), rect, &tool, None);
            // Zero-length ruler (same point twice) labels 0.00 mm, never NaN.
            tool.clear_measurements();
            tool.place_ruler_point(Vec3::X);
            tool.place_ruler_point(Vec3::X);
            paint_measurements(painter, &camera(), rect, &tool, None);
        });
    }

    #[test]
    fn toolbar_toggle_renders_in_every_state() {
        egui::__run_test_ui(|ui| {
            for icon in [MeasureIcon::Ruler, MeasureIcon::Thickness] {
                for enabled in [false, true] {
                    for active in [false, true] {
                        toolbar_toggle(ui, icon, "Label", enabled, active, "tooltip");
                    }
                }
            }
        });
    }
}
