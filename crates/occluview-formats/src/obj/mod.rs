//! OBJ reader.
//!
//! Wavefront OBJ is a common export from intraoral scanners (notably Medit and
//! `exocad DentalCAD`). The dental-relevant subset is small:
//!
//! - `v x y z [r g b]` - vertex position, optionally followed by 3 integer
//!   color channels in `0..=255` (a non-standard but widely-emitted extension;
//!   exocad and several scanners write it). We honor those colors.
//! - `vt u [v]` - texture coordinate (parsed, currently unused).
//! - `vn x y z` - vertex normal (parsed and attached to the matching vertex).
//! - `f a b c ...` - polygonal face; indices are 1-based, may carry
//!   `/vt/vn` suffixes. We fan-triangulate polygons with `>3` corners.
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
//!   rather than aborting. exocad files carry many `#` metadata comments.
//! - **Vertex colors** are detected by counting tokens after `v`: 3 floats =
//!   position only, 6 = position + RGB (ints 0..=255).
//! - **No external file reads**: `mtllib` is recorded but the `.mtl` is not
//!   loaded at parse time (texture loading is a separate concern; v1 attaches
//!   vertex colors only).

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
    let mut builder = MeshBuilder::new().with_name("OBJ");

    for (line_no, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        // Strip comments: everything after the first '#' that is not in a
        // quoted string. exocad files are comment-heavy.
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

    let _ = has_any_color; // builder records colors per-vertex; nothing to do here.
    builder.build().map_err(FormatError::Core)
}
