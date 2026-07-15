use glam::{Quat, Vec3};

use super::Camera;

impl Camera {
    /// Normalized world-space direction from the eye toward the target.
    #[must_use]
    pub fn view_direction(self) -> Vec3 {
        (self.target - self.eye()).normalize_or_zero()
    }

    /// Stable up vector for the current view.
    ///
    /// The viewer uses a free orbit: pitch is allowed to pass over the top and
    /// bottom instead of hitting a hard stop. Near the vertical axes, world Y is
    /// parallel to the view direction, so we switch to Z as the reference axis
    /// to keep the view matrix, picking rays, and panning basis valid.
    #[must_use]
    pub fn view_up(self) -> Vec3 {
        self.view_basis()
            .map_or(Vec3::Y, |(_right, up, _forward)| up)
    }

    pub(super) fn view_basis(self) -> Option<(Vec3, Vec3, Vec3)> {
        let rotation = self.resolved_orientation();
        if !rotation.is_finite() || rotation.length_squared() <= f32::EPSILON {
            return None;
        }
        let right = (rotation * Vec3::X).normalize_or_zero();
        let forward = (rotation * Vec3::NEG_Z).normalize_or_zero();
        let up = (rotation * Vec3::Y).normalize_or_zero();
        if right.length_squared() <= f32::EPSILON
            || up.length_squared() <= f32::EPSILON
            || forward.length_squared() <= f32::EPSILON
        {
            return None;
        }
        Some((right, up, forward))
    }

    pub(super) fn resolved_orientation(self) -> Quat {
        self.orientation
            .unwrap_or_else(|| orientation_from_yaw_pitch(self.yaw, self.pitch))
    }

    pub(super) fn set_yaw_pitch(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch;
        self.orientation = Some(orientation_from_yaw_pitch(yaw, pitch));
    }

    pub(super) fn sync_yaw_pitch_from_orientation(&mut self) {
        let eye_dir = (self.resolved_orientation() * Vec3::Z).normalize_or_zero();
        if eye_dir.length_squared() <= f32::EPSILON {
            return;
        }
        let reference_pitch = self.pitch;
        let (yaw, pitch) = yaw_pitch_from_direction_near(eye_dir, reference_pitch);
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Position of the eye in world space.
    #[must_use]
    pub fn eye(self) -> Vec3 {
        // Orbit: eye = target + distance * direction.
        let dir = (self.resolved_orientation() * Vec3::Z).normalize_or_zero();
        self.target + dir * self.distance
    }
}

pub(super) fn wrap_angle_rad(angle: f32) -> f32 {
    (angle + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI
}

fn orientation_from_yaw_pitch(yaw: f32, pitch: f32) -> Quat {
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-pitch)
}

fn yaw_pitch_from_direction_near(dir: Vec3, reference_pitch: f32) -> (f32, f32) {
    let yaw = wrap_angle_rad(dir.x.atan2(dir.z));
    let pitch = dir.y.clamp(-1.0, 1.0).asin();
    let alternate_pitch = wrap_angle_rad(core::f32::consts::PI - pitch);
    if angular_distance(alternate_pitch, reference_pitch) < angular_distance(pitch, reference_pitch)
    {
        (wrap_angle_rad(yaw + core::f32::consts::PI), alternate_pitch)
    } else {
        (yaw, pitch)
    }
}

fn angular_distance(a: f32, b: f32) -> f32 {
    wrap_angle_rad(a - b).abs()
}

/// Default occlusal-bias orientation quaternion (for direct GL placement if a
/// caller doesn't use the orbital camera).
#[must_use]
pub fn occlusal_orientation() -> Quat {
    Quat::from_rotation_y(0.0)
}
