//! Per-mesh GPU uniform: model matrix, tint, opacity, texture/color presence,
//! and independent display flags.
//!
//! Bound at group 1, binding 0. One uniform per mesh lets the renderer place
//! multiple meshes (multi-mesh scene) and branch the fragment shader between
//! vertex-color and texture-sampled shading.
//!
//! Layout (112 bytes; the final three u32 values provide the uniform-buffer
//! alignment required by WGSL):
//! - `model`               `[f32;16]`  64 bytes
//! - `tint`                 `[f32;4]`  16 bytes
//! - `opacity`               `f32`       4 bytes
//! - `has_texture`           `u32`       4 bytes
//! - `show_orientation`      `u32`       4 bytes
//! - `show_vertex_colors`    `u32`       4 bytes
//! - `show_texture`          `u32`       4 bytes
//! - `padding`               `[u32;3]`  12 bytes

use bytemuck::{Pod, Zeroable};

/// Per-mesh GPU uniform (see module docs for the full layout).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuMeshUniform {
    /// Column-major model matrix (the result of `Mat4::from_affine3(...)`).
    pub model: [f32; 16],
    /// Linear-sRGB tint multiplied into the base color. Default white.
    pub tint: [f32; 4],
    /// Opacity 0..1.
    pub opacity: f32,
    /// 0 = use vertex color; 1 = sample `mesh_texture`. Stored as `u32` for
    /// std140 alignment.
    pub has_texture: u32,
    /// 1 = orientation diagnostic: paint back-facing fragments solid red
    /// (exocad "Show triangle orientation").
    pub show_orientation: u32,
    /// 0 = ignore scan color/texture and shade with a flat neutral material
    /// (the shader's `NEUTRAL_MATERIAL_RGB`, matching
    /// `occluview_core::scene::material::DEFAULT_UNTEXTURED_MESH_TINT`); 1 =
    /// normal vertex-color/texture shading. Display-only: mesh data is
    /// untouched, so edits and exports keep the real colors.
    pub show_vertex_colors: u32,
    /// 0 = do not sample the attached texture; 1 = texture sampling enabled.
    /// This is intentionally independent from `show_vertex_colors`.
    pub show_texture: u32,
    /// Explicit tail padding: uniform structs have a 16-byte alignment in
    /// WGSL even though each scalar flag is four-byte aligned.
    pub padding: [u32; 3],
}

impl GpuMeshUniform {
    /// The identity uniform: identity model matrix, white tint, full opacity,
    /// no texture, vertex colors shown. Used by the legacy single-mesh draw
    /// path and as a default.
    #[must_use]
    pub const fn identity() -> Self {
        // Column-major identity mat4.
        const IDENTITY: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ];
        Self {
            model: IDENTITY,
            tint: [1.0, 1.0, 1.0, 1.0],
            opacity: 1.0,
            has_texture: 0,
            show_orientation: 0,
            show_vertex_colors: 1,
            show_texture: 1,
            padding: [0; 3],
        }
    }
}

impl Default for GpuMeshUniform {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_112_bytes_and_aligned() {
        assert_eq!(size_of::<GpuMeshUniform>(), 112);
        // The Rust struct only needs 4-byte (f32/u32) alignment; the 16-byte
        // uniform-buffer offset alignment std140 requires is a wgpu binding
        // concern, not a property of this type.
        assert_eq!(align_of::<GpuMeshUniform>(), 4);
    }

    #[test]
    fn identity_round_trips() {
        let u = GpuMeshUniform::identity();
        assert_eq!(u.tint, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(u.opacity, 1.0);
        assert_eq!(u.has_texture, 0);
        // Identity mat4: diagonal = 1, off-diagonal = 0.
        assert_eq!(u.model[0], 1.0);
        assert_eq!(u.model[5], 1.0);
        assert_eq!(u.model[10], 1.0);
        assert_eq!(u.model[15], 1.0);
        assert_eq!(u.model[1], 0.0);
    }

    /// Pins the shader's hand-copied `NEUTRAL_MATERIAL_RGB` (`mesh.wgsl`)
    /// against the core crate's own untextured-mesh tint, so the two can
    /// never silently drift apart — this is the test `mesh.wgsl`'s doc
    /// comment for that constant promises exists.
    #[test]
    fn neutral_material_matches_the_core_untextured_tint() {
        const NEUTRAL_MATERIAL_RGB: [f32; 3] = [0.82, 0.68, 0.42];
        let [r, g, b, _a] = occluview_core::DEFAULT_UNTEXTURED_MESH_TINT;
        assert_eq!(NEUTRAL_MATERIAL_RGB, [r, g, b]);
    }
}
