use glam::{Quat, Vec2, Vec3};

use super::{orientation::wrap_angle_rad, Camera};

impl Camera {
    /// Orbit around the current target by yaw/pitch deltas in radians.
    pub fn orbit_by(&mut self, yaw_delta: f32, pitch_delta: f32) {
        let mut yaw = self.yaw;
        let mut pitch = self.pitch;
        if yaw_delta.is_finite() {
            yaw = wrap_angle_rad(yaw + yaw_delta);
        }
        if pitch_delta.is_finite() {
            pitch = wrap_angle_rad(pitch + pitch_delta);
        }
        self.set_yaw_pitch(yaw, pitch);
    }

    /// Orbit using the current viewport axes instead of fixed world axes.
    ///
    /// This keeps horizontal drags effective from top/bottom views where a
    /// world-Y yaw degenerates into gimbal-like behavior.
    pub fn orbit_view_by(&mut self, yaw_delta: f32, pitch_delta: f32) {
        if !yaw_delta.is_finite() || !pitch_delta.is_finite() {
            return;
        }

        let Some((right, up, _forward)) = self.view_basis() else {
            return;
        };
        let offset = self.eye() - self.target;
        let radius = offset.length();
        if !radius.is_finite() || radius <= f32::EPSILON {
            return;
        }

        let pitch_rotation = Quat::from_axis_angle(right, -pitch_delta);
        let yaw_rotation = Quat::from_axis_angle(up, yaw_delta);
        let orientation = self.resolved_orientation();
        let next_orientation = (yaw_rotation * pitch_rotation * orientation).normalize();
        if !next_orientation.is_finite() || next_orientation.length_squared() <= f32::EPSILON {
            return;
        }

        self.orientation = Some(next_orientation);
        self.sync_yaw_pitch_from_orientation();
    }

    /// Arcball orbit from one normalized viewport point to another.
    ///
    /// Points are in screen space normalized to roughly `[-1, 1]`, with +Y up.
    /// Unlike yaw/pitch orbiting, this has no vertical pole or clamp: crossing
    /// the top/bottom of the model remains a continuous rotation.
    pub fn orbit_trackball(&mut self, from_ndc: Vec2, to_ndc: Vec2) {
        if !from_ndc.is_finite() || !to_ndc.is_finite() {
            return;
        }

        let from = trackball_project(from_ndc);
        let to = trackball_project(to_ndc);
        let local_delta = Quat::from_rotation_arc(from, to);
        if !local_delta.is_finite() || local_delta.length_squared() <= f32::EPSILON {
            return;
        }

        let orientation = self.resolved_orientation();
        if !orientation.is_finite() || orientation.length_squared() <= f32::EPSILON {
            return;
        }
        let world_delta = orientation * local_delta * orientation.inverse();
        let next_orientation = (world_delta * orientation).normalize();
        if !next_orientation.is_finite() || next_orientation.length_squared() <= f32::EPSILON {
            return;
        }

        self.orientation = Some(next_orientation);
        self.sync_yaw_pitch_from_orientation();
    }
}

fn trackball_project(point: Vec2) -> Vec3 {
    let clamped = point.clamp(Vec2::splat(-1.35), Vec2::splat(1.35));
    let len_sq = clamped.length_squared();
    if len_sq <= 1.0 {
        Vec3::new(clamped.x, clamped.y, (1.0 - len_sq).sqrt()).normalize_or_zero()
    } else {
        Vec3::new(clamped.x, clamped.y, 0.0).normalize_or_zero()
    }
}
