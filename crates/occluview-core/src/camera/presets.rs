use crate::bbox::Aabb;

use glam::Vec3;

use super::{Camera, CameraProjection, MIN_ORTHOGRAPHIC_HEIGHT_MM};

/// Named camera presets exposed by the desktop viewer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CameraPreset {
    /// Dental default, looking onto the occlusal plane.
    Occlusal,
    /// Buccal-facing view from +Z.
    Buccal,
    /// Lingual-facing view from -Z.
    Lingual,
    /// Mesial-facing view from +X.
    Mesial,
    /// Distal-facing view from -X.
    Distal,
}

/// Exact axis-aligned views used by the viewport gizmo snap targets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CameraAxisView {
    /// Look from positive X toward the origin.
    PositiveX,
    /// Look from negative X toward the origin.
    NegativeX,
    /// Look from positive Y toward the origin.
    PositiveY,
    /// Look from negative Y toward the origin.
    NegativeY,
    /// Look from positive Z toward the origin.
    PositiveZ,
    /// Look from negative Z toward the origin.
    NegativeZ,
}

impl CameraAxisView {
    /// Stable gizmo order.
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    /// Short axis label for UI markers.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PositiveX => "+X",
            Self::NegativeX => "-X",
            Self::PositiveY => "+Y",
            Self::NegativeY => "-Y",
            Self::PositiveZ => "+Z",
            Self::NegativeZ => "-Z",
        }
    }

    /// World-space direction this axis snap looks from.
    #[must_use]
    pub const fn direction(self) -> Vec3 {
        match self {
            Self::PositiveX => Vec3::X,
            Self::NegativeX => Vec3::NEG_X,
            Self::PositiveY => Vec3::Y,
            Self::NegativeY => Vec3::NEG_Y,
            Self::PositiveZ => Vec3::Z,
            Self::NegativeZ => Vec3::NEG_Z,
        }
    }

    #[must_use]
    const fn yaw_pitch(self) -> (f32, f32) {
        match self {
            Self::PositiveX => (core::f32::consts::FRAC_PI_2, 0.0),
            Self::NegativeX => (-core::f32::consts::FRAC_PI_2, 0.0),
            Self::PositiveY => (0.0, core::f32::consts::FRAC_PI_2),
            Self::NegativeY => (0.0, -core::f32::consts::FRAC_PI_2),
            Self::PositiveZ => (0.0, 0.0),
            Self::NegativeZ => (core::f32::consts::PI, 0.0),
        }
    }
}

impl CameraPreset {
    /// Stable toolbar order.
    pub const ALL: [Self; 5] = [
        Self::Occlusal,
        Self::Buccal,
        Self::Lingual,
        Self::Mesial,
        Self::Distal,
    ];

    /// Short UI label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Occlusal => "Occlusal",
            Self::Buccal => "Buccal",
            Self::Lingual => "Lingual",
            Self::Mesial => "Mesial",
            Self::Distal => "Distal",
        }
    }

    /// Build a camera that frames the provided bbox from this preset.
    #[must_use]
    pub fn frame_bbox(self, bbox: Aabb, fovy: f32) -> Camera {
        match self {
            Self::Occlusal => Camera::default().frame_occlusal(bbox, fovy),
            Self::Buccal => frame_planar(bbox, fovy, 0.0, 0.0),
            Self::Lingual => frame_planar(bbox, fovy, core::f32::consts::PI, 0.0),
            Self::Mesial => frame_planar(bbox, fovy, core::f32::consts::FRAC_PI_2, 0.0),
            Self::Distal => frame_planar(bbox, fovy, -core::f32::consts::FRAC_PI_2, 0.0),
        }
    }
}

fn frame_planar(bbox: Aabb, fovy: f32, yaw: f32, pitch: f32) -> Camera {
    let mut camera = Camera::default();
    if bbox.is_empty() {
        return camera;
    }

    let size = bbox.size();
    let radius = (0.5 * size.length()).max(1.0);
    let half_fov = 0.5 * fovy;

    camera.target = bbox.center();
    camera.set_yaw_pitch(yaw, pitch);
    camera.projection = CameraProjection::Orthographic;
    camera.fovy = fovy;
    camera.orthographic_height = (radius * 2.0 / 0.7).max(MIN_ORTHOGRAPHIC_HEIGHT_MM);
    camera.distance = if half_fov > 1e-5 {
        radius / half_fov.tan() / 0.7
    } else {
        radius * 2.0
    };
    camera.fit_clip_planes_to_bbox(bbox);
    camera
}

impl Camera {
    /// Snap the camera orientation to an exact axis-aligned view while
    /// preserving the current target, distance, FOV, and clip planes.
    pub fn snap_to_axis(&mut self, axis: CameraAxisView) {
        let (yaw, pitch) = axis.yaw_pitch();
        self.set_yaw_pitch(yaw, pitch);
    }
}
