use super::accessor::{read_color_f32, read_f32_vec3, read_indices, read_texcoord};
use super::error::malformed;
use super::json;
use crate::error::FormatError;
use glam::{DMat4, Mat4, Vec3};
use occluview_core::{MeshBuilder, Vertex};

pub(super) fn emit_primitive(
    doc: &json::GltfDoc,
    prim: &json::Primitive,
    transform: Mat4,
    bin_chunk: &[u8],
    builder: &mut MeshBuilder,
) -> Result<(), FormatError> {
    if let Some(mode) = prim.mode {
        if mode != 4 {
            return Err(malformed(&format!(
                "primitive mode {mode} not supported (only triangles=4)"
            )));
        }
    }

    let pos_acc_idx = prim
        .attributes
        .position
        .ok_or_else(|| malformed("primitive has no POSITION"))?;
    let positions = read_f32_vec3(doc, pos_acc_idx, bin_chunk)?;
    let normals = prim
        .attributes
        .normal
        .map(|i| read_f32_vec3(doc, i, bin_chunk))
        .transpose()?;
    let colors = prim
        .attributes
        .color_0
        .map(|i| read_color_f32(doc, i, bin_chunk))
        .transpose()?;
    let uvs = prim
        .attributes
        .texcoord_0
        .map(|i| read_texcoord(doc, i, bin_chunk))
        .transpose()?;

    let vertex_count = positions.len();
    for (name, count) in [
        ("NORMAL", normals.as_ref().map(Vec::len)),
        ("COLOR_0", colors.as_ref().map(Vec::len)),
        ("TEXCOORD_0", uvs.as_ref().map(Vec::len)),
    ] {
        if count.is_some_and(|count| count != vertex_count) {
            return Err(malformed(&format!(
                "{name} count does not match POSITION count"
            )));
        }
    }
    let base = builder_push_vertices(
        VertexStreams {
            positions: &positions,
            normals: normals.as_deref(),
            colors: colors.as_deref(),
            uvs: uvs.as_deref(),
        },
        transform,
        builder,
    )?;
    // Positions are baked into the mesh, so a reflection must also reverse
    // corners to retain glTF's front-facing side in the stored geometry.
    let mirrored = transform.as_dmat4().determinant() < 0.0;
    let mut push_triangle = |a, b, c| {
        let (b, c) = if mirrored { (c, b) } else { (b, c) };
        builder.push_triangle(a, b, c);
    };

    if let Some(idx_acc) = prim.indices {
        let indices = read_indices(doc, idx_acc, bin_chunk)?;
        if indices.len() % 3 != 0 {
            return Err(malformed(
                "indexed primitive with index count not divisible by 3",
            ));
        }
        for chunk in indices.as_chunks::<3>().0 {
            let (a, b, c) = (chunk[0], chunk[1], chunk[2]);
            // A corner is arbitrary input, and `base + a` is unchecked: with
            // the release profile's wrapped arithmetic a sentinel such as
            // 0xFFFF_FFFF in the second primitive would alias a valid vertex
            // and build a triangle from the wrong one without an error, because
            // `Mesh::new` only sees the wrapped result. A debug build would
            // panic instead, which ends the process under panic = "abort".
            let limit = u32::try_from(vertex_count).unwrap_or(u32::MAX);
            if a >= limit || b >= limit || c >= limit {
                return Err(malformed("index out of range for this primitive"));
            }
            push_triangle(base + a, base + b, base + c);
        }
    } else if vertex_count % 3 == 0 {
        for i in (0..vertex_count).step_by(3) {
            push_triangle(
                base + i as u32,
                base + (i + 1) as u32,
                base + (i + 2) as u32,
            );
        }
    } else {
        return Err(malformed(
            "non-indexed primitive with vertex count not divisible by 3",
        ));
    }
    Ok(())
}

/// Push `positions` (with optional matching normals/colors/uvs) and return the
/// handle of the first pushed vertex, used as the index base.
struct VertexStreams<'a> {
    positions: &'a [[f32; 3]],
    normals: Option<&'a [[f32; 3]]>,
    colors: Option<&'a [[u8; 4]]>,
    uvs: Option<&'a [[f32; 2]]>,
}

fn builder_push_vertices(
    streams: VertexStreams<'_>,
    transform: Mat4,
    builder: &mut MeshBuilder,
) -> Result<u32, FormatError> {
    let mut first = 0u32;
    let normal_transform = normal_transform_for(transform);
    for (i, p) in streams.positions.iter().enumerate() {
        let position = transform.transform_point3(Vec3::from_array(*p));
        for component in position.to_array() {
            crate::finite_coordinate(component, "glTF", i)?;
        }
        let mut v = Vertex::at(position);
        if let Some(ns) = streams.normals {
            if i < ns.len() {
                let normal = normal_transform.transform_vector3(Vec3::from_array(ns[i]).as_dvec3());
                let normal = if normal.length_squared() > 0.0 {
                    normal.normalize()
                } else {
                    normal
                };
                let normal = normal.as_vec3();
                for component in normal.to_array() {
                    crate::finite_coordinate(component, "glTF", i)?;
                }
                v = v.with_normal(normal);
            }
        }
        if let Some(cs) = streams.colors {
            if i < cs.len() {
                v = v.with_color(cs[i]);
            }
        }
        if let Some(uvs) = streams.uvs {
            if i < uvs.len() {
                v = v.with_uv(uvs[i]);
            }
        }
        let h = builder.push_vertex(v);
        if i == 0 {
            first = h;
        }
    }
    Ok(first)
}

fn normal_transform_for(transform: Mat4) -> DMat4 {
    let transform = transform.as_dmat4();
    if transform.determinant() == 0.0 {
        transform
    } else {
        transform.inverse().transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normals_survive_extreme_nonsingular_scales() {
        for scale in [1.0e-20, 1.0e20, f32::MIN_POSITIVE] {
            let mut builder = MeshBuilder::new();
            builder_push_vertices(
                VertexStreams {
                    positions: &[[0.0, 0.0, 0.0]],
                    normals: Some(&[[1.0, 1.0, 0.0]]),
                    colors: None,
                    uvs: None,
                },
                Mat4::from_scale(Vec3::new(scale, scale * 2.0, scale)),
                &mut builder,
            )
            .expect("a finite nonsingular transform");
            let mesh = builder.build().expect("a point with a normal");
            let actual = Vec3::from_array(mesh.vertices()[0].normal);
            let expected = Vec3::new(2.0, 1.0, 0.0).normalize();
            assert!(
                actual.abs_diff_eq(expected, 1.0e-6),
                "scale {scale}: {actual:?}"
            );
        }
    }

    #[test]
    fn tiny_reflections_preserve_front_face_winding() {
        let doc: json::GltfDoc = serde_json::from_value(serde_json::json!({
            "buffers": [{ "byteLength": 36 }],
            "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
            "accessors": [{
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"
            }]
        }))
        .expect("a position stream");
        let primitive = json::Primitive {
            targets: Vec::new(),
            attributes: json::Attributes {
                position: Some(0),
                ..json::Attributes::default()
            },
            indices: None,
            mode: None,
            material: None,
        };
        let bytes: Vec<u8> = [Vec3::ZERO, Vec3::X, Vec3::Y]
            .into_iter()
            .flat_map(|p| p.to_array().into_iter().flat_map(f32::to_le_bytes))
            .collect();
        let mut builder = MeshBuilder::new();
        emit_primitive(
            &doc,
            &primitive,
            Mat4::from_scale(Vec3::new(-1.0e-20, 1.0e-20, 1.0e-20)),
            &bytes,
            &mut builder,
        )
        .expect("a finite reflection");
        let mesh = builder.build().expect("a reflected triangle");
        assert_eq!(mesh.indices(), [0, 2, 1]);
    }

    #[test]
    fn small_nonsingular_scales_use_inverse_transpose_normals() {
        let transform = Mat4::from_scale(Vec3::new(0.0001, 0.0002, 0.0001));
        let normal = Vec3::new(1.0, 1.0, 0.0).normalize();
        let actual = normal_transform_for(transform)
            .transform_vector3(normal.as_dvec3())
            .normalize()
            .as_vec3();
        let expected = Vec3::new(2.0, 1.0, 0.0).normalize();
        assert!(
            actual.abs_diff_eq(expected, 1.0e-6),
            "{actual:?} != {expected:?}"
        );
    }
}
