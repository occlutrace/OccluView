//! OBJ line parsers - the per-directive helpers used by [`super::read`].
//!
//! Each helper consumes from a `split_ascii_whitespace` iterator and returns a
//! typed value or a `FormatError` carrying the source line number for context.
//!
//! OBJ indices are 1-based integers; we narrow them to `usize`/`u32` here, so
//! the corresponding clippy cast lints are allowed at module scope.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss
)]

use crate::error::FormatError;
use glam::Vec3;
use occluview_core::MeshBuilder;

/// Parse a `v` line. Returns `(position, optional_rgb_color)`.
///
/// Color is detected by counting tokens: 3 floats = position only; 6 values
/// = position + integer RGB (the exocad/dental extension).
pub(super) fn vertex_line<'a, I>(
    tokens: &mut I,
    line_no: usize,
    raw: &str,
) -> Result<(Vec3, Option<[u8; 4]>), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let px = next_f32(tokens, line_no, raw)?;
    let py = next_f32(tokens, line_no, raw)?;
    let pz = next_f32(tokens, line_no, raw)?;
    let pos = Vec3::new(px, py, pz);

    // Optional color: up to 3 more integer tokens (RGB). Some writers add a
    // 4th alpha; we read 3 and ignore any extras beyond.
    let cr = tokens.next();
    let cg = tokens.next();
    let cb = tokens.next();
    let color = match (cr, cg, cb) {
        (Some(rk), Some(gk), Some(bk)) => {
            let r = parse_color_channel(rk, line_no, raw)?;
            let g = parse_color_channel(gk, line_no, raw)?;
            let b = parse_color_channel(bk, line_no, raw)?;
            Some([r, g, b, 255])
        }
        _ => None,
    };
    Ok((pos, color))
}

/// Parse a `vn` line.
pub(super) fn normal_line<'a, I>(
    tokens: &mut I,
    line_no: usize,
    raw: &str,
) -> Result<Vec3, FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let nx = next_f32(tokens, line_no, raw)?;
    let ny = next_f32(tokens, line_no, raw)?;
    let nz = next_f32(tokens, line_no, raw)?;
    Ok(Vec3::new(nx, ny, nz))
}

/// Parse a `vt` line. Returns `[u, v]`. A `vt` with only one component is
/// tolerated (v defaults to 0). Returns `None` if parsing fails — a malformed
/// texcoord should not abort the whole file (OBJ files often have spurious vt).
pub(super) fn texcoord_line<'a, I>(tokens: &mut I, line_no: usize, raw: &str) -> Option<[f32; 2]>
where
    I: Iterator<Item = &'a str>,
{
    let u = next_f32(tokens, line_no, raw).ok()?;
    // The v component is optional per the spec; default 0.
    let v = next_f32(tokens, line_no, raw).unwrap_or(0.0);
    Some([u, v])
}

/// Parsed-so-far mesh data, passed into `face_line` to resolve indices.
pub(super) struct MeshData<'a> {
    pub positions: &'a [Vec3],
    pub normals: &'a [Vec3],
    pub colors: &'a [[u8; 4]],
    pub texcoords: &'a [[f32; 2]],
}

/// Parse an `f` line. Fan-triangulates polygons with >3 corners. Each face
/// vertex may be `v`, `v/vt`, `v//vn`, or `v/vt/vn`.
///
/// # Errors
/// - [`FormatError::Core`](`occluview_core::CoreError::IndexOutOfRange`) if any
///   index is out of range (positive beyond vertex count, or negative beyond
///   the start).
pub(super) fn face_line<'a, I>(
    tokens: &mut I,
    data: &MeshData<'_>,
    builder: &mut MeshBuilder,
    line_no: usize,
    raw: &str,
) -> Result<(), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let positions = data.positions;
    let normals = data.normals;
    let colors = data.colors;
    let texcoords = data.texcoords;

    // Collect the resolved vertex indices (into our `positions` slice) for the
    // whole face. Fan-triangulate once we have them.
    let mut face_vertex_idxs: Vec<u32> = Vec::with_capacity(4);
    for tok in tokens {
        // A face vertex token may contain slashes: `v`, `v/vt`, `v//vn`, `v/vt/vn`.
        let mut parts = tok.split('/');
        let v_str = parts.next().unwrap_or("");
        let texcoord_idx_str = parts.next();
        let normal_idx_str = parts.next();

        let v_idx = resolve_index(v_str, positions.len(), line_no, raw)?;
        // normal_idx_str may be None (no normal on this face vertex) or Some("")
        // (e.g. `v//vt` shape). Both mean: no normal.
        let normal = match normal_idx_str {
            Some(s) if !s.is_empty() => resolve_normal(s, normals),
            _ => None,
        };
        // texcoord_idx_str follows the same lenient resolve as normals.
        let uv = match texcoord_idx_str {
            Some(s) if !s.is_empty() => resolve_texcoord(s, texcoords),
            _ => None,
        };

        let pos = positions[v_idx];
        let color = colors.get(v_idx).copied().unwrap_or([255, 255, 255, 255]);
        let mut v = occluview_core::Vertex::at(pos);
        if let Some(n) = normal {
            v = v.with_normal(n);
        }
        if color != [255, 255, 255, 255] {
            v = v.with_color(color);
        }
        if let Some(uv) = uv {
            v = v.with_uv(uv);
        }
        let h = builder.push_vertex(v);
        face_vertex_idxs.push(h);
    }

    if face_vertex_idxs.len() < 3 {
        return Err(FormatError::Malformed {
            format: "OBJ",
            offset: line_no,
            reason: format!("face with < 3 vertices: {raw:?}"),
        });
    }

    // Fan triangulate: (v0, vi, vi+1).
    let f0 = face_vertex_idxs[0];
    for window in face_vertex_idxs[1..].windows(2) {
        builder.push_triangle(f0, window[0], window[1]);
    }
    Ok(())
}

/// Resolve a `vn` index string (1-based positive, or negative relative) to a
/// normal vector. Returns `None` if the index is missing or out of range; in
/// that case the face vertex is emitted without a normal.
fn resolve_normal(s: &str, normals: &[Vec3]) -> Option<Vec3> {
    let n: i64 = s.parse().ok()?;
    let n_idx = match n.cmp(&0) {
        std::cmp::Ordering::Greater => (n - 1) as usize,
        std::cmp::Ordering::Less => normals.len().checked_sub(n.unsigned_abs() as usize)?,
        std::cmp::Ordering::Equal => return None,
    };
    normals.get(n_idx).copied()
}

/// Resolve a `vt` index string (1-based positive, or negative relative) to a
/// UV pair. Returns `None` if the index is missing or out of range; in that
/// case the face vertex is emitted without a UV.
fn resolve_texcoord(s: &str, texcoords: &[[f32; 2]]) -> Option<[f32; 2]> {
    let n: i64 = s.parse().ok()?;
    let idx = match n.cmp(&0) {
        std::cmp::Ordering::Greater => (n - 1) as usize,
        std::cmp::Ordering::Less => texcoords.len().checked_sub(n.unsigned_abs() as usize)?,
        std::cmp::Ordering::Equal => return None,
    };
    texcoords.get(idx).copied()
}

/// Resolve a single OBJ index string (1-based positive, or negative relative)
/// into a 0-based index into `len`-sized array. Empty string is an error.
fn resolve_index(s: &str, len: usize, line_no: usize, raw: &str) -> Result<usize, FormatError> {
    let n: i64 = s.parse().map_err(|_| FormatError::Malformed {
        format: "OBJ",
        offset: line_no,
        reason: format!("bad index {s:?} in: {raw:?}"),
    })?;
    let idx = match n.cmp(&0) {
        std::cmp::Ordering::Greater => (n - 1) as usize,
        std::cmp::Ordering::Less => {
            // Negative = relative to current end of vertex list.
            len.checked_sub(n.unsigned_abs() as usize)
                .ok_or_else(|| FormatError::Malformed {
                    format: "OBJ",
                    offset: line_no,
                    reason: format!("negative index {n} underflows (len={len}): {raw:?}"),
                })?
        }
        std::cmp::Ordering::Equal => {
            return Err(FormatError::Malformed {
                format: "OBJ",
                offset: line_no,
                reason: format!("zero is not a valid OBJ index: {raw:?}"),
            });
        }
    };
    if idx >= len {
        return Err(FormatError::Core(
            occluview_core::CoreError::IndexOutOfRange {
                at_index: line_no,
                value: idx as u32,
                vertex_count: len as u32,
            },
        ));
    }
    Ok(idx)
}

fn next_f32<'a, I>(tokens: &mut I, line_no: usize, raw: &str) -> Result<f32, FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let tok = tokens.next().ok_or_else(|| FormatError::Malformed {
        format: "OBJ",
        offset: line_no,
        reason: format!("missing number in: {raw:?}"),
    })?;
    tok.parse::<f32>().map_err(|_| FormatError::Malformed {
        format: "OBJ",
        offset: line_no,
        reason: format!("bad number {tok:?} in: {raw:?}"),
    })
}

fn parse_color_channel(s: &str, line_no: usize, raw: &str) -> Result<u8, FormatError> {
    // Two real-world conventions for OBJ vertex colors:
    //   1. Integer 0..=255 (exocad DentalCAD, most dental scanners).
    //   2. Float 0.0..=1.0 (some CAD tools, research datasets like CrossTooth).
    // Detect by whether the token parses as int; if not, try float and scale.
    if let Ok(v) = s.parse::<i32>() {
        return Ok(v.clamp(0, 255) as u8);
    }
    if let Ok(f) = s.parse::<f32>() {
        // Scale 0.0..=1.0 to 0..=255. Values slightly out of range clamp.
        return Ok((f.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    Err(FormatError::Malformed {
        format: "OBJ",
        offset: line_no,
        reason: format!("bad color channel {s:?} in: {raw:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj;

    fn read_obj(text: &str) -> occluview_core::Mesh {
        obj::read(text.as_bytes()).expect("OBJ should parse")
    }

    #[test]
    fn minimal_triangle() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        assert_eq!(m.triangle_count(), 1);
        assert_eq!(m.vertices().len(), 3);
        assert_eq!(m.vertices()[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn utf8_bom_before_first_vertex_is_ignored() {
        let m = read_obj("\u{feff}v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");

        assert_eq!(m.triangle_count(), 1);
        assert_eq!(m.vertices().len(), 3);
        assert_eq!(m.vertices()[0].position, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn vertex_colors_via_six_token_v() {
        // exocad extension: v x y z r g b (integer 0..=255).
        let m = read_obj("v 0 0 0 255 0 0\nv 1 0 0 0 255 0\nv 0 1 0 0 0 255\nf 1 2 3\n");
        assert!(m.has_vertex_colors());
        assert_eq!(m.vertices()[0].color, [255, 0, 0, 255]);
        assert_eq!(m.vertices()[1].color, [0, 255, 0, 255]);
    }

    #[test]
    fn vertex_colors_via_float_0_to_1() {
        // Research dataset (CrossTooth) convention: v x y z r g b with floats.
        let m =
            read_obj("v 0 0 0 1.0 0.0 0.0\nv 1 0 0 0.0 1.0 0.0\nv 0 1 0 0.0 0.0 1.0\nf 1 2 3\n");
        assert!(m.has_vertex_colors());
        assert_eq!(m.vertices()[0].color, [255, 0, 0, 255]);
        assert_eq!(m.vertices()[1].color, [0, 255, 0, 255]);
        assert_eq!(m.vertices()[2].color, [0, 0, 255, 255]);
    }

    #[test]
    fn quad_fan_triangulates() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n");
        assert_eq!(m.triangle_count(), 2);
    }

    #[test]
    fn face_with_normal_indices() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//1 2//1 3//1\n");
        for v in m.vertices() {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn face_with_vt_and_vn() {
        let m = read_obj(
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n",
        );
        assert_eq!(m.triangle_count(), 1);
        assert!(m.has_uvs());
        let vs = m.vertices();
        assert_eq!(vs[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(vs[1].normal, [0.0, 0.0, 1.0]);
        assert_eq!(vs[2].normal, [0.0, 0.0, 1.0]);
        // UVs resolved: vt 0=(0,0), vt 1=(1,0), vt 2=(0,1).
        assert_eq!(vs[0].uv, [0.0, 0.0]);
        assert_eq!(vs[1].uv, [1.0, 0.0]);
        assert_eq!(vs[2].uv, [0.0, 1.0]);
    }

    #[test]
    fn face_with_vt_only() {
        // `v/vt` form (no normal).
        let m = read_obj(
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.75\nvt 0.5 0.5\nvt 0.75 0.25\nf 1/1 2/2 3/3\n",
        );
        assert!(m.has_uvs());
        let vs = m.vertices();
        assert_eq!(vs[0].uv, [0.25, 0.75]);
        assert_eq!(vs[1].uv, [0.5, 0.5]);
        assert_eq!(vs[2].uv, [0.75, 0.25]);
    }

    #[test]
    fn face_without_vt_has_no_uvs() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        assert!(!m.has_uvs());
    }

    #[test]
    fn out_of_range_index_is_rejected() {
        // bad_small.obj from the ripoint-face-index-oob fuzz corpus.
        let err = obj::read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 4 3\n").unwrap_err();
        assert!(matches!(
            err,
            FormatError::Core(occluview_core::CoreError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn negative_relative_indices() {
        // Spec: negative indices are relative to the end of the current list.
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n");
        assert_eq!(m.triangle_count(), 1);
        assert_eq!(m.vertices()[0].position, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn zero_index_rejected() {
        let err = obj::read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n").unwrap_err();
        assert!(matches!(err, FormatError::Malformed { .. }));
    }

    #[test]
    fn comments_and_unknown_directives_tolerated() {
        // exocad files are full of metadata comments.
        let text = "# exocad GmbH - DentalCAD v8349
# DentalBase-OBJ-File Version: 2.7
# Mesh: (3 Vertices, 1 Triangles)
o \"upper\"
s on
g ID0 NID0
custom_directive foo bar
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3\n";
        let m = read_obj(text);
        assert_eq!(m.triangle_count(), 1);
    }

    #[test]
    fn non_utf8_rejected() {
        let mut bytes = b"v 0 0 0\n".to_vec();
        bytes.push(0xFF);
        assert!(obj::read(&bytes).is_err());
    }

    #[test]
    fn resolves_one_based_and_negative_indices() {
        // Local unit test for resolve_index, covering both branches.
        assert_eq!(resolve_index("1", 5, 0, "").unwrap(), 0);
        assert_eq!(resolve_index("5", 5, 0, "").unwrap(), 4);
        assert_eq!(resolve_index("-1", 5, 0, "").unwrap(), 4);
        assert_eq!(resolve_index("-5", 5, 0, "").unwrap(), 0);
        assert!(resolve_index("6", 5, 0, "").is_err());
        assert!(resolve_index("0", 5, 0, "").is_err());
    }
}
