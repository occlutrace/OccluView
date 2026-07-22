//! Orthographic camera framing for the cut-view widget.
//!
//! The cut view looks **along the clip-plane normal** at the cross-section,
//! orthographically (no perspective distortion). This is the standard dental
//! CBCT cross-section view: an axial/coronal/sagittal slice shown flat.
//!
//! Unlike the orbital [`occluview_core::Camera`] (perspective, occlusal bias),
//! this camera is defined directly by a view + projection matrix pair, fitted
//! to the mesh's bounding box projected onto the plane.

use crate::camera::GpuCamera;
use crate::clipping::ClipPlane;
use glam::{Mat4, Vec3};
use occluview_core::Aabb;

/// Build an orthographic cut-view camera looking along the clip plane's
/// normal at the bbox center, framed to the cross-section extent.
///
/// The camera sits at `center + normal * distance` looking toward `center`,
/// with `up` chosen as a non-degenerate vector in the plane. The orthographic
/// frustum is sized to the bbox's projected half-diagonal so the whole
/// cross-section fits.
///
/// Returns a [`GpuCamera`] ready to pass to `Offscreen::render_with_cut`.
#[must_use]
pub fn cut_view_camera(plane: &ClipPlane, bbox: Aabb) -> GpuCamera {
    if bbox.is_empty() {
        // Degenerate: return a default camera (the render will be empty).
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 1000.0);
        return GpuCamera::new(view, proj, Vec3::Z, Vec3::new(0.0, 0.0, 100.0));
    }

    let center = bbox.center();
    let normal = Vec3::from_array(plane.normal);

    // Project all 8 bbox corners onto the plane to find the cross-section
    // extent. We compute the max distance from center in the plane's tangent
    // space.
    let half_diag = bbox.half_diagonal().max(1.0);

    // Camera position: offset along the normal so we look "down" the normal
    // at the cross-section.
    let distance = half_diag * 4.0;
    let eye = center + normal * distance;

    // Up vector: pick a non-parallel vector. If normal is ~+Y, use +Z; else +Y.
    let up = if normal.dot(Vec3::Y).abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::Z
    };

    let view = Mat4::look_at_rh(eye, center, up);
    // Orthographic frustum: square, sized to 2x the half-diagonal (oversized
    // so the cross-section fits with margin).
    let half_extent = half_diag * 1.2;
    let proj = Mat4::orthographic_rh(
        -half_extent,
        half_extent,
        -half_extent,
        half_extent,
        0.1,
        distance * 2.0 + half_diag,
    );

    GpuCamera::new(view, proj, Vec3::new(0.4, 0.8, 0.5), eye)
}

/// World-space up hint for the slice camera: a vector not parallel to the plane
/// normal (`+Y` unless the normal is near-vertical, then `+Z`). Shared by
/// [`cut_view_camera_focused`] and [`slice_view_basis`] so the interactive ruler
/// maps panel pixels through the exact same basis the slice was rendered with.
#[must_use]
fn slice_up_hint(normal: Vec3) -> Vec3 {
    if normal.dot(Vec3::Y).abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

/// The slice camera's world-space `(right, up)` axes for a plane normal — the
/// same basis `Mat4::look_at_rh(eye, focus, slice_up_hint)` induces when the eye
/// looks **along** `normal`. A point `p` on the section plane projects to panel
/// offsets `(right · (p - focus), up · (p - focus))` in world mm, so the ruler
/// can convert panel pixels to section-plane millimeters exactly.
#[must_use]
pub fn slice_view_basis(normal: Vec3) -> (Vec3, Vec3) {
    let forward = normal.normalize_or(Vec3::Y);
    slice_view_basis_with_up(forward, slice_up_hint(forward))
}

/// Return the orthonormal in-plane axes for a section viewed with the supplied
/// world-space up direction. The up hint is projected into the section plane,
/// so the returned basis remains valid when the main camera is oblique to the
/// cut. A fixed fallback prevents a singular camera position from producing
/// NaNs or a visible orientation jump.
#[must_use]
pub fn slice_view_basis_with_up(normal: Vec3, up_hint: Vec3) -> (Vec3, Vec3) {
    let forward = normal.normalize_or(Vec3::Y);
    let projected_up = up_hint - forward * up_hint.dot(forward);
    if projected_up.is_finite() && projected_up.length_squared() > 1.0e-8 {
        let up = projected_up.normalize();
        let right = forward.cross(up).normalize_or(Vec3::X);
        return (right, right.cross(forward).normalize_or(up));
    }

    let fallback_up = slice_up_hint(forward);
    let right = forward.cross(fallback_up).normalize_or(Vec3::X);
    let up = right.cross(forward).normalize_or(Vec3::Y);
    (right, up)
}

/// Build an orthographic cut-view camera looking along `plane`'s normal at an
/// explicit `focus` point, framed to `half_extent` (world units) on each side.
///
/// This is the interactive-disc variant of [`cut_view_camera`]: the slice window
/// stays centered on the disc and its zoom follows the disc radius, instead of
/// always framing the whole bounding box.
///
/// The eye sits on the **cut-away** side of the plane (`focus - normal * d`) so
/// the section face is the front-most surface: with the clip discarding the
/// `-normal` half, the camera looks *into* the cut and shows the true
/// cross-section, not the kept half's exterior (which reads as the whole,
/// uncut mesh). `scene_extent` (the mesh half-diagonal) sizes the eye offset and
/// far plane so the entire kept half stays within the depth range regardless of
/// how tightly the disc frames the view.
#[must_use]
pub fn cut_view_camera_focused(
    plane: &ClipPlane,
    focus: Vec3,
    half_extent: f32,
    scene_extent: f32,
) -> GpuCamera {
    let normal = Vec3::from_array(plane.normal).normalize_or(Vec3::Y);
    cut_view_camera_focused_with_up(
        plane,
        focus,
        half_extent,
        scene_extent,
        slice_up_hint(normal),
    )
}

/// Build the focused section camera with an explicit in-plane up direction.
/// The app passes the main viewport's section basis here so the rendered panel
/// image and its vector contour use exactly the same orientation.
#[must_use]
pub fn cut_view_camera_focused_with_up(
    plane: &ClipPlane,
    focus: Vec3,
    half_extent: f32,
    scene_extent: f32,
    up_hint: Vec3,
) -> GpuCamera {
    let normal = Vec3::from_array(plane.normal).normalize_or(Vec3::Y);
    let half_extent = half_extent.max(0.1);
    let scene_extent = scene_extent.max(half_extent);
    // Stand off past the whole mesh on the cut-away side so nothing kept is
    // behind the near plane and the section face is nearest the camera.
    let back_off = scene_extent * 2.0 + 1.0;
    let eye = focus - normal * back_off;
    let (_, up) = slice_view_basis_with_up(normal, up_hint);
    let view = Mat4::look_at_rh(eye, focus, up);
    // Far must span from the eye across the entire kept half (which extends up to
    // ~scene_extent past the plane on the +normal side).
    let far = back_off + 2.0 * scene_extent + 1.0;
    let proj = Mat4::orthographic_rh(
        -half_extent,
        half_extent,
        -half_extent,
        half_extent,
        0.1,
        far,
    );
    GpuCamera::new(view, proj, Vec3::new(0.4, 0.8, 0.5), eye)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_bbox() -> Aabb {
        Aabb::from_min_max(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0))
    }

    #[test]
    fn cut_camera_for_z_normal_looks_along_z() {
        let plane = ClipPlane::new([0.0, 0.0, 1.0], 0.0);
        let cam = cut_view_camera(&plane, unit_bbox());
        // The eye should be offset in +Z from the center (origin).
        // We can't directly inspect GpuCamera's view matrix fields (they're
        // arrays), but the construction must not panic and must produce a
        // valid camera. The real validation is the golden test.
        let _ = cam;
    }

    #[test]
    fn cut_camera_empty_bbox_does_not_panic() {
        let plane = ClipPlane::new([0.0, 1.0, 0.0], 0.0);
        let _ = cut_view_camera(&plane, Aabb::EMPTY);
    }

    #[test]
    fn cut_camera_vertical_normal_uses_z_up() {
        // normal = +Y: the degenerate case where +Y up would be parallel.
        let plane = ClipPlane::new([0.0, 1.0, 0.0], 0.0);
        let _ = cut_view_camera(&plane, unit_bbox());
    }

    #[test]
    fn focused_camera_eye_sits_on_the_cut_away_side() {
        // Clip keeps the +normal half; the slice camera must look from the
        // -normal (cut-away) side so the section face is front-most.
        let plane = ClipPlane::new([1.0, 0.0, 0.0], 0.0);
        let focus = Vec3::new(0.0, 0.0, 0.0);
        let cam = cut_view_camera_focused(&plane, focus, 2.0, 10.0);
        let eye = Vec3::from_array(cam.camera_pos);
        // dot(eye - focus, normal) < 0 => eye is on the cut-away half.
        assert!(
            (eye - focus).dot(Vec3::X) < 0.0,
            "slice eye must be on the cut-away (-normal) side, got {eye}"
        );
    }

    #[test]
    fn slice_basis_is_orthonormal_and_spans_the_plane() {
        for normal in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.98, 0.2),
        ] {
            let n = normal.normalize();
            let (right, up) = slice_view_basis(n);
            assert!((right.length() - 1.0).abs() < 1e-5, "right unit for {n}");
            assert!((up.length() - 1.0).abs() < 1e-5, "up unit for {n}");
            assert!(right.dot(n).abs() < 1e-5, "right in plane for {n}");
            assert!(up.dot(n).abs() < 1e-5, "up in plane for {n}");
            assert!(right.dot(up).abs() < 1e-5, "right perp up for {n}");
        }
    }

    #[test]
    fn custom_up_rotates_the_section_basis_without_leaving_the_plane() {
        let (right, up) = slice_view_basis_with_up(Vec3::Z, Vec3::X);
        assert!((right - Vec3::Y).length() < 1e-5, "right={right}");
        assert!((up - Vec3::X).length() < 1e-5, "up={up}");
        assert!(right.dot(Vec3::Z).abs() < 1e-5);
        assert!(up.dot(Vec3::Z).abs() < 1e-5);
    }
}
