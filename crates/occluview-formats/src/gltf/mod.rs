//! glTF/GLB reader.
//!
//! Native Rust reader for the dental-viewer subset of glTF 2.0. Only the GLB
//! binary-container form is supported in v1 (the entire OccluTrace corpus is
//! GLB). External `.gltf` with separate buffer URIs is intentionally out of
//! scope for v1 — adding it later requires the path-traversal protection
//! described in SECURITY.md, not just parsing.
//!
//! Mesh subset: `POSITION` (FLOAT VEC3), `indices` (`UINT`/`USHORT`/`UBYTE`),
//! optional `NORMAL` (FLOAT VEC3), optional `COLOR_0` (FLOAT VEC3/VEC4 or
//! `UNSIGNED_BYTE` VEC3/VEC4). Primitive mode 4 (triangles) only.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

pub mod glb;
pub mod json;

mod accessor;
mod error;
mod primitive;
mod reader;
mod scene;
mod texture;

use crate::error::FormatError;
use occluview_core::Mesh;
/// Read a GLB from raw bytes into a [`Mesh`].
///
/// # Errors
/// - [`FormatError::BadSignature`] if not a GLB.
/// - [`FormatError::Malformed`] for invalid JSON or an unsupported feature.
/// - [`FormatError::Truncated`] for a buffer view past end of BIN chunk.
/// - [`FormatError::Core`] for index-out-of-range.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    if !bytes.starts_with(b"glTF") {
        return Err(FormatError::BadSignature {
            format: "glTF",
            offset: 0,
        });
    }
    let (json_bytes, bin_chunk) = glb::split(bytes)?;
    let doc: json::GltfDoc =
        serde_json::from_slice(&json_bytes).map_err(|e| FormatError::Malformed {
            format: "glTF",
            offset: 0,
            reason: format!("invalid JSON: {e}"),
        })?;
    reader::read_doc(&doc, bin_chunk)
}
