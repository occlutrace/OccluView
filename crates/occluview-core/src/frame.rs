//! Coordinate-frame conventions and conversions.
//!
//! OccluView's internal frame is **right-handed, Y-up**. Each file
//! format's loader converts its native frame to this one on read, so the rest of
//! the codebase can assume a single frame. This module centralizes the
//! conventions and the small conversion helpers.

use glam::{Affine3A, Quat, Vec3};

/// OccluView's canonical up axis.
pub const UP: Vec3 = Vec3::Y;

/// OccluView's canonical forward axis (into the screen, away from camera).
pub const FORWARD: Vec3 = Vec3::NEG_Z;

/// OccluView's canonical right axis.
pub const RIGHT: Vec3 = Vec3::X;

/// A format's declared coordinate frame, used to drive load-time conversion.
///
/// This enumerates only the frames we actually encounter in dental files; it is
/// not a general CG frame taxonomy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SourceFrame {
    /// Right-handed, Y-up — already matches OccluView's frame. OBJ, glTF.
    RightHandedYUp,
    /// Right-handed, Z-up — 3MF (and some CAD exports). Swap Y and Z.
    RightHandedZUp,
    /// Left-handed, Y-up — some game formats.
    LeftHandedYUp,
    /// Unknown / scanner-dependent; load as-is and let the user rotate.
    Unknown,
}

impl SourceFrame {
    /// Return the transform that maps `self` into OccluView's canonical frame.
    ///
    /// For `Unknown`, this is identity — the user is expected to orient the mesh
    /// in the UI. We never guess a frame silently for a format that declares one.
    #[must_use]
    #[inline]
    pub fn to_canonical(self) -> Affine3A {
        match self {
            SourceFrame::RightHandedYUp | SourceFrame::Unknown => Affine3A::IDENTITY,
            // Z-up -> Y-up: rotate -90° around X. Maps +Z(old up) -> +Y(new up).
            SourceFrame::RightHandedZUp => {
                Affine3A::from_quat(Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2))
            }
            // Left-handed Y-up -> right-handed Y-up: mirror X.
            SourceFrame::LeftHandedYUp => {
                Affine3A::IDENTITY * Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn y_up_frame_is_identity() {
        assert_eq!(
            SourceFrame::RightHandedYUp.to_canonical(),
            Affine3A::IDENTITY
        );
    }

    #[test]
    fn z_up_to_y_up_maps_old_z_axis_to_new_y() {
        let xform = SourceFrame::RightHandedZUp.to_canonical();
        // A point that was "up" (0,0,1) in Z-up should map to (0,1,0) in Y-up.
        let mapped = xform.transform_point3(Vec3::new(0.0, 0.0, 1.0));
        assert!((mapped - Vec3::Y).length() < 1e-5, "got {mapped}");
    }

    #[test]
    fn unknown_is_passthrough() {
        assert_eq!(SourceFrame::Unknown.to_canonical(), Affine3A::IDENTITY);
    }

    #[test]
    fn left_handed_mirrors_x() {
        let xform = SourceFrame::LeftHandedYUp.to_canonical();
        let mapped = xform.transform_point3(Vec3::X);
        assert!(
            (mapped - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5,
            "got {mapped}"
        );
    }
}
