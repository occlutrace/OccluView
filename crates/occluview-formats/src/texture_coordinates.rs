//! Format UVs are normalized to the renderer's top-left image convention.
//! PLY and OBJ address images from the bottom left; GLB and HPS already
//! supply the convention used by our decoded top-to-bottom raster rows.

/// Convert between bottom-left file UVs and top-left internal UVs. Applying
/// this at both the reader and writer boundaries keeps external atlases upright.
pub(crate) fn flip_vertical(uv: [f32; 2]) -> [f32; 2] {
    [uv[0], 1.0 - uv[1]]
}
