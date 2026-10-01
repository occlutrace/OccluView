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
#[cfg(test)]
mod tests;
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
    crate::memory::check_estimate(estimate_peak_bytes(bytes, 0)?)?;
    read_admitted(bytes)
}

pub(crate) fn read_admitted(bytes: &[u8]) -> Result<Mesh, FormatError> {
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

/// A channel the reader decodes into the built mesh.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DecodedChannel {
    Position,
    Normal,
    Color,
    TexCoord,
    Index,
}

/// Bytes one element of `channel` occupies once decoded.
///
/// These are the sizes of the types `accessor.rs` returns — `Vec3` for a
/// position or a normal, `[u8; 4]` for a colour, `[f32; 2]` for a texcoord, `u32`
/// for an index — and not the sizes the file stores. A `FLOAT` `VEC4` colour
/// arrives as sixteen bytes and is decoded to four, so four is the correct
/// number here; the bytes it arrived in are counted by `source_bytes` instead.
/// Deriving each from the decoded type keeps the estimate from drifting away
/// from what the reader actually builds.
fn decoded_element_bytes(channel: DecodedChannel) -> usize {
    match channel {
        DecodedChannel::Position | DecodedChannel::Normal => size_of::<[f32; 3]>(),
        DecodedChannel::Color => size_of::<[u8; 4]>(),
        DecodedChannel::TexCoord => size_of::<[f32; 2]>(),
        DecodedChannel::Index => size_of::<u32>(),
    }
}

/// Peak bytes the GLB reader may hold while it builds its mesh.
///
/// The model is the source bytes, slack for the JSON and BIN views, and the
/// decoded footprint of the largest single primitive. It is the number
/// `check_estimate` weighs against the import budget, so it must not under-count
/// what the reader materialises; see [`decoded_element_bytes`].
pub(crate) fn estimate_peak_bytes(bytes: &[u8], reserved_bytes: u64) -> Result<u64, FormatError> {
    if !bytes.starts_with(b"glTF") {
        return Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    }
    let (json_chunk, bin_chunk) = glb::split(bytes)?;
    let source_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let json_bytes = u64::try_from(json_chunk.len()).unwrap_or(u64::MAX);
    let bin_bytes = u64::try_from(bin_chunk.len()).unwrap_or(u64::MAX);
    let base_estimate = source_bytes
        .saturating_add(json_bytes.saturating_mul(4))
        .saturating_add(bin_bytes.saturating_mul(12));
    crate::memory::check_estimate(reserved_bytes.saturating_add(base_estimate))?;

    let doc: json::GltfDoc =
        serde_json::from_slice(&json_chunk).map_err(|error| FormatError::Malformed {
            format: "glTF",
            offset: 0,
            reason: format!("invalid JSON: {error}"),
        })?;
    let largest_primitive =
        doc.meshes
            .iter()
            .flat_map(|mesh| &mesh.primitives)
            .fold(0_u64, |largest, primitive| {
                let stream_bytes = [
                    (primitive.attributes.position, DecodedChannel::Position),
                    (primitive.attributes.normal, DecodedChannel::Normal),
                    (primitive.attributes.color_0, DecodedChannel::Color),
                    (primitive.attributes.texcoord_0, DecodedChannel::TexCoord),
                    (primitive.indices, DecodedChannel::Index),
                ]
                .into_iter()
                .fold(0_u64, |total, (accessor, channel)| {
                    accessor
                        .and_then(|index| doc.accessors.get(index))
                        .map_or(total, |accessor| {
                            total.saturating_add(
                                u64::try_from(accessor.count)
                                    .unwrap_or(u64::MAX)
                                    .saturating_mul(
                                        u64::try_from(decoded_element_bytes(channel))
                                            .unwrap_or(u64::MAX),
                                    ),
                            )
                        })
                });
                largest.max(stream_bytes)
            });
    Ok(base_estimate.saturating_add(largest_primitive))
}
