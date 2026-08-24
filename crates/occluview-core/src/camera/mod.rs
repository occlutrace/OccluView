//! Camera model and the dental **occlusal default** framing.
//!
//! The default camera looks down onto the occlusal plane — the chewing surface
//! — fit to the mesh bounding box. This is the single most visible
//! "this is a dental tool" signal, and it is shared by the app and the thumbnail
//! renderer so the two match pixel-for-pixel.

mod framing;
mod input;
mod movement;
mod orbit;
mod orientation;
mod presets;

use glam::{Quat, Vec3};

pub use input::{
    orbit_delta_from_pointer_motion, zoom_factor_from_scroll, CAD_ORBIT_DRAG_GAIN,
    CAD_ZOOM_SCROLL_SENSITIVITY,
};
pub use presets::{CameraAxisView, CameraPreset};

/// Smallest orthographic height anything will fit to.
///
/// Public because the Explorer preview fits its own camera and had to copy
/// this number, with a comment asking for it to be exported.
pub const MIN_ORTHOGRAPHIC_HEIGHT_MM: f32 = 0.01;

/// How much of the framed height a fitted bounding sphere fills.
///
/// The bare `0.7` was written three times -- twice inside this module, once in
/// the shell's preview -- with nothing naming what it meant. The three radius
/// derivations around it differ on purpose and stay separate; the fill factor
/// does not differ and now has one name.
///
/// Thumbnails use a tighter 0.90 because a tile is square. Not a fourth copy
/// of this one.
pub const BBOX_FRAME_FILL: f32 = 0.7;
/// Zoom-out ceiling: generous (a 1 km tall viewport for mm-scale dental
/// scenes) yet far below f32 overflow, keeping the projection matrix finite.
pub(super) const MAX_ORTHOGRAPHIC_HEIGHT_MM: f32 = 1.0e6;

/// Projection model used by the inspection camera.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CameraProjection {
    /// CAD-style view with no perspective scale distortion.
    Orthographic,
}

/// An orbital camera, the natural model for inspecting a mesh.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Camera {
    /// World-space point the camera orbits around.
    pub target: Vec3,
    /// Distance from target to eye, in millimeters.
    pub distance: f32,
    /// Yaw (around world Y), in radians.
    pub yaw: f32,
    /// Pitch (elevation from the horizontal plane), in radians.
    pub pitch: f32,
    /// Free-orbit view orientation. When absent, the camera derives the
    /// orientation from `yaw`/`pitch` for simple exact-axis construction.
    pub orientation: Option<Quat>,
    /// Projection model used for the viewport.
    pub projection: CameraProjection,
    /// Orthographic viewport height in world millimeters.
    pub orthographic_height: f32,
    /// Vertical field of view, in radians.
    pub fovy: f32,
    /// Near plane, millimeters.
    pub near: f32,
    /// Far plane, millimeters.
    pub far: f32,
}

#[cfg(test)]
mod tests;
