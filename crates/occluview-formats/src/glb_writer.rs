//! Deterministic self-contained GLB export for textured triangle meshes.

use crate::FormatError;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use occluview_core::{Mesh, MeshKind, MeshTexture};
use serde_json::{json, Map, Value};

const GLB_VERSION: u32 = 2;
const JSON_CHUNK_TYPE: u32 = 0x4E4F_534A;
const BIN_CHUNK_TYPE: u32 = 0x004E_4942;
const ARRAY_BUFFER: u32 = 34_962;
const ELEMENT_ARRAY_BUFFER: u32 = 34_963;
const FLOAT: u32 = 5_126;
const UNSIGNED_BYTE: u32 = 5_121;
const UNSIGNED_INT: u32 = 5_125;
const LINEAR: u32 = 9_729;
const CLAMP_TO_EDGE: u32 = 33_071;

#[derive(Copy, Clone)]
struct BinaryView {
    offset: usize,
    length: usize,
}

struct BinaryPayload {
    bytes: Vec<u8>,
    positions: BinaryView,
    normals: BinaryView,
    uvs: BinaryView,
    colors: Option<BinaryView>,
    indices: BinaryView,
    image: BinaryView,
    position_min: [f32; 3],
    position_max: [f32; 3],
}

struct BufferViewIndices {
    positions: usize,
    normals: usize,
    uvs: usize,
    colors: Option<usize>,
    indices: usize,
    image: usize,
}

struct AccessorSchema {
    values: Vec<Value>,
    attributes: Map<String, Value>,
    indices: usize,
}

/// Encode one textured triangle mesh as a self-contained GLB 2.0 document.
///
/// # Errors
///
/// Returns [`FormatError`] when the mesh cannot be represented by this writer.
pub fn write_textured_glb(mesh: &Mesh) -> Result<Vec<u8>, FormatError> {
    let texture = validate_mesh(mesh)?;
    let payload = encode_binary_payload(mesh, texture)?;
    let buffer_length = u32_len(payload.bytes.len(), "BIN chunk")?;
    let (buffer_views, view_indices) = build_buffer_views(&payload)?;
    let accessors = build_accessors(mesh, &payload, &view_indices)?;
    let document = build_document(buffer_length, buffer_views, view_indices.image, accessors);
    build_glb(&document, payload.bytes)
}

fn encode_binary_payload(mesh: &Mesh, texture: &MeshTexture) -> Result<BinaryPayload, FormatError> {
    let mut position_min = [f32::INFINITY; 3];
    let mut position_max = [f32::NEG_INFINITY; 3];
    let mut bytes = Vec::new();
    let positions = append_aligned(&mut bytes, |bytes| {
        for vertex in mesh.vertices() {
            for (axis, value) in vertex.position.into_iter().enumerate() {
                position_min[axis] = position_min[axis].min(value);
                position_max[axis] = position_max[axis].max(value);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    });
    let normals = append_aligned(&mut bytes, |bytes| {
        for vertex in mesh.vertices() {
            for value in vertex.normal {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    });
    let uvs = append_aligned(&mut bytes, |bytes| {
        for vertex in mesh.vertices() {
            for value in vertex.uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    });
    let colors = mesh.has_vertex_colors().then(|| {
        append_aligned(&mut bytes, |bytes| {
            for vertex in mesh.vertices() {
                bytes.extend_from_slice(&vertex.color);
            }
        })
    });
    let indices = append_aligned(&mut bytes, |bytes| {
        for index in mesh.indices() {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
    });
    let png = encode_png(texture)?;
    let image = append_aligned(&mut bytes, |bytes| bytes.extend_from_slice(&png));
    Ok(BinaryPayload {
        bytes,
        positions,
        normals,
        uvs,
        colors,
        indices,
        image,
        position_min,
        position_max,
    })
}

fn build_buffer_views(
    payload: &BinaryPayload,
) -> Result<(Vec<Value>, BufferViewIndices), FormatError> {
    let mut buffer_views = Vec::new();
    let positions = push_buffer_view(&mut buffer_views, payload.positions, Some(ARRAY_BUFFER))?;
    let normals = push_buffer_view(&mut buffer_views, payload.normals, Some(ARRAY_BUFFER))?;
    let uvs = push_buffer_view(&mut buffer_views, payload.uvs, Some(ARRAY_BUFFER))?;
    let colors = payload
        .colors
        .map(|view| push_buffer_view(&mut buffer_views, view, Some(ARRAY_BUFFER)))
        .transpose()?;
    let indices = push_buffer_view(
        &mut buffer_views,
        payload.indices,
        Some(ELEMENT_ARRAY_BUFFER),
    )?;
    let image = push_buffer_view(&mut buffer_views, payload.image, None)?;
    Ok((
        buffer_views,
        BufferViewIndices {
            positions,
            normals,
            uvs,
            colors,
            indices,
            image,
        },
    ))
}

fn build_accessors(
    mesh: &Mesh,
    payload: &BinaryPayload,
    views: &BufferViewIndices,
) -> Result<AccessorSchema, FormatError> {
    let vertex_count = u32::try_from(mesh.vertices().len())
        .map_err(|_| malformed("vertex count exceeds GLB limits"))?;
    let index_count = u32::try_from(mesh.indices().len())
        .map_err(|_| malformed("index count exceeds GLB limits"))?;

    let mut accessors = vec![
        json!({
            "bufferView": views.positions,
            "componentType": FLOAT,
            "count": vertex_count,
            "max": payload.position_max,
            "min": payload.position_min,
            "type": "VEC3"
        }),
        json!({
            "bufferView": views.normals,
            "componentType": FLOAT,
            "count": vertex_count,
            "type": "VEC3"
        }),
        json!({
            "bufferView": views.uvs,
            "componentType": FLOAT,
            "count": vertex_count,
            "type": "VEC2"
        }),
    ];
    let mut attributes = Map::new();
    attributes.insert("POSITION".to_string(), json!(0));
    attributes.insert("NORMAL".to_string(), json!(1));
    attributes.insert("TEXCOORD_0".to_string(), json!(2));
    if let Some(view) = views.colors {
        let accessor = accessors.len();
        accessors.push(json!({
            "bufferView": view,
            "componentType": UNSIGNED_BYTE,
            "count": vertex_count,
            "normalized": true,
            "type": "VEC4"
        }));
        attributes.insert("COLOR_0".to_string(), json!(accessor));
    }
    let index_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": views.indices,
        "componentType": UNSIGNED_INT,
        "count": index_count,
        "type": "SCALAR"
    }));
    Ok(AccessorSchema {
        values: accessors,
        attributes,
        indices: index_accessor,
    })
}

fn build_document(
    buffer_length: u32,
    buffer_views: Vec<Value>,
    image_view: usize,
    accessors: AccessorSchema,
) -> Value {
    json!({
        "accessors": accessors.values,
        "asset": {
            "generator": "OccluView",
            "version": "2.0"
        },
        "buffers": [{
            "byteLength": buffer_length
        }],
        "bufferViews": buffer_views,
        "images": [{
            "bufferView": image_view,
            "mimeType": "image/png"
        }],
        "materials": [{
            "pbrMetallicRoughness": {
                "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                "baseColorTexture": {
                    "index": 0,
                    "texCoord": 0
                },
                "metallicFactor": 0.0,
                "roughnessFactor": 1.0
            }
        }],
        "meshes": [{
            "primitives": [{
                "attributes": Value::Object(accessors.attributes),
                "indices": accessors.indices,
                "material": 0,
                "mode": 4
            }]
        }],
        "nodes": [{
            "mesh": 0
        }],
        "samplers": [{
            "magFilter": LINEAR,
            "minFilter": LINEAR,
            "wrapS": CLAMP_TO_EDGE,
            "wrapT": CLAMP_TO_EDGE
        }],
        "scene": 0,
        "scenes": [{
            "nodes": [0]
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }]
    })
}

fn validate_mesh(mesh: &Mesh) -> Result<&MeshTexture, FormatError> {
    if mesh.kind() != MeshKind::TriangleMesh || mesh.indices().is_empty() {
        return Err(malformed("textured GLB export requires a triangle mesh"));
    }
    if mesh.vertices().is_empty() {
        return Err(malformed("textured GLB export requires vertices"));
    }
    for vertex in mesh.vertices() {
        if !vertex.position.into_iter().all(f32::is_finite) {
            return Err(malformed("vertex positions must be finite"));
        }
        if !vertex.normal.into_iter().all(f32::is_finite) {
            return Err(malformed("vertex normals must be finite"));
        }
        if !vertex.uv.into_iter().all(f32::is_finite) {
            return Err(malformed("texture coordinates must be finite"));
        }
    }

    let texture = mesh
        .texture()
        .ok_or_else(|| malformed("textured GLB export requires an RGBA texture"))?;
    if texture.width == 0 || texture.height == 0 {
        return Err(malformed("texture dimensions must be non-zero"));
    }
    let expected_length = u64::from(texture.width)
        .checked_mul(u64::from(texture.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| malformed("texture dimensions exceed platform limits"))?;
    if texture.rgba.len() != expected_length {
        return Err(malformed(
            "texture RGBA length does not match its dimensions",
        ));
    }
    Ok(texture)
}

fn encode_png(texture: &MeshTexture) -> Result<Vec<u8>, FormatError> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Best, FilterType::Paeth)
        .write_image(
            &texture.rgba,
            texture.width,
            texture.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| malformed(format!("PNG encoding failed: {error}")))?;
    Ok(png)
}

fn append_aligned(bytes: &mut Vec<u8>, append: impl FnOnce(&mut Vec<u8>)) -> BinaryView {
    pad_to_four(bytes, 0);
    let offset = bytes.len();
    append(bytes);
    BinaryView {
        offset,
        length: bytes.len() - offset,
    }
}

fn push_buffer_view(
    views: &mut Vec<Value>,
    view: BinaryView,
    target: Option<u32>,
) -> Result<usize, FormatError> {
    let index = views.len();
    let mut value = Map::new();
    value.insert("buffer".to_string(), json!(0));
    value.insert(
        "byteLength".to_string(),
        json!(u32_len(view.length, "buffer view")?),
    );
    value.insert(
        "byteOffset".to_string(),
        json!(u32_len(view.offset, "buffer view offset")?),
    );
    if let Some(target) = target {
        value.insert("target".to_string(), json!(target));
    }
    views.push(Value::Object(value));
    Ok(index)
}

fn build_glb(document: &Value, mut bin: Vec<u8>) -> Result<Vec<u8>, FormatError> {
    let mut json_bytes = serde_json::to_vec(document)
        .map_err(|error| malformed(format!("JSON encoding failed: {error}")))?;
    pad_to_four(&mut json_bytes, b' ');
    pad_to_four(&mut bin, 0);

    let total_length = 12usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json_bytes.len()))
        .and_then(|length| length.checked_add(8))
        .and_then(|length| length.checked_add(bin.len()))
        .ok_or_else(|| malformed("GLB length overflow"))?;
    let total_length = u32_len(total_length, "GLB")?;
    let json_length = u32_len(json_bytes.len(), "JSON chunk")?;
    let bin_length = u32_len(bin.len(), "BIN chunk")?;

    let mut glb = Vec::with_capacity(total_length as usize);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&GLB_VERSION.to_le_bytes());
    glb.extend_from_slice(&total_length.to_le_bytes());
    glb.extend_from_slice(&json_length.to_le_bytes());
    glb.extend_from_slice(&JSON_CHUNK_TYPE.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&bin_length.to_le_bytes());
    glb.extend_from_slice(&BIN_CHUNK_TYPE.to_le_bytes());
    glb.extend_from_slice(&bin);
    Ok(glb)
}

fn pad_to_four(bytes: &mut Vec<u8>, value: u8) {
    let padding = (4 - bytes.len() % 4) % 4;
    bytes.resize(bytes.len() + padding, value);
}

fn u32_len(length: usize, label: &str) -> Result<u32, FormatError> {
    u32::try_from(length).map_err(|_| malformed(format!("{label} exceeds GLB limits")))
}

fn malformed(reason: impl Into<String>) -> FormatError {
    FormatError::Malformed {
        format: "glTF export",
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::write_textured_glb;
    use glam::Vec3;
    use occluview_core::{Mesh, MeshTexture, Vertex};
    use serde_json::Value;

    fn textured_seam_mesh() -> Mesh {
        let vertices = vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0))
                .with_normal(Vec3::Z)
                .with_color([11, 21, 31, 255])
                .with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0))
                .with_normal(Vec3::Y)
                .with_color([41, 51, 61, 245])
                .with_uv([1.0, 0.0]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0))
                .with_normal(Vec3::X)
                .with_color([71, 81, 91, 235])
                .with_uv([0.0, 1.0]),
            // Position duplicated at a UV seam; the writer must not weld it.
            Vertex::at(Vec3::new(1.0, 0.0, 0.0))
                .with_normal(Vec3::NEG_Z)
                .with_color([101, 111, 121, 225])
                .with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0))
                .with_normal(Vec3::NEG_Y)
                .with_color([131, 141, 151, 215])
                .with_uv([1.0, 1.0]),
            // Position duplicated at the other side of the same seam.
            Vertex::at(Vec3::new(0.0, 1.0, 0.0))
                .with_normal(Vec3::NEG_X)
                .with_color([161, 171, 181, 205])
                .with_uv([1.0, 0.0]),
        ];
        let mut mesh = Mesh::new(
            Some("textured seam".to_string()),
            vertices,
            vec![1, 4, 2, 5, 0, 3],
        )
        .expect("valid seam mesh");
        mesh.set_texture(MeshTexture::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 0, 0,
            ],
        ));
        mesh
    }

    fn parse_glb(glb: &[u8]) -> (Value, &[u8]) {
        let (json, bin) = crate::gltf::glb::split(glb).expect("writer must emit valid GLB");
        let document = serde_json::from_slice(&json).expect("writer JSON must parse");
        (document, bin)
    }

    fn embedded_png<'a>(document: &Value, bin: &'a [u8]) -> &'a [u8] {
        let image_view = document["images"][0]["bufferView"]
            .as_u64()
            .expect("embedded image bufferView") as usize;
        let view = &document["bufferViews"][image_view];
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"].as_u64().expect("image byteLength") as usize;
        &bin[offset..offset + length]
    }

    #[test]
    fn round_trip_preserves_geometry_seams_attributes_indices_and_texture() {
        let source = textured_seam_mesh();

        let glb = write_textured_glb(&source).expect("write textured GLB");
        let decoded = crate::gltf::read(&glb).expect("read written GLB");

        assert_eq!(decoded.indices(), source.indices());
        assert_eq!(decoded.vertices().len(), source.vertices().len());
        for (actual, expected) in decoded.vertices().iter().zip(source.vertices()) {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.normal, expected.normal);
            assert_eq!(actual.color, expected.color);
            assert_eq!(actual.uv, expected.uv);
        }
        assert_eq!(
            decoded.vertices()[1].position,
            decoded.vertices()[3].position
        );
        assert_ne!(decoded.vertices()[1].uv, decoded.vertices()[3].uv);
        assert_eq!(
            decoded.vertices()[2].position,
            decoded.vertices()[5].position
        );
        assert_ne!(decoded.vertices()[2].uv, decoded.vertices()[5].uv);

        let actual_texture = decoded.texture().expect("embedded texture");
        let expected_texture = source.texture().expect("source texture");
        assert_eq!(actual_texture.width, expected_texture.width);
        assert_eq!(actual_texture.height, expected_texture.height);
        assert_eq!(actual_texture.rgba, expected_texture.rgba);
    }

    #[test]
    fn output_is_self_contained_aligned_and_uses_base_color_texture_semantics() {
        let glb = write_textured_glb(&textured_seam_mesh()).expect("write textured GLB");
        let (document, _bin) = parse_glb(&glb);

        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(glb[4..8].try_into().expect("version")),
            2
        );
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().expect("length")) as usize,
            glb.len()
        );
        assert_eq!(glb.len() % 4, 0);
        assert!(document["buffers"][0].get("uri").is_none());
        assert!(document["images"][0].get("uri").is_none());
        assert_eq!(document["images"][0]["mimeType"], "image/png");

        for view in document["bufferViews"].as_array().expect("buffer views") {
            assert_eq!(view["byteOffset"].as_u64().unwrap_or(0) % 4, 0);
        }

        let index_accessor = document["meshes"][0]["primitives"][0]["indices"]
            .as_u64()
            .expect("index accessor") as usize;
        assert_eq!(document["accessors"][index_accessor]["componentType"], 5125);
        let index_view = document["accessors"][index_accessor]["bufferView"]
            .as_u64()
            .expect("index buffer view") as usize;
        assert_eq!(document["bufferViews"][index_view]["target"], 34963);

        // glTF defines a PBR base-color texture's RGB channels as sRGB.
        assert_eq!(
            document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
        assert_eq!(document["textures"][0]["source"], 0);
        assert_eq!(document["textures"][0]["sampler"], 0);
        assert_eq!(document["samplers"][0]["wrapS"], 33071);
        assert_eq!(document["samplers"][0]["wrapT"], 33071);
    }

    #[test]
    fn embedded_png_and_complete_glb_are_byte_deterministic() {
        let mesh = textured_seam_mesh();

        let first = write_textured_glb(&mesh).expect("first GLB");
        let second = write_textured_glb(&mesh).expect("second GLB");
        let (first_document, first_bin) = parse_glb(&first);
        let (second_document, second_bin) = parse_glb(&second);
        let first_png = embedded_png(&first_document, first_bin);
        let second_png = embedded_png(&second_document, second_bin);

        assert_eq!(first_png, second_png);
        assert!(first_png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(first, second);
    }
}
