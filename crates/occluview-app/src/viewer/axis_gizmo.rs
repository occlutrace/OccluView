use eframe::egui;
use glam::Vec3;
use occluview_core::{Camera, CameraAxisView};

const AXIS_GIZMO_RADIUS_PX: f32 = 24.0;
const AXIS_GIZMO_MARKER_RADIUS_PX: f32 = 9.0;
const AXIS_GIZMO_MARGIN_PX: f32 = 16.0;
/// Backing-halo radius drawn under the gizmo ring (the `+ 10.0` disc below).
const AXIS_GIZMO_GLOW_PX: f32 = 10.0;
/// Vertical room the lifted gizmo needs above the Section panel: its margin
/// plus the full glow circle. The panel's size budget reserves this so the
/// lifted gizmo can never be pushed above the viewport or into the cut strip.
pub(crate) const AXIS_GIZMO_LIFT_RESERVE_PX: f32 =
    AXIS_GIZMO_MARGIN_PX + 2.0 * (AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX);

#[derive(Clone, Copy)]
struct AxisGizmoMarker {
    axis: CameraAxisView,
    center: egui::Pos2,
    depth: f32,
}

/// Paint the navigation gizmo. `avoid` (the docked Section panel, when the cut
/// tool is active) lifts the gizmo to sit just ABOVE that rect instead of its
/// default bottom-right home, so the panel can own the bottom-right corner.
pub(crate) fn paint_axis_gizmo(
    ui: &egui::Ui,
    image_rect: egui::Rect,
    camera: &Camera,
    response: &egui::Response,
    avoid: Option<egui::Rect>,
) -> Option<CameraAxisView> {
    let markers = axis_gizmo_markers(camera, image_rect, avoid);
    let painter = ui.painter();
    let center = axis_gizmo_center(image_rect, avoid);
    painter.circle_filled(
        center,
        AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX,
        egui::Color32::from_rgba_unmultiplied(248, 250, 252, 210),
    );
    painter.circle_stroke(
        center,
        AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX,
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(15, 23, 42, 42)),
    );

    for (negative, positive, color) in [
        (
            CameraAxisView::NegativeX,
            CameraAxisView::PositiveX,
            egui::Color32::from_rgb(224, 92, 92),
        ),
        (
            CameraAxisView::NegativeY,
            CameraAxisView::PositiveY,
            egui::Color32::from_rgb(112, 198, 120),
        ),
        (
            CameraAxisView::NegativeZ,
            CameraAxisView::PositiveZ,
            egui::Color32::from_rgb(96, 150, 234),
        ),
    ] {
        let Some(negative_center) = axis_gizmo_marker_center(&markers, negative) else {
            continue;
        };
        let Some(positive_center) = axis_gizmo_marker_center(&markers, positive) else {
            continue;
        };
        painter.line_segment(
            [negative_center, positive_center],
            egui::Stroke::new(1.5, color),
        );
    }

    let mut ordered = markers.clone();
    ordered.sort_by(|left, right| left.depth.total_cmp(&right.depth));
    for marker in ordered {
        let color = axis_gizmo_color(marker.axis);
        let is_positive = matches!(
            marker.axis,
            CameraAxisView::PositiveX | CameraAxisView::PositiveY | CameraAxisView::PositiveZ
        );
        let fill = if is_positive {
            color
        } else {
            egui::Color32::from_rgba_unmultiplied(248, 250, 252, 235)
        };
        painter.circle_filled(marker.center, AXIS_GIZMO_MARKER_RADIUS_PX, fill);
        painter.circle_stroke(
            marker.center,
            AXIS_GIZMO_MARKER_RADIUS_PX,
            egui::Stroke::new(1.5, color),
        );
        painter.text(
            marker.center,
            egui::Align2::CENTER_CENTER,
            axis_gizmo_marker_label(marker.axis),
            egui::FontId::proportional(11.0),
            if is_positive {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(15, 23, 42)
            },
        );
    }

    if response.clicked_by(egui::PointerButton::Primary) {
        return response
            .interact_pointer_pos()
            .and_then(|pointer| axis_gizmo_snap_target(&markers, pointer));
    }
    None
}

fn axis_gizmo_markers(
    camera: &Camera,
    image_rect: egui::Rect,
    avoid: Option<egui::Rect>,
) -> Vec<AxisGizmoMarker> {
    let (right, up, forward) = axis_gizmo_basis(camera);
    let center = axis_gizmo_center(image_rect, avoid);
    CameraAxisView::ALL
        .into_iter()
        .map(|axis| {
            let direction = axis.direction();
            let projected = egui::vec2(direction.dot(right), -direction.dot(up));
            AxisGizmoMarker {
                axis,
                center: center + projected * AXIS_GIZMO_RADIUS_PX,
                depth: -direction.dot(forward),
            }
        })
        .collect()
}

/// The gizmo's full interactive footprint (ring + outer markers + glow) for a
/// viewport. Input adapters treat this as chrome: a click on an axis marker
/// must snap the camera, never double as a disc plant or a measure anchor.
pub(crate) fn axis_gizmo_footprint(
    image_rect: egui::Rect,
    avoid: Option<egui::Rect>,
) -> egui::Rect {
    let center = axis_gizmo_center(image_rect, avoid);
    // The glow disc (ring radius + glow) is the outermost paint: markers sit ON
    // the ring, so their reach (ring + marker radius) stays inside it.
    let reach = AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX;
    egui::Rect::from_center_size(center, egui::vec2(reach * 2.0, reach * 2.0))
}

fn axis_gizmo_center(image_rect: egui::Rect, avoid: Option<egui::Rect>) -> egui::Pos2 {
    // Same right-aligned column in both states (no horizontal jump); only the
    // vertical anchor moves: default sits in the bottom-right corner, the
    // avoid case lifts straight up to just above the Section panel.
    let x = image_rect.right() - AXIS_GIZMO_MARGIN_PX - AXIS_GIZMO_RADIUS_PX;
    let y = match avoid {
        Some(panel) => {
            let lifted =
                panel.top() - AXIS_GIZMO_MARGIN_PX - (AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX);
            // Defense in depth: the panel's size budget already reserves the
            // lift room, but the gizmo must never leave the viewport even if
            // a caller hands it a taller rect to avoid.
            lifted.max(image_rect.top() + AXIS_GIZMO_MARGIN_PX + AXIS_GIZMO_RADIUS_PX)
        }
        None => image_rect.bottom() - AXIS_GIZMO_MARGIN_PX - AXIS_GIZMO_RADIUS_PX,
    };
    egui::pos2(x, y)
}

fn axis_gizmo_basis(camera: &Camera) -> (Vec3, Vec3, Vec3) {
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    (right, up, forward)
}

fn axis_gizmo_marker_center(
    markers: &[AxisGizmoMarker],
    axis: CameraAxisView,
) -> Option<egui::Pos2> {
    markers
        .iter()
        .find(|marker| marker.axis == axis)
        .map(|marker| marker.center)
}

fn axis_gizmo_snap_target(
    markers: &[AxisGizmoMarker],
    pointer: egui::Pos2,
) -> Option<CameraAxisView> {
    markers
        .iter()
        .filter(|marker| marker.center.distance(pointer) <= AXIS_GIZMO_MARKER_RADIUS_PX + 2.0)
        .max_by(|left, right| left.depth.total_cmp(&right.depth))
        .map(|marker| marker.axis)
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

fn axis_gizmo_marker_label(axis: CameraAxisView) -> &'static str {
    match axis {
        CameraAxisView::PositiveX | CameraAxisView::NegativeX => "X",
        CameraAxisView::PositiveY | CameraAxisView::NegativeY => "Y",
        CameraAxisView::PositiveZ | CameraAxisView::NegativeZ => "Z",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_gizmo_markers_project_front_view_axes() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));

        let markers = axis_gizmo_markers(&camera, rect, None);
        let center = axis_gizmo_center(rect, None);
        let positive_x = markers
            .iter()
            .find(|marker| marker.axis == CameraAxisView::PositiveX);
        let positive_y = markers
            .iter()
            .find(|marker| marker.axis == CameraAxisView::PositiveY);
        assert!(positive_x.is_some(), "missing +X marker");
        assert!(positive_y.is_some(), "missing +Y marker");
        let Some(positive_x) = positive_x else {
            return;
        };
        let Some(positive_y) = positive_y else {
            return;
        };

        assert!(
            positive_x.center.x > center.x,
            "marker={:?}",
            positive_x.center
        );
        assert!(
            positive_y.center.y < center.y,
            "marker={:?}",
            positive_y.center
        );
    }

    #[test]
    fn gizmo_lifts_clear_above_the_section_panel_when_avoiding() {
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let default_center = axis_gizmo_center(vp, None);
        // Default: bottom-right corner.
        assert!(default_center.x > vp.center().x && default_center.y > vp.center().y);

        // A docked Section panel in the bottom-right: the gizmo lifts to sit
        // entirely ABOVE it (glow included) with no horizontal jump.
        let panel = egui::Rect::from_min_size(egui::pos2(1240.0, 470.0), egui::vec2(352.0, 389.0));
        let lifted = axis_gizmo_center(vp, Some(panel));
        assert!(
            (lifted.x - default_center.x).abs() < f32::EPSILON,
            "the gizmo keeps its right-aligned column (no horizontal jump)"
        );
        assert!(
            lifted.y + AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX <= panel.top(),
            "the gizmo (with glow) must clear the panel top: gizmo_bottom={}, panel_top={}",
            lifted.y + AXIS_GIZMO_RADIUS_PX + AXIS_GIZMO_GLOW_PX,
            panel.top()
        );
        assert!(
            lifted.x >= panel.left() && lifted.x <= panel.right(),
            "lifted gizmo stays horizontally over the panel it sits above"
        );
    }

    #[test]
    fn axis_gizmo_hit_test_uses_marker_center() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
        let markers = axis_gizmo_markers(&camera, rect, None);
        let Some(marker) = markers
            .iter()
            .find(|marker| marker.axis == CameraAxisView::PositiveX)
        else {
            return;
        };

        assert_eq!(
            axis_gizmo_snap_target(&markers, marker.center),
            Some(CameraAxisView::PositiveX)
        );
    }
}
