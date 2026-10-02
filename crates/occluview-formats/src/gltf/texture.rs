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
    let buffer = doc
        .buffers
        .get(bv.buffer)
        .ok_or_else(|| malformed("image buffer out of range"))?;
    if bv.buffer != 0 || buffer.uri.is_some() {
        return Err(malformed(
            "external image buffers are unsupported (GLB only)",
        ));
    }
    let offset = bv.byte_offset.unwrap_or(0);
    let end = offset
        .checked_add(bv.byte_length as usize)
        .ok_or_else(|| malformed("image bufferView byte range overflows"))?;
    if end > buffer.byte_length as usize {
        return Err(malformed(
            "image bufferView extends past its declared buffer",
        ));
    }
    let img_bytes = bin_chunk.get(offset..end).ok_or(FormatError::Truncated {
        format: "glTF",
        expected: end,
        got: bin_chunk.len(),
    })?;
    decode_embedded_raster(img_bytes, "glTF").map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_document() -> (json::GltfDoc, Vec<u8>) {
        let png = crate::glb_writer::encode_png(&MeshTexture::new(1, 1, vec![255, 0, 0, 255]))
            .expect("PNG");
        let doc = serde_json::from_value(serde_json::json!({
            "asset": {"version": "2.0"},
            "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
            "textures": [{"source": 0}],
            "images": [{"bufferView": 0, "mimeType": "image/png"}],
            "bufferViews": [{"buffer": 0, "byteLength": png.len()}],
            "buffers": [{"byteLength": png.len()}]
        }))
        .expect("document");
        (doc, png)
    }

    #[test]
    fn image_view_rejects_offset_overflow() {
        let (mut doc, png) = image_document();
        doc.buffer_views[0].byte_offset = Some(usize::MAX);
        doc.buffer_views[0].byte_length = 1;
        assert!(resolve_material_texture(&doc, 0, &png).is_err());
    }

    #[test]
    fn image_view_rejects_the_wrong_buffer() {
        let (mut doc, png) = image_document();
        doc.buffer_views[0].buffer = 1;
        assert!(resolve_material_texture(&doc, 0, &png).is_err());
    }

    #[test]
    fn image_view_cannot_read_past_its_declared_buffer() {
        let (mut doc, png) = image_document();
        doc.buffers[0].byte_length = 1;
        assert!(resolve_material_texture(&doc, 0, &png).is_err());
    }

    #[test]
    fn image_view_reads_a_valid_embedded_image() {
        let (doc, png) = image_document();
        let texture = resolve_material_texture(&doc, 0, &png)
            .expect("valid view")
            .expect("image");
        assert_eq!(texture.rgba, [255, 0, 0, 255]);
    }
}
