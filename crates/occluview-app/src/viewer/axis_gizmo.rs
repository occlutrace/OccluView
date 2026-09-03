use eframe::egui;
use glam::Vec3;
use occluview_core::{Camera, CameraAxisView};

use crate::app_settings::ViewportBackground;

/// The gizmo is a small projected axis triad, not a second toolbar. Its
/// footprint is deliberately larger than the painted arms so pointer routing
/// and painting share one stable rectangle.
const AXIS_GIZMO_ARM_PX: f32 = 36.0;
const AXIS_GIZMO_FOOTPRINT_PX: f32 = 96.0;
const AXIS_GIZMO_MARGIN_PX: f32 = 14.0;
const AXIS_GIZMO_GAP_PX: f32 = 8.0;
const AXIS_GIZMO_HIT_RADIUS_PX: f32 = 10.0;
const AXIS_GIZMO_ARROW_PX: f32 = 7.0;

/// Vertical room needed when the bottom-right gizmo lifts above the docked
/// Section panel: viewport margin + full footprint + a breathing gap.
pub(crate) const AXIS_GIZMO_LIFT_RESERVE_PX: f32 =
    AXIS_GIZMO_MARGIN_PX + AXIS_GIZMO_FOOTPRINT_PX + AXIS_GIZMO_GAP_PX;

#[derive(Clone, Copy, Debug)]
struct AxisGizmoMarker {
    axis: CameraAxisView,
    endpoint: egui::Pos2,
    depth: f32,
    projected_length: f32,
}

#[derive(Clone, Copy)]
struct AxisGizmoPalette {
    shadow: egui::Color32,
    hub_fill: egui::Color32,
    hub_stroke: egui::Color32,
    label: egui::Color32,
}

fn axis_gizmo_palette(background: ViewportBackground) -> AxisGizmoPalette {
    if background.is_dark() {
        AxisGizmoPalette {
            shadow: egui::Color32::from_black_alpha(180),
            hub_fill: egui::Color32::from_rgb(226, 232, 240),
            hub_stroke: egui::Color32::from_rgb(15, 23, 42),
            label: egui::Color32::from_rgb(245, 247, 250),
        }
    } else {
        AxisGizmoPalette {
            shadow: egui::Color32::from_black_alpha(95),
            hub_fill: egui::Color32::from_rgb(15, 23, 42),
            hub_stroke: egui::Color32::from_rgb(248, 250, 252),
            label: egui::Color32::from_rgb(15, 23, 42),
        }
    }
}

/// Paint the camera-facing axis triad and return a snap target only for a
/// primary click near an axis endpoint. The viewport response remains the
/// shared input surface; this painter owns no separate pointer stream.
pub(crate) fn paint_axis_gizmo(
    ui: &egui::Ui,
    image_rect: egui::Rect,
    camera: &Camera,
    response: &egui::Response,
    avoid: Option<egui::Rect>,
    background: ViewportBackground,
) -> Option<CameraAxisView> {
    let Some((center, mut markers)) = axis_gizmo_markers(camera, image_rect, avoid) else {
        return None;
    };
    let hovered = response
        .hover_pos()
        .and_then(|pointer| axis_gizmo_hit(&markers, center, pointer));
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let palette = axis_gizmo_palette(background);
    let painter = ui.painter();
    markers.sort_by(|left, right| left.depth.total_cmp(&right.depth));
    for marker in markers.iter() {
        let delta = marker.endpoint - center;
        let length = delta.length();
        if length <= f32::EPSILON {
            continue;
        }
        let direction = delta / length;
        let is_hovered = hovered == Some(marker.axis);
        let is_positive = matches!(
            marker.axis,
            CameraAxisView::PositiveX | CameraAxisView::PositiveY | CameraAxisView::PositiveZ
        );
        let axis_color = axis_gizmo_color(marker.axis);
        let color = if is_positive {
            axis_color
        } else {
            axis_color.gamma_multiply(if background.is_dark() { 0.62 } else { 0.48 })
        };
        let width = if is_hovered {
            2.75
        } else if is_positive {
            2.0
        } else {
            1.25
        };
        painter.line_segment(
            [center, marker.endpoint],
            egui::Stroke::new(width + 1.5, palette.shadow),
        );
        painter.line_segment([center, marker.endpoint], egui::Stroke::new(width, color));

        if is_positive {
            paint_arrowhead(
                painter,
                marker.endpoint,
                direction,
                if is_hovered {
                    axis_color.gamma_multiply(1.12)
                } else {
                    axis_color
                },
                palette.shadow,
            );
        } else {
            painter.circle_filled(marker.endpoint, if is_hovered { 3.5 } else { 2.5 }, color);
        }

        // The positive ends carry the readable labels. Negative ends stay as
        // quiet dots: labeling both ends makes standard views collide when
        // two projected axes share a screen direction.
        if is_positive {
            painter.text(
                marker.endpoint + direction * 7.0,
                egui::Align2::CENTER_CENTER,
                marker.axis.label(),
                egui::FontId::proportional(if is_hovered { 11.5 } else { 10.5 }),
                if is_hovered {
                    axis_color
                } else {
                    palette.label
                },
            );
        }
    }

    painter.circle_filled(center, 4.0, palette.shadow);
    painter.circle_filled(center, 2.75, palette.hub_fill);
    painter.circle_stroke(center, 2.75, egui::Stroke::new(0.75, palette.hub_stroke));

    if response.clicked_by(egui::PointerButton::Primary) {
        return response
            .interact_pointer_pos()
            .and_then(|pointer| axis_gizmo_hit(&markers, center, pointer));
    }
    None
}

fn paint_arrowhead(
    painter: &egui::Painter,
    tip: egui::Pos2,
    direction: egui::Vec2,
    fill: egui::Color32,
    shadow: egui::Color32,
) {
    let perpendicular = egui::vec2(-direction.y, direction.x);
    let base = tip - direction * AXIS_GIZMO_ARROW_PX;
    let width = AXIS_GIZMO_ARROW_PX * 0.62;
    let triangle = vec![
        tip,
        base + perpendicular * width,
        base - perpendicular * width,
    ];
    painter.add(egui::Shape::convex_polygon(
        triangle.clone(),
        shadow,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        triangle,
        fill,
        egui::Stroke::new(0.75, shadow),
    ));
}

/// Project all six world-axis endpoints around the camera's current basis.
/// Their screen positions therefore rotate continuously with the viewport.
fn axis_gizmo_markers(
    camera: &Camera,
    viewport: egui::Rect,
    avoid: Option<egui::Rect>,
) -> Option<(egui::Pos2, Vec<AxisGizmoMarker>)> {
    let (right, up, forward) = axis_gizmo_basis(camera)?;
    let center = axis_gizmo_center(viewport, avoid);
    let eye_direction = -forward;
    let markers = CameraAxisView::ALL
        .into_iter()
        .map(|axis| {
            let direction = axis.direction();
            let projected = egui::vec2(direction.dot(right), -direction.dot(up));
            AxisGizmoMarker {
                axis,
                endpoint: center + projected * AXIS_GIZMO_ARM_PX,
                depth: direction.dot(eye_direction),
                projected_length: projected.length() * AXIS_GIZMO_ARM_PX,
            }
        })
        .collect();
    Some((center, markers))
}

/// Return the front-most axis endpoint under `pointer`. The centre and empty
/// space remain decorative, so they cannot accidentally snap the camera.
fn axis_gizmo_hit(
    markers: &[AxisGizmoMarker],
    center: egui::Pos2,
    pointer: egui::Pos2,
) -> Option<CameraAxisView> {
    markers
        .iter()
        .filter(|marker| marker.projected_length > AXIS_GIZMO_HIT_RADIUS_PX * 0.5)
        .filter(|marker| marker.endpoint.distance(pointer) <= AXIS_GIZMO_HIT_RADIUS_PX)
        .max_by(|left, right| left.depth.total_cmp(&right.depth))
        .map(|marker| marker.axis)
        .filter(|_| pointer.distance(center) > AXIS_GIZMO_HIT_RADIUS_PX)
}

/// The painted footprint when the Section panel owns the bottom-right corner.
pub(crate) fn axis_gizmo_footprint_for(
    viewport: egui::Rect,
    avoid: Option<egui::Rect>,
) -> egui::Rect {
    egui::Rect::from_center_size(
        axis_gizmo_center(viewport, avoid),
        egui::vec2(AXIS_GIZMO_FOOTPRINT_PX, AXIS_GIZMO_FOOTPRINT_PX),
    )
}

fn axis_gizmo_center(viewport: egui::Rect, avoid: Option<egui::Rect>) -> egui::Pos2 {
    let half = AXIS_GIZMO_FOOTPRINT_PX * 0.5;
    let x = (viewport.right() - AXIS_GIZMO_MARGIN_PX - half)
        .max(viewport.left() + AXIS_GIZMO_MARGIN_PX + half);
    let bottom_home = viewport.bottom() - AXIS_GIZMO_MARGIN_PX - half;
    let y = avoid.map_or(bottom_home, |panel| {
        (panel.top() - AXIS_GIZMO_GAP_PX - half)
            .clamp(viewport.top() + AXIS_GIZMO_MARGIN_PX + half, bottom_home)
    });
    egui::pos2(x, y)
}

fn axis_gizmo_basis(camera: &Camera) -> Option<(Vec3, Vec3, Vec3)> {
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    (forward.is_finite()
        && up.is_finite()
        && right.is_finite()
        && forward.length_squared() > f32::EPSILON
        && up.length_squared() > f32::EPSILON
        && right.length_squared() > f32::EPSILON)
        .then_some((right, up, forward))
}

fn axis_gizmo_color(axis: CameraAxisView) -> egui::Color32 {
    match axis {
        CameraAxisView::PositiveX | CameraAxisView::NegativeX => {
            egui::Color32::from_rgb(224, 92, 92)
        }
        CameraAxisView::PositiveY | CameraAxisView::NegativeY => {
            egui::Color32::from_rgb(112, 198, 120)
        }
        CameraAxisView::PositiveZ | CameraAxisView::NegativeZ => {
            egui::Color32::from_rgb(96, 150, 234)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_gizmo_projects_rotating_axes_and_only_endpoints_are_clickable() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.55,
            pitch: 0.45,
            ..Camera::default()
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let Some((center, markers)) = axis_gizmo_markers(&camera, viewport, None) else {
            panic!("camera basis should be valid");
        };

        assert_eq!(markers.len(), 6);
        for marker in &markers {
            assert!(marker.projected_length > 0.0);
            assert_eq!(
                axis_gizmo_hit(&markers, center, marker.endpoint),
                Some(marker.axis)
            );
        }
        assert!(axis_gizmo_hit(&markers, center, center).is_none());
    }

    #[test]
    fn axis_gizmo_lives_bottom_right_and_lifts_above_section_panel() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let home = axis_gizmo_footprint_for(viewport, None);
        assert!(home.right() < viewport.right());
        assert!(home.bottom() < viewport.bottom());
        assert!(home.left() > viewport.center().x);
        assert!(home.top() > viewport.center().y);

        let panel = egui::Rect::from_min_size(egui::pos2(1240.0, 470.0), egui::vec2(352.0, 389.0));
        let lifted = axis_gizmo_footprint_for(viewport, Some(panel));
        assert!(!lifted.intersects(panel));
        assert!(lifted.bottom() <= panel.top());
    }

    #[test]
    fn axis_gizmo_palette_changes_with_the_viewport_background() {
        let light = axis_gizmo_palette(ViewportBackground::White);
        let dark = axis_gizmo_palette(ViewportBackground::Dark);

        assert_ne!(light.hub_fill, dark.hub_fill);
        assert_ne!(light.label, dark.label);
    }
}
