use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// A vertex. `#[repr(C)]` so it can be uploaded to the GPU verbatim via
/// `bytemuck::cast_slice`.
///
/// Layout (36 bytes, no padding, max align 4):
/// - `position` `[f32;3]` @ 0
/// - `normal`   `[f32;3]` @ 12
/// - `color`    `[u8;4]`  @ 24
/// - `uv`       `[f32;2]` @ 28
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    /// Position in OccluView's canonical frame, in millimeters.
    pub position: [f32; 3],
    /// Unit normal, in canonical frame. Defaults to zero (flat-shaded on load).
    pub normal: [f32; 3],
    /// sRGBA color, packed 0..=255. `(255, 255, 255, 255)` when color is absent.
    pub color: [u8; 4],
    /// Texture coordinates (UV). `[0.0, 0.0]` when the vertex has no UV
    /// (plain STL, untextured PLY, etc.). Loaders set this from `TEXCOORD_0`
    /// (glTF), `vt` (OBJ), or `texcoord`/`s`/`t` (PLY).
    pub uv: [f32; 2],
}

impl Vertex {
    /// Construct a vertex with position only (normal zeroed, color white,
    /// UV zeroed).
    #[inline]
    #[must_use]
    pub fn at(position: Vec3) -> Self {
        Self {
            position: position.to_array(),
            normal: [0.0; 3],
            color: [255, 255, 255, 255],
            uv: [0.0, 0.0],
        }
    }

    /// Construct a vertex with position and normal (color white, UV zeroed).
    #[inline]
    #[must_use]
    pub fn with_normal(mut self, normal: Vec3) -> Self {
        self.normal = normal.to_array();
        self
    }

    /// Construct a vertex with a vertex color.
    #[inline]
    #[must_use]
    pub fn with_color(mut self, rgba: [u8; 4]) -> Self {
        self.color = rgba;
        self
    }

    /// Construct a vertex with texture coordinates.
    #[inline]
    #[must_use]
    pub fn with_uv(mut self, uv: [f32; 2]) -> Self {
        self.uv = uv;
        self
    }
}
