//! OBJ reader.
//!
//! Wavefront OBJ is a common export from intraoral scanners and dental CAD
//! software. The dental-relevant subset is small:
//!
//! - `v x y z [r g b]` - vertex position, optionally followed by 3 integer
//!   color channels in `0..=255` (a non-standard but widely-emitted extension;
//!   dental CAD software and several scanners write it). We honor those colors.
//! - `vt u [v]` - texture coordinate (parsed, currently unused).
//! - `vn x y z` - vertex normal (parsed and attached to the matching vertex).
//! - `f a b c ...` - polygonal face; indices are 1-based, may carry
//!   `/vt/vn` suffixes. We fan-triangulate polygons with `>3` corners.
//! - `p a b ...` - point element; a file with vertices but no faces is loaded
//!   as a point cloud, so an OBJ export of a point cloud keeps its geometry.
//! - `g`, `o`, `s`, `usemtl`, `mtllib`, `#` - group/object/smoothing/material
//!   directives; tolerated, not geometry-affecting for v1.
//!
//! ## Robustness rules (from the real corpus)
//!
//! - **1-based indexing**, with negative (relative) indices per spec.
//! - **Out-of-range indices are rejected** with `IndexOutOfRange` (the
//!   `ripoint-face-index-oob` corpus validates this; `bad_small.obj` has
//!   `f 1 4 3` with 3 vertices and must fail cleanly, never panic).
//! - **Lenient on unknown directives** - we skip lines we don't recognize
//!   rather than aborting. Dental CAD software's files carry many `#`
//!   metadata comments.
//! - **Vertex colors** are detected by counting tokens after `v`: 3 floats =
//!   position only, 6 = position + RGB (ints 0..=255).
//! - **No external file reads here**: `mtllib` is resolved after this parser
//!   has produced the mesh, by the companion loader the path-aware entry points
//!   call once the bytes are parsed (texture loading is a separate concern; v1
//!   attaches vertex colors only).

use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder};

mod parse;

/// Read an OBJ from raw bytes.
///
/// # Errors
/// - [`FormatError::Malformed`] for an unparseable line.
/// - [`FormatError::Core`] (`IndexOutOfRange`) for a face referencing an
///   out-of-range vertex (e.g. fuzz corpus `f 1 4 3` with 3 vertices).
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    read_shaded(bytes, crate::MeshShading::Reconstructed)
}

pub(crate) fn estimate_peak_bytes(bytes: &[u8]) -> u64 {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    };
    let mut positions = 0_u64;
    let mut normals = 0_u64;
    let mut texcoords = 0_u64;
    let mut face_corners = 0_u64;
    let mut triangle_count = 0_u64;
    let mut has_faces = false;

    for line in text.trim_start_matches('\u{feff}').lines() {
        let mut tokens = line
            .split('#')
            .next()
            .unwrap_or_default()
            .split_ascii_whitespace();
        match tokens.next() {
            Some("v") => positions = positions.saturating_add(1),
            Some("vn") => normals = normals.saturating_add(1),
            Some("vt") => texcoords = texcoords.saturating_add(1),
            Some("f") => {
                has_faces = true;
                let corners = u64::try_from(tokens.count()).unwrap_or(u64::MAX);
                face_corners = face_corners.saturating_add(corners);
                triangle_count = triangle_count.saturating_add(corners.saturating_sub(2));
            }
            _ => {}
        }
    }

    let output_vertices = if has_faces { face_corners } else { positions };
    let parser_bytes = positions
        .saturating_mul(16)
        .saturating_add(normals.saturating_mul(12))
        .saturating_add(texcoords.saturating_mul(8))
        .saturating_add(face_corners.saturating_mul(4))
        .saturating_add(output_vertices.saturating_mul(36))
        .saturating_add(triangle_count.saturating_mul(12));
    u64::try_from(bytes.len())
        .unwrap_or(u64::MAX)
        .saturating_add(parser_bytes.saturating_mul(2))
}

pub(crate) fn may_have_uvs(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| {
        text.trim_start_matches('\u{feff}').lines().any(|line| {
            line.split('#')
                .next()
                .unwrap_or_default()
                .split_ascii_whitespace()
                .next()
                == Some("vt")
        })
    })
}

/// As [`read`], choosing how vertex normals are produced.
///
/// # Errors
/// See [`read`].
pub fn read_shaded(bytes: &[u8], shading: crate::MeshShading) -> Result<Mesh, FormatError> {
    crate::memory::check_estimate(crate::memory::estimate_file_peak_bytes(
        crate::probe::FormatKind::Obj,
        bytes,
        0,
    )?)?;
    read_admitted(bytes, shading)
}

pub(crate) fn read_admitted(
    bytes: &[u8],
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    // OBJ is text; reject non-UTF-8 early with a clean error.
    let text = std::str::from_utf8(bytes).map_err(|_| FormatError::Malformed {
        format: "OBJ",
        offset: 0,
        reason: "file is not valid UTF-8".to_string(),
    })?;

    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    // Parallel to positions; true where a vertex carries a color.
    let mut colors: Vec<[u8; 4]> = Vec::new();
    let mut has_any_color = false;
    // Texture coordinates (vt lines).
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut builder = MeshBuilder::new()
        .with_name("OBJ")
        .from_input_of(bytes.len());
    let mut has_faces = false;

    for (line_no, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        // Strip comments: everything after the first '#' that is not in a
        // quoted string. Dental CAD software's files are comment-heavy.
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_ascii_whitespace();
        let Some(tag) = tokens.next() else {
            continue;
        };

        match tag {
            "v" => {
                let (pos, color) = parse::vertex_line(&mut tokens, line_no, line)?;
                positions.push(pos);
                if let Some(c) = color {
                    has_any_color = true;
                    colors.push(c);
                } else {
                    colors.push([255, 255, 255, 255]);
                }
            }
            "vn" => {
                let n = parse::normal_line(&mut tokens, line_no, line)?;
                normals.push(n);
            }
            "vt" => {
                if let Some(uv) = parse::texcoord_line(&mut tokens, line_no, line) {
                    texcoords.push(uv);
                }
            }
            // The directives below carry no geometry for v1.
            // We list recognized-but-ignored directives explicitly (rather than
            // folding them into `_`) so the source documents which OBJ features
            // we have *chosen* to skip vs. which are genuinely unknown.
            #[allow(clippy::match_same_arms)]
            "g" | "o" | "s" | "usemtl" | "mtllib" | "newmtl" | "bevel" | "cstype" | "deg"
            | "curv" | "curv2" | "surf" | "parm" | "trim" | "hole" | "scrv" | "sp" | "end"
            | "con" | "bmat" | "step" => {}
            "f" => {
                has_faces = true;
                let data = parse::MeshData {
                    positions: &positions,
                    normals: &normals,
                    colors: &colors,
                    texcoords: &texcoords,
                };
                parse::face_line(&mut tokens, &data, &mut builder, line_no, line)?;
            }
            _ => {
                // Unknown directive: tolerated (OBJ has a long vendor tail).
            }
        }
    }

    // Many point-cloud OBJ writers emit only `v` records, while others add a
    // `p` record. In either form there are no face corners for the normal OBJ
    // path to materialize, so preserve the position payload explicitly.
    if !has_faces {
        for (index, position) in positions.iter().enumerate() {
            let mut vertex = occluview_core::Vertex::at(*position);
            if let Some(normal) = normals.get(index) {
                vertex = vertex.with_normal(*normal);
            }
            if let Some(color) = colors.get(index).copied() {
                if color != [255, 255, 255, 255] {
                    vertex = vertex.with_color(color);
                }
            }
            if let Some(uv) = texcoords.get(index).copied() {
                if uv != [0.0, 0.0] {
                    vertex = vertex.with_uv(uv);
                }
            }
            builder.push_vertex(vertex);
        }
        builder = builder.as_point_cloud();
    }

    let _ = has_any_color; // builder records colors per-vertex; nothing to do here.
    shading.build(builder).map_err(FormatError::Core)
}
