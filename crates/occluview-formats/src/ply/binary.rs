//! PLY binary data reader (little- and big-endian).
//!
//! PLY indices and polygon-counts are non-negative integers per spec, but the
//! declared scalar type can be any of int/uint/short/uchar. We centralize the
//! `i64 -> u32` / `f32 -> u32` narrowing here, so we allow the corresponding
//! clippy cast lints at module scope rather than at each call site.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
//!
//! Same vertex/face logic as [`super::ascii`], but reads packed binary records.
//! Endianness comes from the header's `format` line; we honor it rather than
//! assuming the host's native order, so a big-endian PLY read correctly on a
//! little-endian machine.

use super::ascii::FieldPlan;
use super::header::{Element, ParsedHeader, Property, ScalarType};
use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder, Vertex};

/// Read a little-endian binary PLY.
///
/// # Errors
/// Returns [`FormatError::Truncated`] if the data section ends mid-record, or
/// [`FormatError::Malformed`] for a structurally invalid element/property.
pub fn read_le(parsed: &ParsedHeader<'_>) -> Result<Mesh, FormatError> {
    read_with(parsed, Endian::Little)
}

/// Read a big-endian binary PLY.
///
/// # Errors
/// Same as [`read_le`].
pub fn read_be(parsed: &ParsedHeader<'_>) -> Result<Mesh, FormatError> {
    read_with(parsed, Endian::Big)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Endian {
    Little,
    Big,
}

/// Cursor over the data section, tracking position and endianness.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    endian: Endian,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self {
            bytes,
            pos: 0,
            endian,
        }
    }

    fn take(&mut self, n: usize, ctx: &str) -> Result<&'a [u8], FormatError> {
        if self.pos + n > self.bytes.len() {
            return Err(FormatError::Truncated {
                format: "PLY (binary)",
                expected: self.pos + n,
                got: self.bytes.len(),
            });
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        let _ = ctx;
        Ok(slice)
    }

    fn read_scalar(&mut self, ty: ScalarType) -> Result<ScalarValue, FormatError> {
        // `take_array<N>` reads exactly N bytes and returns a fixed-size array,
        // so the rest of this function is panic-free and `unwrap`-free.
        Ok(match ty {
            // 1-byte types: endianness is irrelevant.
            ScalarType::Char => {
                // Reinterpret the byte as signed; `from_ne_bytes` is the
                // standard pattern and avoids a `as i8` cast.
                ScalarValue::Int(i64::from(i8::from_ne_bytes([self.take_byte()?])))
            }
            ScalarType::Uchar => ScalarValue::Int(i64::from(self.take_byte()?)),
            ScalarType::Short => {
                let a = self.take_array::<2>()?;
                ScalarValue::Int(i64::from(self.endian.read_i16(a)))
            }
            ScalarType::Ushort => {
                let a = self.take_array::<2>()?;
                ScalarValue::Int(i64::from(self.endian.read_u16(a)))
            }
            ScalarType::Int => {
                let a = self.take_array::<4>()?;
                ScalarValue::Int(i64::from(self.endian.read_i32(a)))
            }
            ScalarType::Uint => {
                let a = self.take_array::<4>()?;
                ScalarValue::Int(i64::from(self.endian.read_u32(a)))
            }
            ScalarType::Float => {
                let a = self.take_array::<4>()?;
                ScalarValue::Float(self.endian.read_f32(a))
            }
            // Double -> f32 is a deliberately lossy narrowing; PLY doubles are
            // vanishingly rare in dental files, and our internal type is f32.
            ScalarType::Double => {
                let a = self.take_array::<8>()?;
                ScalarValue::Float(self.endian.read_f64(a) as f32)
            }
        })
    }

    /// Read exactly one byte.
    fn take_byte(&mut self) -> Result<u8, FormatError> {
        Ok(*self
            .take(1, "byte")?
            .first()
            .ok_or(FormatError::Malformed {
                format: "PLY (binary)",
                offset: self.pos,
                reason: "expected 1 byte, got 0".to_string(),
            })?)
    }

    /// Read exactly N bytes as a fixed-size array. Infallible `try_into` because
    /// `take(N)` already guaranteed the length.
    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], FormatError> {
        self.take(N, "array")?
            .try_into()
            .map_err(|_| FormatError::Malformed {
                format: "PLY (binary)",
                offset: self.pos,
                reason: format!("expected {N} bytes"),
            })
    }
}

impl Endian {
    fn read_i16(self, a: [u8; 2]) -> i16 {
        match self {
            Endian::Little => i16::from_le_bytes(a),
            Endian::Big => i16::from_be_bytes(a),
        }
    }
    fn read_u16(self, a: [u8; 2]) -> u16 {
        match self {
            Endian::Little => u16::from_le_bytes(a),
            Endian::Big => u16::from_be_bytes(a),
        }
    }
    fn read_i32(self, a: [u8; 4]) -> i32 {
        match self {
            Endian::Little => i32::from_le_bytes(a),
            Endian::Big => i32::from_be_bytes(a),
        }
    }
    fn read_u32(self, a: [u8; 4]) -> u32 {
        match self {
            Endian::Little => u32::from_le_bytes(a),
            Endian::Big => u32::from_be_bytes(a),
        }
    }
    fn read_f32(self, a: [u8; 4]) -> f32 {
        match self {
            Endian::Little => f32::from_le_bytes(a),
            Endian::Big => f32::from_be_bytes(a),
        }
    }
    fn read_f64(self, a: [u8; 8]) -> f64 {
        match self {
            Endian::Little => f64::from_le_bytes(a),
            Endian::Big => f64::from_be_bytes(a),
        }
    }
}

/// A scalar value tagged as integer-ish or float-ish — lets us route colors
/// (integers) separately from positions (floats) without knowing the declared
/// type at the call site.
#[derive(Clone, Copy, Debug)]
enum ScalarValue {
    Int(i64),
    Float(f32),
}

fn read_with(parsed: &ParsedHeader<'_>, endian: Endian) -> Result<Mesh, FormatError> {
    let mut cursor = Cursor::new(parsed.data, endian);
    let has_face = parsed.elements.iter().any(|e| e.name == "face");
    let builder_init = MeshBuilder::new().with_name("PLY");
    let mut builder = if has_face {
        builder_init
    } else {
        builder_init.as_point_cloud()
    };

    for element in &parsed.elements {
        match element.name.as_str() {
            "vertex" => read_vertices(&mut cursor, element, &mut builder)?,
            "face" => read_faces(&mut cursor, element, &mut builder)?,
            other => {
                return Err(FormatError::Malformed {
                    format: "PLY (binary)",
                    offset: cursor.pos,
                    reason: format!("cannot skip unknown binary element {other:?}"),
                })
            }
        }
    }

    builder.build().map_err(FormatError::Core)
}

fn read_vertices(
    cursor: &mut Cursor<'_>,
    element: &Element,
    builder: &mut MeshBuilder,
) -> Result<(), FormatError> {
    let plan = FieldPlan::plan_for(element);
    for _ in 0..element.count {
        let mut position = [0.0_f32; 3];
        let mut normal = [0.0_f32; 3];
        let mut color = [255u8; 4];
        let mut uv = [0.0_f32; 2];
        for (route, ty) in &plan {
            let v = cursor.read_scalar(*ty)?;
            route_value(v, *route, &mut position, &mut normal, &mut color, &mut uv);
        }
        let mut vert = Vertex::at(Vec3::from_array(position));
        if normal.iter().any(|&n| n != 0.0) {
            vert = vert.with_normal(Vec3::from_array(normal));
        }
        if color != [255, 255, 255, 255] {
            vert = vert.with_color(color);
        }
        if uv != [0.0, 0.0] {
            vert = vert.with_uv(uv);
        }
        builder.push_vertex(vert);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn route_value(
    v: ScalarValue,
    route: FieldPlan,
    position: &mut [f32; 3],
    normal: &mut [f32; 3],
    color: &mut [u8; 4],
    uv: &mut [f32; 2],
) {
    match (v, route) {
        (ScalarValue::Float(f), FieldPlan::Position(i)) if i < 3 => position[i] = f,
        (ScalarValue::Float(f), FieldPlan::Normal(i)) if i < 3 => normal[i] = f,
        (ScalarValue::Float(f), FieldPlan::Uv(i)) if i < 2 => uv[i] = f,
        (ScalarValue::Int(n), FieldPlan::Color(i)) if i < 4 => {
            color[i] = n.clamp(0, 255) as u8;
        }
        _ => {}
    }
}

fn read_faces(
    cursor: &mut Cursor<'_>,
    element: &Element,
    builder: &mut MeshBuilder,
) -> Result<(), FormatError> {
    // Locate the vertex_indices list (by name). Faces may carry multiple list
    // properties (e.g. iTero: `vertex_indices` + `texcoord`); we must consume
    // every declared list per row so the next row's bytes line up.
    let Some(indices_prop_idx) = element
        .properties
        .iter()
        .position(|p| matches!(p, Property::List { name, .. } if name == "vertex_indices"))
    else {
        return Ok(()); // No vertex_indices list — nothing to triangulate.
    };

    for _ in 0..element.count {
        // Walk every declared property in order, consuming bytes. For the
        // vertex_indices list we fan-triangulate; for every other list we
        // discard its `count` elements.
        for (i, prop) in element.properties.iter().enumerate() {
            let Property::List {
                count_ty, elem_ty, ..
            } = prop
            else {
                let Property::Scalar { ty, .. } = prop else {
                    continue;
                };
                let _ = cursor.read_scalar(*ty)?;
                continue;
            };
            let poly_n = match cursor.read_scalar(*count_ty)? {
                ScalarValue::Int(n) => n.max(0) as u32,
                ScalarValue::Float(f) => f.max(0.0) as u32,
            };
            if i == indices_prop_idx {
                // The geometry list — fan-triangulate.
                if poly_n < 3 {
                    for _ in 0..poly_n {
                        let _ = read_index(cursor, *elem_ty)?;
                    }
                    continue;
                }
                let first = read_index(cursor, *elem_ty)?;
                let mut prev = read_index(cursor, *elem_ty)?;
                for _ in 2..poly_n {
                    let cur = read_index(cursor, *elem_ty)?;
                    builder.push_triangle(first, prev, cur);
                    prev = cur;
                }
            } else {
                // Non-geometry list (texcoord, etc.) — discard n values.
                for _ in 0..poly_n {
                    let _ = cursor.read_scalar(*elem_ty)?;
                }
            }
        }
    }
    Ok(())
}

fn read_index(cursor: &mut Cursor<'_>, ty: ScalarType) -> Result<u32, FormatError> {
    match cursor.read_scalar(ty)? {
        ScalarValue::Int(n) => Ok(n.max(0) as u32),
        ScalarValue::Float(f) => Ok(f.max(0.0) as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ply::header::{self, Format};

    fn colored_triangle_le() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"ply\nformat binary_little_endian 1.0\n");
        out.extend_from_slice(b"element vertex 3\n");
        out.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        out.extend_from_slice(b"property uchar red\nproperty uchar green\nproperty uchar blue\n");
        out.extend_from_slice(b"element face 1\n");
        out.extend_from_slice(b"property list uchar int vertex_indices\n");
        out.extend_from_slice(b"end_header\n");
        // 3 vertices: (0,0,0) red, (1,0,0) green, (0,1,0) blue.
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.push(255);
        out.push(0);
        out.push(0);
        out.extend_from_slice(&1.0_f32.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.push(0);
        out.push(255);
        out.push(0);
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.extend_from_slice(&1.0_f32.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.push(0);
        out.push(0);
        out.push(255);
        // One face: 3 indices.
        out.push(3);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out
    }

    #[test]
    fn reads_binary_le_colored_triangle() {
        let bytes = colored_triangle_le();
        let parsed = header::parse(&bytes).expect("header");
        assert_eq!(parsed.format, Format::BinaryLittleEndian);
        let mesh = read_le(&parsed).expect("valid");
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.has_vertex_colors());
        assert_eq!(mesh.vertices()[0].color, [255, 0, 0, 255]);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn reads_binary_be_colored_triangle() {
        // Same data, big-endian, format line changed.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ply\nformat binary_big_endian 1.0\n");
        bytes.extend_from_slice(b"element vertex 1\n");
        bytes.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        bytes.extend_from_slice(b"end_header\n");
        bytes.extend_from_slice(&1.0_f32.to_be_bytes());
        bytes.extend_from_slice(&2.0_f32.to_be_bytes());
        bytes.extend_from_slice(&3.0_f32.to_be_bytes());
        let parsed = header::parse(&bytes).expect("header");
        assert_eq!(parsed.format, Format::BinaryBigEndian);
        let mesh = read_be(&parsed).expect("valid");
        assert_eq!(mesh.vertices()[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn detects_truncation_in_vertex_data() {
        // Header declares 3 vertices but data has only 1.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ply\nformat binary_little_endian 1.0\n");
        bytes.extend_from_slice(b"element vertex 3\n");
        bytes.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        bytes.extend_from_slice(b"end_header\n");
        bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        bytes.extend_from_slice(&2.0_f32.to_le_bytes());
        bytes.extend_from_slice(&3.0_f32.to_le_bytes());
        let parsed = header::parse(&bytes).expect("header");
        let err = read_le(&parsed).unwrap_err();
        assert!(matches!(err, FormatError::Truncated { .. }));
    }

    #[test]
    fn quad_fan_triangulates_in_binary() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ply\nformat binary_little_endian 1.0\n");
        bytes.extend_from_slice(b"element vertex 4\n");
        bytes.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        bytes.extend_from_slice(b"element face 1\n");
        bytes.extend_from_slice(b"property list uchar int vertex_indices\n");
        bytes.extend_from_slice(b"end_header\n");
        for v in 0..4u32 {
            bytes.extend_from_slice(&(v as f32).to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        bytes.push(4); // polygon count
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        let parsed = header::parse(&bytes).expect("header");
        let mesh = read_le(&parsed).expect("valid");
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn consumes_face_scalar_properties_after_index_lists() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ply\nformat binary_little_endian 1.0\n");
        bytes.extend_from_slice(b"element vertex 4\n");
        bytes.extend_from_slice(b"property float x\nproperty float y\nproperty float z\n");
        bytes.extend_from_slice(b"element face 2\n");
        bytes.extend_from_slice(b"property list uchar int vertex_indices\n");
        bytes.extend_from_slice(b"property uchar confidence\n");
        bytes.extend_from_slice(b"end_header\n");
        for v in 0..4u32 {
            bytes.extend_from_slice(&(v as f32).to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
            bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        bytes.push(3);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.push(127);
        bytes.push(3);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.push(128);

        let parsed = header::parse(&bytes).expect("header");
        let mesh = read_le(&parsed).expect("valid");

        assert_eq!(mesh.triangle_count(), 2);
    }
}
