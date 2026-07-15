//! PLY ASCII data reader.
//!
//! Consumes the data section of a [`super::header::ParsedHeader`] (format =
//! Ascii), reading `vertex` and `face` elements in declaration order. We honor
//! the declared properties rather than hard-coding an order — dental scanners
//! emit wildly different property sets.
//!
//! PLY colors are non-negative integers clamped to `[0, 255]`; the `i32 -> u8`
//! narrowing is centralized here, so the corresponding clippy cast lint is
//! allowed at module scope.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use super::header::{Element, ParsedHeader, Property, ScalarType};
use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder, Vertex};

/// Plan for how to read one vertex row, derived from the `vertex` element's
/// declared properties. Each entry says "read a value of this type and route it
/// to this field of `Vertex`".
#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldPlan {
    /// Route to position component (index 0..3).
    Position(usize),
    /// Route to normal component (index 0..3).
    Normal(usize),
    /// Route to color channel (index 0..3 for r/g/b/a).
    Color(usize),
    /// Route to UV coordinate component (index 0..1).
    Uv(usize),
    /// Read but discard (unknown property like `confidence`).
    Skip,
}

impl FieldPlan {
    /// Build a plan from a vertex element's property list.
    pub(crate) fn plan_for(element: &Element) -> Vec<(FieldPlan, ScalarType)> {
        element
            .properties
            .iter()
            .filter_map(|p| match p {
                Property::Scalar { name, ty } => Some((route(name), *ty)),
                // A `list` property inside the vertex element is unusual but
                // allowed; we skip it (its length prefix would break the row).
                Property::List { .. } => None,
            })
            .collect()
    }
}

fn route(name: &str) -> FieldPlan {
    match name {
        "x" => FieldPlan::Position(0),
        "y" => FieldPlan::Position(1),
        "z" => FieldPlan::Position(2),
        "nx" => FieldPlan::Normal(0),
        "ny" => FieldPlan::Normal(1),
        "nz" => FieldPlan::Normal(2),
        "red" | "r" => FieldPlan::Color(0),
        "green" | "g" => FieldPlan::Color(1),
        "blue" | "b" => FieldPlan::Color(2),
        "alpha" | "a" => FieldPlan::Color(3),
        // UV property names. `s`/`t` are the common PLY UV names; `texture_u`/
        // `texture_v` are used by some scanners. We avoid mapping bare `u`/`v`
        // because they conflict with vertex-letter conventions in some files.
        "s" | "texture_u" | "tu" => FieldPlan::Uv(0),
        "t" | "texture_v" | "tv" => FieldPlan::Uv(1),
        _ => FieldPlan::Skip,
    }
}

/// Read an ASCII PLY into a [`Mesh`].
///
/// # Errors
/// Returns [`FormatError::Malformed`] for an unparseable token, or
/// [`FormatError::Truncated`] if the data section ends before all declared
/// rows are read.
pub fn read(parsed: &ParsedHeader<'_>) -> Result<Mesh, FormatError> {
    let data_text = std::str::from_utf8(parsed.data).map_err(|_| FormatError::Malformed {
        format: "PLY (ascii)",
        offset: 0,
        reason: "data section is not valid UTF-8".to_string(),
    })?;

    let mut tokens = data_text.split_ascii_whitespace();

    // Point cloud if the header declares no `element face`.
    let has_face = parsed.elements.iter().any(|e| e.name == "face");
    let builder_init = MeshBuilder::new().with_name("PLY");
    let mut builder = if has_face {
        builder_init
    } else {
        builder_init.as_point_cloud()
    };

    // Process elements in declaration order, consuming exactly `count` rows
    // for each. The vertex element provides positions/normals/colors; the face
    // element provides triangle indices.
    for element in &parsed.elements {
        match element.name.as_str() {
            "vertex" => read_vertices(&mut tokens, element, &mut builder)?,
            "face" => read_faces(&mut tokens, element, &mut builder)?,
            // Unknown element types (edge, etc.) — skip their tokens.
            other => skip_element(&mut tokens, element, other)?,
        }
    }

    builder.build().map_err(FormatError::Core)
}

fn read_vertices<'a, I>(
    tokens: &mut I,
    element: &Element,
    builder: &mut MeshBuilder,
) -> Result<(), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let plan = FieldPlan::plan_for(element);
    for _ in 0..element.count {
        let mut fields = VertexFields::default();
        for (route, ty) in &plan {
            let tok = tokens.next().ok_or(FormatError::Truncated {
                format: "PLY (ascii)",
                expected: element.count,
                got: 0,
            })?;
            apply_scalar(tok, *ty, *route, &mut fields)?;
        }
        let mut v = Vertex::at(Vec3::from_array(fields.position));
        if has_nonzero_normal(&fields.normal) {
            v = v.with_normal(Vec3::from_array(fields.normal));
        }
        if fields.color != [255, 255, 255, 255] {
            v = v.with_color(fields.color);
        }
        if fields.uv != [0.0, 0.0] {
            v = v.with_uv(fields.uv);
        }
        builder.push_vertex(v);
    }
    Ok(())
}

/// Mutable scratch space for routing property values into one vertex row.
#[derive(Clone, Debug)]
struct VertexFields {
    position: [f32; 3],
    normal: [f32; 3],
    color: [u8; 4],
    uv: [f32; 2],
}

impl Default for VertexFields {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            normal: [0.0; 3],
            // Default color is opaque white — matches Vertex's default.
            color: [255, 255, 255, 255],
            uv: [0.0, 0.0],
        }
    }
}

/// True if the normal field carries any non-zero component. We use this instead
/// of `!= [0.0; 3]` to avoid a strict float-array comparison (`clippy::float_cmp`
/// in array form) while preserving "no normal declared" detection.
fn has_nonzero_normal(normal: &[f32; 3]) -> bool {
    normal.iter().any(|&n| n != 0.0)
}

fn read_faces<'a, I>(
    tokens: &mut I,
    element: &Element,
    builder: &mut MeshBuilder,
) -> Result<(), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    // Find the vertex-indices list property (by name). Faces may carry multiple
    // list properties (real-world case: iTero and other textured scanners emit
    // `property list uchar int vertex_indices` PLUS `property list uchar float
    // texcoord`). We must consume every declared list per row or the next row's
    // tokens get misaligned.
    let Some(indices_prop_idx) = element
        .properties
        .iter()
        .position(|p| matches!(p, Property::List { name, .. } if name == "vertex_indices"))
    else {
        // No vertex-indices list on this element — skip its tokens entirely.
        return skip_element(tokens, element, "face");
    };

    for _ in 0..element.count {
        // Walk every declared property in order, consuming tokens. For the
        // vertex_indices list we fan-triangulate; for every other list we
        // discard its `count` elements.
        for (i, prop) in element.properties.iter().enumerate() {
            let Property::List { elem_ty, .. } = prop else {
                // Scalar properties on faces are rare but legal; skip one token.
                if tokens.next().is_none() {
                    return Err(FormatError::Truncated {
                        format: "PLY (ascii)",
                        expected: element.count,
                        got: 0,
                    });
                }
                continue;
            };
            // Count prefix.
            let count_tok = tokens.next().ok_or(FormatError::Truncated {
                format: "PLY (ascii)",
                expected: element.count,
                got: 0,
            })?;
            let n = parse_index(count_tok)?;
            if i == indices_prop_idx {
                // The geometry list — fan-triangulate.
                if n < 3 {
                    for _ in 0..n {
                        let _ = read_face_index(tokens)?;
                    }
                    continue;
                }
                let f0 = read_face_index(tokens)?;
                let mut prev_p = read_face_index(tokens)?;
                for _ in 2..n {
                    let cur = read_face_index(tokens)?;
                    builder.push_triangle(f0, prev_p, cur);
                    prev_p = cur;
                }
            } else {
                // Non-geometry list (texcoord, etc.) — discard n values.
                let _ = elem_ty;
                for _ in 0..n {
                    if tokens.next().is_none() {
                        return Err(FormatError::Truncated {
                            format: "PLY (ascii)",
                            expected: element.count,
                            got: 0,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Read one vertex-index token.
fn read_face_index<'a, I>(tokens: &mut I) -> Result<u32, FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let tok = tokens.next().ok_or(FormatError::Truncated {
        format: "PLY (ascii)",
        expected: 0,
        got: 0,
    })?;
    parse_index(tok)
}

fn skip_element<'a, I>(tokens: &mut I, element: &Element, name: &str) -> Result<(), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    // We can't know how many tokens an unknown element consumes without its
    // property layout. For scalar-only elements, it's count * properties.len();
    // for list elements, it's variable. We support scalar-only skipping; if a
    // list appears, we error (rare in practice for non-vertex/face elements).
    if element
        .properties
        .iter()
        .any(|p| matches!(p, Property::List { .. }))
    {
        return Err(FormatError::Malformed {
            format: "PLY (ascii)",
            offset: 0,
            reason: format!("cannot skip unknown list element {name:?}"),
        });
    }
    let tokens_to_skip = element.count * element.properties.len();
    for _ in 0..tokens_to_skip {
        if tokens.next().is_none() {
            return Err(FormatError::Truncated {
                format: "PLY (ascii)",
                expected: tokens_to_skip,
                got: 0,
            });
        }
    }
    Ok(())
}

/// Route one parsed property value into the right field of a `VertexFields`.
///
/// # Errors
/// Returns [`FormatError::Malformed`] if `tok` is not a valid scalar for `ty`.
fn apply_scalar(
    tok: &str,
    ty: ScalarType,
    route: FieldPlan,
    fields: &mut VertexFields,
) -> Result<(), FormatError> {
    match ty {
        ScalarType::Float | ScalarType::Double => {
            let v: f32 = tok.parse().map_err(|_| FormatError::Malformed {
                format: "PLY (ascii)",
                offset: 0,
                reason: format!("bad float: {tok:?}"),
            })?;
            match route {
                FieldPlan::Position(i) if i < 3 => fields.position[i] = v,
                FieldPlan::Normal(i) if i < 3 => fields.normal[i] = v,
                FieldPlan::Uv(i) if i < 2 => fields.uv[i] = v,
                _ => {}
            }
        }
        ScalarType::Uchar
        | ScalarType::Char
        | ScalarType::Ushort
        | ScalarType::Short
        | ScalarType::Uint
        | ScalarType::Int => {
            // Integer-valued; route colors here. (We could also accept
            // integer-valued positions, but PLY uses float for xyz.)
            if let FieldPlan::Color(i) = route {
                let v: i32 = tok.parse().map_err(|_| FormatError::Malformed {
                    format: "PLY (ascii)",
                    offset: 0,
                    reason: format!("bad integer: {tok:?}"),
                })?;
                if i < 4 {
                    fields.color[i] = v.clamp(0, 255) as u8;
                }
            }
        }
    }
    Ok(())
}

fn parse_index(tok: &str) -> Result<u32, FormatError> {
    tok.parse::<u32>().map_err(|_| FormatError::Malformed {
        format: "PLY (ascii)",
        offset: 0,
        reason: format!("bad index: {tok:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ply::header::{self, Format};

    fn parse_full(text: &str) -> ParsedHeader<'_> {
        let parsed = header::parse(text.as_bytes()).expect("header parses");
        assert_eq!(parsed.format, Format::Ascii, "test fixture must be ASCII");
        parsed
    }

    const COLORED_TRIANGLE: &str = "ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 1
property list uchar int vertex_indices
end_header
0 0 0 255 0 0
1 0 0 0 255 0
0 1 0 0 0 255
3 0 1 2\n";

    #[test]
    fn reads_colored_triangle() {
        let parsed = parse_full(COLORED_TRIANGLE);
        let mesh = read(&parsed).expect("valid");
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.has_vertex_colors());
        assert_eq!(mesh.vertices()[0].color, [255, 0, 0, 255]);
        assert_eq!(mesh.vertices()[1].color, [0, 255, 0, 255]);
        assert_eq!(mesh.vertices()[2].color, [0, 0, 255, 255]);
    }

    #[test]
    fn fan_triangulates_quads() {
        let ply = "ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar int vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
4 0 1 2 3\n";
        let parsed = parse_full(ply);
        let mesh = read(&parsed).expect("valid");
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn skips_unknown_vertex_properties() {
        // `confidence` is a non-standard property some scanners add.
        let ply = "ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property float confidence
end_header
1.0 2.0 3.0 0.9\n";
        let parsed = parse_full(ply);
        let mesh = read(&parsed).expect("valid");
        assert_eq!(mesh.vertices().len(), 1);
        assert_eq!(mesh.vertices()[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn reads_normals_when_present() {
        let ply = "ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property float nx
property float ny
property float nz
end_header
0 0 0 0 0 1\n";
        let parsed = parse_full(ply);
        let mesh = read(&parsed).expect("valid");
        assert_eq!(mesh.vertices()[0].normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn vertices_without_colors_default_white() {
        let ply = "ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
end_header
0 0 0\n";
        let parsed = parse_full(ply);
        let mesh = read(&parsed).expect("valid");
        assert!(!mesh.has_vertex_colors());
        assert_eq!(mesh.vertices()[0].color, [255, 255, 255, 255]);
    }

    #[test]
    fn face_with_texcoord_list_parses() {
        // Real iTero / textured-scan layout: each face has TWO list properties
        // - vertex_indices (the geometry) and texcoord (UV pairs). We must
        // consume the texcoord list so the next face row parses correctly,
        // instead of reading UV floats as vertex indices.
        let ply = "ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 2
property list uchar int vertex_indices
property list uchar float texcoord
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2 6 0.0 0.0 1.0 0.0 0.0 1.0
3 0 1 2 6 0.0 0.0 1.0 0.0 0.0 1.0\n";
        let parsed = parse_full(ply);
        let mesh = read(&parsed).expect("multi-list face must parse");
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn position_route_works() {
        let mut fields = VertexFields::default();
        apply_scalar(
            "1.0",
            ScalarType::Float,
            FieldPlan::Position(0),
            &mut fields,
        )
        .unwrap();
        assert_eq!(fields.position[0], 1.0);
    }

    #[test]
    fn color_clamps_out_of_range() {
        let mut fields = VertexFields {
            color: [0; 4],
            ..VertexFields::default()
        };
        apply_scalar("300", ScalarType::Uchar, FieldPlan::Color(0), &mut fields).unwrap();
        assert_eq!(fields.color[0], 255);
    }

    #[test]
    fn uv_route_populates_uv_field() {
        let mut fields = VertexFields::default();
        apply_scalar("0.75", ScalarType::Float, FieldPlan::Uv(0), &mut fields).unwrap();
        apply_scalar("0.25", ScalarType::Float, FieldPlan::Uv(1), &mut fields).unwrap();
        assert_eq!(fields.uv, [0.75, 0.25]);
    }

    #[test]
    fn ply_with_st_vertex_properties_reads_uvs() {
        // A PLY with `s` and `t` float vertex properties.
        let ply = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float s
property float t
element face 1
property list uchar uint vertex_indices
end_header
0 0 0 0.0 0.0
1 0 0 1.0 0.0
0 1 0 0.0 1.0
3 0 1 2
";
        let mesh = crate::ply::read(ply).expect("PLY should parse");
        assert!(mesh.has_uvs());
        let vs = mesh.vertices();
        assert_eq!(vs[0].uv, [0.0, 0.0]);
        assert_eq!(vs[1].uv, [1.0, 0.0]);
        assert_eq!(vs[2].uv, [0.0, 1.0]);
    }
}
