//! Moving a scan by hand, in the frame the operator is looking at.
//!
//! The viewport camera is orthographic, so a pixel maps to a fixed number of
//! millimetres regardless of depth: the conversion here is exact, not an
//! approximation that drifts as the operator zooms.

use eframe::egui;
use glam::{Quat, Vec3};

/// Degrees of rotation per pixel of drag. Slow enough that a small correction
/// stays small, fast enough that a half-turn does not need three gestures.
pub(crate) const DEGREES_PER_PIXEL: f32 = 0.35;

/// Which directions a hand drag is allowed to move in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DragConstraint {
    /// Move in any direction.
    #[default]
    Free,
    /// Move only along the world Z axis.
    ZOnly,
    /// Move only within the world XY plane.
    XyPlane,
}

impl DragConstraint {
    /// The label the panel shows.
    ///
    /// Named for the direction a hand moves, not for the axis letter. An
    /// operator dragging a scan is thinking "lift it", not "constrain to
    /// world Z".
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Free => "Any way",
            Self::ZOnly => "Up · down",
            Self::XyPlane => "Sideways",
        }
    }

    /// What the constraint does, in one line.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Free => "Drag the scan in any direction",
            Self::ZOnly => "Drag only along the vertical axis",
            Self::XyPlane => "Drag only across the horizontal plane",
        }
    }
}

/// Drop the components a constraint forbids.
pub(crate) fn constrain_translation(delta: Vec3, constraint: DragConstraint) -> Vec3 {
    match constraint {
        DragConstraint::Free => delta,
        DragConstraint::ZOnly => Vec3::new(0.0, 0.0, delta.z),
        DragConstraint::XyPlane => Vec3::new(delta.x, delta.y, 0.0),
    }
}

/// Convert a screen drag into a world translation across the view plane.
///
/// `world_per_pixel` is the orthographic height over the viewport height. The
/// screen y axis points down and the camera's up axis points up, hence the
/// negation.
pub(crate) fn screen_delta_to_world(
    delta_px: egui::Vec2,
    camera_right: Vec3,
    camera_up: Vec3,
    world_per_pixel: f32,
) -> Vec3 {
    if !world_per_pixel.is_finite() || world_per_pixel <= 0.0 {
        return Vec3::ZERO;
    }
    camera_right * (delta_px.x * world_per_pixel) - camera_up * (delta_px.y * world_per_pixel)
}

/// Convert a screen drag into a rotation about the camera's own axes.
///
/// Horizontal drag turns about the camera's up axis and vertical drag about
/// its right axis, which is what makes the scan appear to follow the pointer
/// rather than spinning about some world axis the operator cannot see.
pub(crate) fn rotation_from_drag(
    delta_px: egui::Vec2,
    camera_right: Vec3,
    camera_up: Vec3,
    degrees_per_pixel: f32,
) -> Quat {
    let yaw = (delta_px.x * degrees_per_pixel).to_radians();
    let pitch = (delta_px.y * degrees_per_pixel).to_radians();
    let up = camera_up.normalize_or_zero();
    let right = camera_right.normalize_or_zero();
    if up.length_squared() <= 0.0 || right.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    (Quat::from_axis_angle(up, yaw) * Quat::from_axis_angle(right, pitch)).normalize()
}

#[cfg(test)]
mod tests {
    use super::{constrain_translation, rotation_from_drag, screen_delta_to_world, DragConstraint};
    use eframe::egui;
    use glam::{Quat, Vec3};

    #[test]
    fn free_movement_passes_the_delta_through() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(constrain_translation(delta, DragConstraint::Free), delta);
    }

    #[test]
    fn z_only_keeps_the_vertical_component() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(
            constrain_translation(delta, DragConstraint::ZOnly),
            Vec3::new(0.0, 0.0, 3.0)
        );
    }

    #[test]
    fn the_xy_plane_drops_the_vertical_component() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(
            constrain_translation(delta, DragConstraint::XyPlane),
            Vec3::new(1.0, -2.0, 0.0)
        );
    }

    #[test]
    fn a_screen_drag_moves_the_scan_the_way_the_pointer_went() {
        // Ten pixels right and four pixels down, at a tenth of a millimetre
        // per pixel: one millimetre along the camera's right axis and 0.4 mm
        // *down*, because screen y grows downward.
        let world = screen_delta_to_world(egui::vec2(10.0, 4.0), Vec3::X, Vec3::Y, 0.1);
        assert!((world.x - 1.0).abs() < 1e-6, "{world:?}");
        assert!((world.y + 0.4).abs() < 1e-6, "{world:?}");
    }

    #[test]
    fn a_zero_or_broken_scale_moves_nothing() {
        assert_eq!(
            screen_delta_to_world(egui::vec2(50.0, 50.0), Vec3::X, Vec3::Y, 0.0),
            Vec3::ZERO
        );
        assert_eq!(
            screen_delta_to_world(egui::vec2(50.0, 50.0), Vec3::X, Vec3::Y, f32::NAN),
            Vec3::ZERO
        );
    }

    #[test]
    fn an_empty_drag_produces_no_rotation() {
        let rotation = rotation_from_drag(egui::Vec2::ZERO, Vec3::X, Vec3::Y, 0.5);
        assert!(rotation.to_axis_angle().1.abs() < 1e-6);
    }

    #[test]
    fn a_horizontal_drag_turns_about_the_camera_up_axis() {
        let rotation = rotation_from_drag(egui::vec2(90.0, 0.0), Vec3::X, Vec3::Y, 1.0);
        let (axis, angle) = rotation.to_axis_angle();
        assert!(axis.dot(Vec3::Y).abs() > 0.99, "axis was {axis:?}");
        assert!((angle.to_degrees() - 90.0).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn a_vertical_drag_turns_about_the_camera_right_axis() {
        let rotation = rotation_from_drag(egui::vec2(0.0, 45.0), Vec3::X, Vec3::Y, 1.0);
        let (axis, angle) = rotation.to_axis_angle();
        assert!(axis.dot(Vec3::X).abs() > 0.99, "axis was {axis:?}");
        assert!((angle.to_degrees() - 45.0).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn a_rotation_is_always_a_unit_quaternion() {
        let rotation = rotation_from_drag(egui::vec2(37.0, -21.0), Vec3::X, Vec3::Y, 0.4);
        assert!((rotation.length() - 1.0).abs() < 1e-5);
        assert_ne!(rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_degenerate_camera_basis_rotates_nothing() {
        let rotation = rotation_from_drag(egui::vec2(30.0, 30.0), Vec3::ZERO, Vec3::Y, 1.0);
        assert_eq!(rotation, Quat::IDENTITY);
    }
}
