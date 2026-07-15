//! Painting for the interactive cut view: the section contour polylines and the
//! disc manipulator (dashed rim, center dot, rim handles).
//!
//! Everything is drawn in 2D by reprojecting world-space geometry through the
//! orthographic camera with [`project_world_to_viewport`], the same technique as
//! the axis gizmo and scale bar — no GPU work. The kernel-computed contour lives
//! in world space and is only *reprojected* each frame, so camera motion never
//! recomputes it.

use crate::cut_manipulator::DiscPose;
use crate::viewer::project_world_to_viewport;
use eframe::egui;
use glam::Vec3;
use occluview_core::scene::SceneSection;
use occluview_core::{Camera, SceneMeshId};

/// White halo drawn under every stroke (scale-bar style).
const HALO: egui::Color32 = egui::Color32::from_rgba_premultiplied(214, 219, 224, 168);
/// Slate used for the disc and as the darkening target for contour tints.
const SLATE: egui::Color32 = egui::Color32::from_rgb(15, 23, 42);
/// Halo width (px) under the 2px contour / disc strokes.
const HALO_WIDTH: f32 = 4.0;
/// Main contour / disc stroke width (px).
const STROKE_WIDTH: f32 = 2.0;
/// Samples around the disc rim for the projected outline.
const DISC_SEGMENTS: u16 = 72;

/// Paint the world-space section contour of every intersected layer.
pub(crate) fn paint_section_contour(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    section: &SceneSection,
    color_for: impl Fn(SceneMeshId) -> egui::Color32,
) {
    for layer in &section.per_layer {
        let color = color_for(layer.layer_id);
        for polyline in &layer.polylines {
            let mut screen: Vec<egui::Pos2> = Vec::with_capacity(polyline.points.len() + 1);
            for point in &polyline.points {
                let world = point.as_vec3();
                match project_world_to_viewport(camera, rect, world) {
                    Some((pos, depth)) if depth > 0.0 => screen.push(pos),
                    // A vertex behind the camera breaks the run into a new one.
                    _ => {
                        draw_polyline(painter, &screen, false, color);
                        screen.clear();
                    }
                }
            }
            draw_polyline(painter, &screen, polyline.closed, color);
        }
    }
}

/// Draw one projected polyline: white halo under a colored stroke.
fn draw_polyline(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    closed: bool,
    color: egui::Color32,
) {
    if points.len() < 2 {
        return;
    }
    let mut pts = points.to_vec();
    if closed {
        pts.push(pts[0]);
    }
    painter.add(egui::Shape::line(
        pts.clone(),
        egui::Stroke::new(HALO_WIDTH, HALO),
    ));
    painter.add(egui::Shape::line(
        pts,
        egui::Stroke::new(STROKE_WIDTH, color),
    ));
}

/// Paint the disc manipulator: dashed rim, translucent center, and — when
/// planted — the four rim handles.
pub(crate) fn paint_disc(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    pose: DiscPose,
    planted: bool,
) {
    let (u, v) = plane_basis(pose.plane_normal);
    let mut rim: Vec<egui::Pos2> = Vec::with_capacity(usize::from(DISC_SEGMENTS) + 1);
    for i in 0..DISC_SEGMENTS {
        let theta = std::f32::consts::TAU * f32::from(i) / f32::from(DISC_SEGMENTS);
        let world = pose.center + (u * theta.cos() + v * theta.sin()) * pose.radius_mm;
        match project_world_to_viewport(camera, rect, world) {
            Some((pos, depth)) if depth > 0.0 => rim.push(pos),
            _ => return, // any rim vertex behind the camera: skip the whole disc
        }
    }
    rim.push(rim[0]);

    // Dashed outline over a solid white halo so it reads on any background.
    painter.add(egui::Shape::line(
        rim.clone(),
        egui::Stroke::new(HALO_WIDTH, HALO),
    ));
    painter.extend(egui::Shape::dashed_line(
        &rim,
        egui::Stroke::new(STROKE_WIDTH, SLATE),
        7.0,
        5.0,
    ));

    if let Some((center, depth)) = project_world_to_viewport(camera, rect, pose.center) {
        if depth > 0.0 {
            painter.circle_filled(
                center,
                5.0,
                egui::Color32::from_rgba_unmultiplied(15, 23, 42, 51),
            );
            painter.circle_stroke(center, 5.0, egui::Stroke::new(1.5, SLATE));
        }
    }

    if planted {
        for i in 0u16..4 {
            let theta = std::f32::consts::FRAC_PI_2 * f32::from(i);
            let world = pose.center + (u * theta.cos() + v * theta.sin()) * pose.radius_mm;
            if let Some((pos, depth)) = project_world_to_viewport(camera, rect, world) {
                if depth > 0.0 {
                    painter.circle_filled(pos, 4.5, HALO);
                    painter.circle_filled(pos, 3.0, SLATE);
                }
            }
        }
    }
}

/// An orthonormal basis `(u, v)` spanning the plane with the given normal.
fn plane_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalize_or(Vec3::Y);
    let seed = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = (seed - n * seed.dot(n)).normalize_or(Vec3::X);
    let v = n.cross(u).normalize_or(Vec3::Z);
    (u, v)
}

/// Darken a linear-RGBA layer tint toward slate for the contour stroke, keeping
/// enough of the layer's hue to disambiguate multiple layers.
pub(crate) fn contour_color(tint_linear: [f32; 4]) -> egui::Color32 {
    // egui does the linear -> sRGB (u8) conversion for us.
    let srgb = egui::Color32::from(egui::Rgba::from_rgb(
        tint_linear[0],
        tint_linear[1],
        tint_linear[2],
    ));
    let mix = |channel: u8, slate: u8| -> u8 {
        u8::try_from((u16::from(channel) + u16::from(slate) * 2) / 3).unwrap_or(u8::MAX)
    };
    egui::Color32::from_rgb(mix(srgb.r(), 15), mix(srgb.g(), 23), mix(srgb.b(), 42))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_basis_is_orthonormal_and_in_plane() {
        for normal in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
            let n = normal.normalize();
            let (u, v) = plane_basis(n);
            assert!((u.length() - 1.0).abs() < 1e-5, "u unit");
            assert!((v.length() - 1.0).abs() < 1e-5, "v unit");
            assert!(u.dot(n).abs() < 1e-5, "u in plane");
            assert!(v.dot(n).abs() < 1e-5, "v in plane");
            assert!(u.dot(v).abs() < 1e-5, "u perp v");
        }
    }

    #[test]
    fn contour_color_darkens_toward_slate() {
        // Pure white tint should come back well below full white (mixed 1:2
        // toward slate) but still bright.
        let c = contour_color([1.0, 1.0, 1.0, 1.0]);
        assert!(c.r() < 200 && c.r() > 80, "r={}", c.r());
        // A red-tinted layer keeps a red bias.
        let red = contour_color([1.0, 0.0, 0.0, 1.0]);
        assert!(red.r() > red.g() && red.r() > red.b());
    }
}
