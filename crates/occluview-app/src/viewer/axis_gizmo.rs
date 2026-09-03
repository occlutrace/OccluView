use eframe::egui;
use glam::Vec3;
use occluview_core::{Camera, CameraAxisView};

use crate::app_settings::ViewportBackground;

/// The projected cube is deliberately compact: it belongs to the viewport
/// corner, not to the model. The footprint is larger than the cube's maximum
/// projected extent so the cut and measure adapters can reserve one exact
/// rectangle without duplicating the projection math.
const ORIENTATION_CUBE_SCALE_PX: f32 = 48.0;
const ORIENTATION_CUBE_FOOTPRINT_PX: f32 = 96.0;
const ORIENTATION_CUBE_MARGIN_PX: f32 = 14.0;
const ORIENTATION_CUBE_EDGE_PX: f32 = 1.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OrientationCubeFace {
    pub(crate) axis: CameraAxisView,
    pub(crate) polygon: [egui::Pos2; 4],
    pub(crate) center: egui::Pos2,
    depth: f32,
}

#[derive(Clone, Copy)]
struct AxisGizmoPalette {
    face_front: egui::Color32,
    face_side: egui::Color32,
    face_hover: egui::Color32,
    edge_hover: egui::Color32,
    axis_edge: egui::Color32,
    label: egui::Color32,
}

fn axis_gizmo_palette(background: ViewportBackground) -> AxisGizmoPalette {
    if background.is_dark() {
        AxisGizmoPalette {
            face_front: egui::Color32::from_rgba_unmultiplied(44, 51, 63, 236),
            face_side: egui::Color32::from_rgba_unmultiplied(31, 38, 49, 226),
            face_hover: egui::Color32::from_rgba_unmultiplied(76, 91, 111, 246),
            edge_hover: egui::Color32::from_rgba_unmultiplied(236, 240, 246, 210),
            axis_edge: egui::Color32::from_rgb(206, 216, 230),
            label: egui::Color32::from_rgb(244, 247, 251),
        }
    } else {
        AxisGizmoPalette {
            face_front: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 242),
            face_side: egui::Color32::from_rgba_unmultiplied(231, 235, 240, 236),
            face_hover: egui::Color32::from_rgba_unmultiplied(208, 218, 230, 248),
            edge_hover: egui::Color32::from_rgba_unmultiplied(15, 23, 42, 180),
            axis_edge: egui::Color32::from_rgb(44, 55, 72),
            label: egui::Color32::from_rgb(20, 29, 43),
        }
    }
}

/// Paint the projected navigation cube and return an axis only for a primary
/// click on a face. The viewport response remains the shared input surface;
/// this painter owns no separate pointer stream.
pub(crate) fn paint_axis_gizmo(
    ui: &egui::Ui,
    image_rect: egui::Rect,
    camera: &Camera,
    response: &egui::Response,
    background: ViewportBackground,
) -> Option<CameraAxisView> {
    let faces = orientation_cube_faces(camera, image_rect);
    if faces.is_empty() {
        return None;
    }

    let hovered = response
        .hover_pos()
        .and_then(|pointer| orientation_cube_hit(&faces, pointer));
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let palette = axis_gizmo_palette(background);
    let painter = ui.painter();
    for face in &faces {
        let is_hovered = hovered == Some(face.axis);
        let fill = if is_hovered {
            palette.face_hover
        } else if face.depth >= 0.55 {
            palette.face_front
        } else {
            palette.face_side
        };
        let axis_color = axis_gizmo_color(face.axis);
        let outline = if is_hovered {
            palette.edge_hover
        } else {
            palette.axis_edge
        };
        let axis_stroke = egui::Stroke::new(
            if is_hovered {
                2.0
            } else {
                ORIENTATION_CUBE_EDGE_PX
            },
            axis_color,
        );
        painter.add(egui::Shape::convex_polygon(
            face.polygon.to_vec(),
            fill,
            egui::Stroke::new(if is_hovered { 1.5 } else { 1.0 }, outline),
        ));
        painter.line_segment([face.polygon[0], face.polygon[1]], axis_stroke);
        painter.line_segment([face.polygon[1], face.polygon[2]], axis_stroke);
        painter.line_segment([face.polygon[2], face.polygon[3]], axis_stroke);
        painter.line_segment([face.polygon[3], face.polygon[0]], axis_stroke);
        painter.text(
            face.center,
            egui::Align2::CENTER_CENTER,
            face.axis.label(),
            egui::FontId::proportional(11.5),
            if is_hovered {
                palette.label
            } else {
                palette.label.gamma_multiply(0.94)
            },
        );
    }

    if response.clicked_by(egui::PointerButton::Primary) {
        return response
            .interact_pointer_pos()
            .and_then(|pointer| orientation_cube_hit(&faces, pointer));
    }
    None
}

/// Project the camera-facing axis faces of a unit cube into the viewport.
/// Results are ordered from farthest to nearest for painter back-to-front
/// drawing. Only face normals map to camera actions; edges and corner gaps do
/// not.
pub(crate) fn orientation_cube_faces(
    camera: &Camera,
    viewport: egui::Rect,
) -> Vec<OrientationCubeFace> {
    let Some((right, up, forward)) = orientation_cube_basis(camera) else {
        return Vec::new();
    };
    let center = orientation_cube_center(viewport);
    let toward_eye = -forward;
    let mut faces = Vec::with_capacity(3);

    for axis in CameraAxisView::ALL {
        let (normal, corners) = axis_face_geometry(axis);
        let depth = normal.dot(toward_eye);
        if !depth.is_finite() || depth <= 1.0e-4 {
            continue;
        }
        let polygon = corners.map(|corner| project_cube_corner(corner, center, right, up));
        let center = polygon
            .iter()
            .copied()
            .fold(egui::Pos2::ZERO, |sum, point| sum + point.to_vec2())
            / 4.0;
        faces.push(OrientationCubeFace {
            axis,
            polygon,
            center,
            depth,
        });
    }

    faces.sort_by(|left, right| left.depth.total_cmp(&right.depth));
    faces
}

/// Return the front-most face under `pointer`, if any. The convex test keeps
/// projected corner gaps decorative and non-clickable.
pub(crate) fn orientation_cube_hit(
    faces: &[OrientationCubeFace],
    pointer: egui::Pos2,
) -> Option<CameraAxisView> {
    faces
        .iter()
        .rev()
        .find(|face| point_in_convex_quad(pointer, &face.polygon))
        .map(|face| face.axis)
}

/// The one rectangle used by painting and viewport-tool ownership.
pub(crate) fn axis_gizmo_footprint(viewport: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        orientation_cube_center(viewport),
        egui::vec2(ORIENTATION_CUBE_FOOTPRINT_PX, ORIENTATION_CUBE_FOOTPRINT_PX),
    )
}

fn orientation_cube_center(viewport: egui::Rect) -> egui::Pos2 {
    let half = ORIENTATION_CUBE_FOOTPRINT_PX * 0.5;
    let x = (viewport.right() - ORIENTATION_CUBE_MARGIN_PX - half).max(viewport.left() + half);
    let y = (viewport.top() + ORIENTATION_CUBE_MARGIN_PX + half)
        .min(viewport.bottom() - half)
        .max(viewport.top() + half);
    egui::pos2(x, y)
}

fn orientation_cube_basis(camera: &Camera) -> Option<(Vec3, Vec3, Vec3)> {
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

fn project_cube_corner(corner: Vec3, center: egui::Pos2, right: Vec3, up: Vec3) -> egui::Pos2 {
    let screen = egui::vec2(corner.dot(right), -corner.dot(up)) * ORIENTATION_CUBE_SCALE_PX;
    center + screen
}

fn axis_face_geometry(axis: CameraAxisView) -> (Vec3, [Vec3; 4]) {
    let h = 0.5;
    match axis {
        CameraAxisView::PositiveX => (
            Vec3::X,
            [
                Vec3::new(h, -h, -h),
                Vec3::new(h, h, -h),
                Vec3::new(h, h, h),
                Vec3::new(h, -h, h),
            ],
        ),
        CameraAxisView::NegativeX => (
            Vec3::NEG_X,
            [
                Vec3::new(-h, -h, h),
                Vec3::new(-h, h, h),
                Vec3::new(-h, h, -h),
                Vec3::new(-h, -h, -h),
            ],
        ),
        CameraAxisView::PositiveY => (
            Vec3::Y,
            [
                Vec3::new(-h, h, -h),
                Vec3::new(-h, h, h),
                Vec3::new(h, h, h),
                Vec3::new(h, h, -h),
            ],
        ),
        CameraAxisView::NegativeY => (
            Vec3::NEG_Y,
            [
                Vec3::new(-h, -h, h),
                Vec3::new(-h, -h, -h),
                Vec3::new(h, -h, -h),
                Vec3::new(h, -h, h),
            ],
        ),
        CameraAxisView::PositiveZ => (
            Vec3::Z,
            [
                Vec3::new(-h, -h, h),
                Vec3::new(h, -h, h),
                Vec3::new(h, h, h),
                Vec3::new(-h, h, h),
            ],
        ),
        CameraAxisView::NegativeZ => (
            Vec3::NEG_Z,
            [
                Vec3::new(h, -h, -h),
                Vec3::new(-h, -h, -h),
                Vec3::new(-h, h, -h),
                Vec3::new(h, h, -h),
            ],
        ),
    }
}

fn point_in_convex_quad(point: egui::Pos2, polygon: &[egui::Pos2; 4]) -> bool {
    let mut has_positive = false;
    let mut has_negative = false;
    for (from, to) in polygon.iter().zip(polygon.iter().cycle().skip(1)).take(4) {
        let edge = *to - *from;
        let relative = point - *from;
        let cross = edge.x * relative.y - edge.y * relative.x;
        has_positive |= cross > 1.0e-4;
        has_negative |= cross < -1.0e-4;
        if has_positive && has_negative {
            return false;
        }
    }
    true
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
    fn orientation_cube_projects_visible_faces_and_only_faces_are_clickable() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.55,
            pitch: 0.45,
            ..Camera::default()
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let faces = orientation_cube_faces(&camera, viewport);

        assert_eq!(
            faces.len(),
            3,
            "an oblique view should expose three cube faces"
        );
        for face in &faces {
            assert_eq!(orientation_cube_hit(&faces, face.center), Some(face.axis));
        }

        let footprint = axis_gizmo_footprint(viewport);
        assert!(viewport.contains_rect(footprint));
        assert!(footprint.left() > viewport.center().x);
        assert!(footprint.top() < viewport.center().y);
        assert!(
            orientation_cube_hit(&faces, footprint.left_top() + egui::vec2(2.0, 2.0)).is_none(),
            "decorative corner gaps must not snap a camera"
        );
    }

    #[test]
    fn orientation_cube_does_not_overlap_the_bottom_right_section_panel() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let panel = egui::Rect::from_min_size(egui::pos2(1240.0, 470.0), egui::vec2(352.0, 389.0));

        assert!(!axis_gizmo_footprint(viewport).intersects(panel));
    }

    #[test]
    fn orientation_cube_palette_changes_with_the_viewport_background() {
        let light = axis_gizmo_palette(ViewportBackground::White);
        let dark = axis_gizmo_palette(ViewportBackground::Dark);

        assert_ne!(light.face_front, dark.face_front);
        assert_ne!(light.axis_edge, dark.axis_edge);
    }
}
