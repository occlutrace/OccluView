//! GPU-side camera uniform, matching the WGSL `Camera` struct.
//!
//! Layout (160 bytes total): two 4x4 float matrices (view, projection), then a
//! 3-component light direction plus one scalar, then a 3-component camera
//! position plus one scalar. The `#[repr(C)]` field order plus the explicit
//! scalar slots satisfy the WGSL uniform-layout alignment rule
//! (a 3-component vector followed by a scalar occupies 16 bytes).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// GPU uniform: view + projection + lighting, uploaded once per frame.
///
/// The `pad0`/`pad1` scalar slots preserve the WGSL vec3+scalar alignment.
/// The renderer overwrites them with point-splat viewport width/height during
/// camera upload; callers should construct cameras with `GpuCamera::new`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[allow(clippy::struct_field_names, dead_code)]
pub struct GpuCamera {
    /// View matrix (world -> camera).
    pub view: [f32; 16],
    /// Projection matrix (camera -> clip).
    pub projection: [f32; 16],
    /// Direction TO the light from the scene (unit length expected).
    pub light_dir: [f32; 3],
    /// Point-splat viewport width during upload; initialized to zero.
    pub pad0: f32,
    /// Camera position in world space (for specular; unused in v1 Lambertian).
    pub camera_pos: [f32; 3],
    /// Point-splat viewport height during upload; initialized to zero.
    pub pad1: f32,
}

impl GpuCamera {
    /// Build from a `glam` view/projection plus lighting parameters.
    #[must_use]
    pub fn new(view: Mat4, projection: Mat4, light_dir: Vec3, camera_pos: Vec3) -> Self {
        Self {
            view: view.to_cols_array(),
            projection: projection.to_cols_array(),
            light_dir: light_dir.to_array(),
            pad0: 0.0,
            camera_pos: camera_pos.to_array(),
            pad1: 0.0,
        }
    }
}

/// Right-handed view matrix for the shared orbital [`occluview_core::Camera`].
///
/// One home for every consumer (app viewport, Explorer preview pane, shell
/// thumbnails) so their on-screen behavior can never drift apart.
#[must_use]
pub fn camera_view_matrix(camera: &occluview_core::Camera) -> Mat4 {
    Mat4::look_at_rh(camera.eye(), camera.target, camera.view_up())
}

/// Orthographic projection for the shared orbital camera at `aspect` (w/h).
#[must_use]
pub fn camera_ortho_proj_matrix(camera: &occluview_core::Camera, aspect: f32) -> Mat4 {
    let half_height = camera.orthographic_height * 0.5;
    let half_width = half_height * aspect.max(0.001);
    Mat4::orthographic_rh(
        -half_width,
        half_width,
        -half_height,
        half_height,
        camera.near,
        camera.far,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn size_matches_wgsl_layout() {
        // 2 matrices (64 each) + 2 * (vec3 + pad) (16 each) = 160 bytes.
        assert_eq!(size_of::<GpuCamera>(), 160);
    }

    #[test]
    fn constructs_from_glam() {
        let c = GpuCamera::new(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::Y, Vec3::ZERO);
        assert_eq!(c.light_dir, [0.0, 1.0, 0.0]);
        assert_eq!(c.pad0, 0.0);
    }
}
