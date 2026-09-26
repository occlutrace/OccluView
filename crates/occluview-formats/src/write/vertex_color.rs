//! Bake an attached texture into per-vertex colour for the open formats.
//!
//! PLY and OBJ have no texture element (OBJ can only name an image beside the
//! file, PLY cannot name one at all), so the colour travels on the vertices.
//! The sampling rule lives here so both writers show the operator the same
//! colours.

use super::MeshWriteOptions;
use occluview_core::{Mesh, MeshKind, MeshTexture};

/// Sample an attached atlas into one RGBA colour per vertex.
///
/// Returns `None` when there is no atlas to bake, or when one cannot be used:
/// the image needs UVs and faces to be sampled through, and a buffer whose
/// length matches its dimensions. A `None` from an attached texture leaves the
/// caller to report `MeshWriteWarning::TextureImageNotWritten` rather than
/// passing silently.
pub(super) fn baked_colors(mesh: &Mesh, options: &MeshWriteOptions) -> Option<Vec<[u8; 4]>> {
    if !options.include_vertex_colors || !options.include_texture {
        return None;
    }
    let texture = mesh.texture()?;
    // Without faces there is nothing for the UVs to index, and without UVs
    // there is nothing to sample: the image would be dropped anyway.
    if mesh.kind() != MeshKind::TriangleMesh || !mesh.has_uvs() || mesh.indices().is_empty() {
        return None;
    }
    if texture.width == 0
        || texture.height == 0
        || texture.rgba.len()
            != (texture.width as usize)
                .saturating_mul(texture.height as usize)
                .saturating_mul(4)
    {
        return None;
    }
    Some(
        mesh.vertices()
            .iter()
            .map(|vertex| sample(texture, vertex.uv))
            .collect(),
    )
}

/// Bilinear sample of `texture` at `uv`, matching how the viewer samples it.
///
/// The renderer binds the image with `ClampToEdge` and linear filtering, and
/// samples it at the same `uv`, so the colour written here is the colour the
/// operator was looking at. Normalized coordinates put `(0, 0)` on the texture's
/// top-left edge and `(1, 1)` on its bottom-right edge; the half-texel shift
/// turns that into texel-centre space, and the per-address clamp makes the
/// edges return the edge texel, which is what a GPU sampler does. A non-finite
/// coordinate has no texel, and returns opaque white.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub(super) fn sample(texture: &MeshTexture, uv: [f32; 2]) -> [u8; 4] {
    if !uv[0].is_finite() || !uv[1].is_finite() {
        return [255, 255, 255, 255];
    }
    let width = texture.width as usize;
    let height = texture.height as usize;
    if width == 0 || height == 0 {
        return [255, 255, 255, 255];
    }
    let x = f64::from(uv[0]) * width as f64 - 0.5;
    let y = f64::from(uv[1]) * height as f64 - 0.5;
    // The floor and the fraction come from the raw coordinate. Each texel
    // address is then clamped on its own, which is what the GPU's
    // `ClampToEdge` does: a coordinate on the very edge resolves both addresses
    // to the edge texel and returns it unchanged. Clamping the floor first and
    // only then adding one would blend the edge texel with its neighbour along
    // the whole border — a seam the viewer never showed.
    let floor_x = x.floor();
    let floor_y = y.floor();
    let fx = x - floor_x;
    let fy = y - floor_y;
    let edge = |value: f64, max: usize| -> usize { value.clamp(0.0, (max - 1) as f64) as usize };
    let x0 = edge(floor_x, width);
    let x1 = edge(floor_x + 1.0, width);
    let y0 = edge(floor_y, height);
    let y1 = edge(floor_y + 1.0, height);
    let texel = |ix: usize, iy: usize| -> [f64; 4] {
        let offset = (iy * width + ix) * 4;
        let pixel = &texture.rgba[offset..offset + 4];
        [
            f64::from(pixel[0]),
            f64::from(pixel[1]),
            f64::from(pixel[2]),
            f64::from(pixel[3]),
        ]
    };
    let top_left = texel(x0, y0);
    let top_right = texel(x1, y0);
    let bottom_left = texel(x0, y1);
    let bottom_right = texel(x1, y1);
    let mut out = [0_u8; 4];
    for (channel, value) in out.iter_mut().enumerate() {
        let top = top_left[channel] * (1.0 - fx) + top_right[channel] * fx;
        let bottom = bottom_left[channel] * (1.0 - fx) + bottom_right[channel] * fx;
        *value = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}
