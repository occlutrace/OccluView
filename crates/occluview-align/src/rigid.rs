//! The rigid pose shared by every registration stage.
//!
//! Kept in `f64`: vertices are `f32`, and composing many single-precision
//! transforms drifts. A session's pose is a single [`Rigid`], converted to
//! [`Affine3A`] only for rendering and baked into geometry only at export, so
//! repeated fits accumulate no error.

use glam::{Affine3A, DMat3, DQuat, DVec3, Mat3, Vec3};

/// How far a basis column may stray from unit length, and how far two columns
/// may stray from perpendicular, before [`Rigid::from_affine`] refuses the
/// transform as non-rigid.
const ORTHONORMAL_TOLERANCE: f32 = 1e-3;

/// A rotation followed by a translation. No scale, ever.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    /// Unit rotation quaternion.
    pub rotation: DQuat,
    /// Translation applied after the rotation, in millimetres.
    pub translation: DVec3,
}

impl Default for Rigid {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Rigid {
    /// The pose that changes nothing.
    pub const IDENTITY: Self = Self {
        rotation: DQuat::IDENTITY,
        translation: DVec3::ZERO,
    };

    /// Build a pose, normalizing the rotation so repeated composition cannot
    /// let the quaternion drift off the unit sphere.
    #[must_use]
    pub fn new(rotation: DQuat, translation: DVec3) -> Self {
        Self {
            rotation: rotation.normalize(),
            translation,
        }
    }

    /// Map a point from the source frame into the target frame.
    #[must_use]
    pub fn apply(&self, point: DVec3) -> DVec3 {
        self.rotation * point + self.translation
    }

    /// Rotate a direction. Translation does not apply to directions.
    #[must_use]
    pub fn apply_normal(&self, normal: DVec3) -> DVec3 {
        self.rotation * normal
    }

    /// `self` applied after `other`.
    #[must_use]
    pub fn compose(&self, other: &Self) -> Self {
        Self::new(
            self.rotation * other.rotation,
            self.rotation * other.translation + self.translation,
        )
    }

    /// The pose that undoes this one.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.conjugate();
        Self::new(rotation, -(rotation * self.translation))
    }

    /// Whether every component is finite — the guard every stage checks before
    /// letting a fit reach the scene.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.translation.is_finite()
            && self.rotation.x.is_finite()
            && self.rotation.y.is_finite()
            && self.rotation.z.is_finite()
            && self.rotation.w.is_finite()
    }

    /// The single-precision transform the renderer and the scene graph use.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn to_affine(&self) -> Affine3A {
        let rotation = glam::Quat::from_xyzw(
            self.rotation.x as f32,
            self.rotation.y as f32,
            self.rotation.z as f32,
            self.rotation.w as f32,
        )
        .normalize();
        Affine3A::from_rotation_translation(rotation, self.translation.as_vec3())
    }

    /// Read a pose back from a scene transform.
    ///
    /// Returns `None` when the transform carries scale, shear, a mirror, or a
    /// non-finite value. This crate never silently absorbs a non-rigid
    /// transform: dental scans are metric, and a scaled layer means the caller
    /// has a unit problem to report, not a pose to fit.
    #[must_use]
    pub fn from_affine(transform: &Affine3A) -> Option<Self> {
        let basis: Mat3 = transform.matrix3.into();
        let columns = [basis.x_axis, basis.y_axis, basis.z_axis];
        for column in columns {
            if !column.is_finite() || (column.length() - 1.0).abs() > ORTHONORMAL_TOLERANCE {
                return None;
            }
        }
        for (left, right) in [(0, 1), (1, 2), (0, 2)] {
            if columns[left].dot(columns[right]).abs() > ORTHONORMAL_TOLERANCE {
                return None;
            }
        }
        if basis.determinant() <= 0.0 {
            return None;
        }
        let translation: Vec3 = transform.translation.into();
        if !translation.is_finite() {
            return None;
        }
        let rotation = DMat3::from_cols(
            columns[0].as_dvec3(),
            columns[1].as_dvec3(),
            columns[2].as_dvec3(),
        );
        Some(Self::new(
            DQuat::from_mat3(&rotation),
            translation.as_dvec3(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Rigid;
    use glam::{DQuat, DVec3};

    fn sample() -> Rigid {
        Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.3, -0.5, 0.8).normalize(), 0.73),
            DVec3::new(1.5, -2.25, 3.125),
        )
    }

    #[test]
    fn applying_the_inverse_returns_the_original_point() {
        let pose = sample();
        let point = DVec3::new(0.31, 0.42, -0.53);
        let back = pose.inverse().apply(pose.apply(point));
        assert!(
            (back - point).length() < 1e-12,
            "round trip drifted: {back:?}"
        );
    }

    #[test]
    fn composing_with_the_inverse_is_the_identity() {
        let pose = sample();
        let identity = pose.compose(&pose.inverse());
        let point = DVec3::new(-4.0, 7.0, 0.5);
        assert!((identity.apply(point) - point).length() < 1e-12);
    }

    #[test]
    fn compose_matches_sequential_application() {
        let first = sample();
        let second = Rigid::new(
            DQuat::from_axis_angle(DVec3::X, -0.4),
            DVec3::new(-1.0, 2.0, 0.0),
        );
        let point = DVec3::new(2.0, -3.0, 4.0);
        let composed = second.compose(&first).apply(point);
        let stepwise = second.apply(first.apply(point));
        assert!((composed - stepwise).length() < 1e-12);
    }

    #[test]
    fn a_normal_is_rotated_but_not_translated() {
        let pose = sample();
        let normal = DVec3::new(0.0, 0.0, 1.0);
        let rotated = pose.apply_normal(normal);
        assert!((rotated.length() - 1.0).abs() < 1e-12);
        assert!((rotated - (pose.apply(normal) - pose.apply(DVec3::ZERO))).length() < 1e-12);
    }

    #[test]
    fn affine_round_trip_survives_f32_precision() {
        let pose = sample();
        let back = Rigid::from_affine(&pose.to_affine());
        assert!(
            back.is_some(),
            "a rigid pose must convert back from its affine"
        );
        if let Some(back) = back {
            let point = DVec3::new(5.0, -6.0, 7.0);
            assert!((back.apply(point) - pose.apply(point)).length() < 1e-4);
        }
    }

    #[test]
    fn from_affine_rejects_a_scaled_transform() {
        let scaled = glam::Affine3A::from_scale(glam::Vec3::splat(2.0));
        assert!(Rigid::from_affine(&scaled).is_none());
    }

    #[test]
    fn a_non_finite_pose_is_reported() {
        let broken = Rigid::new(DQuat::IDENTITY, DVec3::new(f64::NAN, 0.0, 0.0));
        assert!(!broken.is_finite());
    }
}
