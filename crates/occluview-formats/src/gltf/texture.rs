use super::error::malformed;
use super::json;
use crate::error::FormatError;
use crate::texture_decode::decode_embedded_raster;
use occluview_core::MeshTexture;

/// Resolve a material's base-color texture to a decoded [`MeshTexture`].
///
/// glTF material → `pbrMetallicRoughness.baseColorTexture.index` →
/// `textures[idx].source` → `images[source].bufferView` → decode PNG/JPEG.
///
/// Returns `None` if the material has no base-color texture, or if the texture
/// chain references an external URI (out of scope for v1).
pub(super) fn resolve_material_texture(
    doc: &json::GltfDoc,
    material_idx: usize,
    bin_chunk: &[u8],
) -> Result<Option<MeshTexture>, FormatError> {
    let material = doc
        .materials
        .get(material_idx)
        .ok_or_else(|| malformed("material out of range"))?;
    // materials are opaque serde_json::Value — dig into pbrMetallicRoughness.
    let Some(pbr) = material.get("pbrMetallicRoughness") else {
        return Ok(None);
    };
    let Some(base_color_tex) = pbr.get("baseColorTexture") else {
        return Ok(None); // no texture on this material
    };
    let tex_idx = base_color_tex
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| malformed("baseColorTexture has no index"))? as usize;
    let texture = doc
        .textures
        .get(tex_idx)
        .ok_or_else(|| malformed("texture out of range"))?;
    let source = texture
        .source
        .ok_or_else(|| malformed("texture has no source"))?;
    let image = doc
        .images
        .get(source)
        .ok_or_else(|| malformed("image out of range"))?;
    // Only bufferView-embedded images are supported (external URI rejected).
    let bv_idx = image
        .buffer_view
        .ok_or_else(|| malformed("image has no bufferView (external URI unsupported)"))?;
    let bv = doc
        .buffer_views
        .get(bv_idx)
        .ok_or_else(|| malformed("image bufferView out of range"))?;
    let offset = bv.byte_offset.unwrap_or(0);
    let end = offset + bv.byte_length as usize;
    let img_bytes = bin_chunk.get(offset..end).ok_or(FormatError::Truncated {
        format: "glTF",
        expected: end,
        got: bin_chunk.len(),
    })?;
    decode_embedded_raster(img_bytes, "glTF").map(Some)
}
