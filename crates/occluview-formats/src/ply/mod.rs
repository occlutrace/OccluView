//! PLY reader.
//!
//! PLY (Polygon File Format) is the dental format for **color / NIR scans**:
//! unlike STL it supports per-vertex properties — most importantly `red green
//! blue` vertex colors. Both ASCII and binary (little- or big-endian) variants
//! exist; the header declares which.
//!
//! ## Layout
//!
//! ```text
//! ply
//! format ascii 1.0          # or: binary_little_endian 1.0 / binary_big_endian 1.0
//! comment ...
//! element vertex <N>
//!   property float x        # property <type> <name>
//!   property float y
//!   property float z
//!   property uchar red      # colors are usually 8-bit
//!   property uchar green
//!   property uchar blue
//!   ...
//! element face <M>
//!   property list uchar int vertex_indices
//! end_header
//! <data ...>
//! ```
//!
//! ## Dental-scanner quirks tolerated
//!
//! - Property order/format varies wildly across scanners — parse strictly from
//!   the header, never hard-code.
//! - Mixed endianness; honor the declared binary variant.
//! - Some scanners add non-standard properties (`confidence`, `nx ny nz`,
//!   `alpha`) — read and ignore unknown ones gracefully.
//! - Units sometimes declared in a `comment obj_info` line.

pub mod ascii;
pub mod binary;
pub(crate) mod embed;
pub mod header;

use crate::error::FormatError;
use occluview_core::{Mesh, MeshBuilder, MeshTexture};

/// Texture coordinates gathered from a face element's `texcoord` list.
///
/// PLY's working texture convention stores `u,v` per face corner (`CloudCompare`,
/// Agisoft and `MeshLab` all write it), and a corner's coordinates belong to the
/// vertex it indexes. The list cannot be turned into per-vertex data while the
/// vertices are being read — it arrives with the faces, after them — so it is
/// gathered here and applied to the builder before the mesh is finalized.
///
/// The first corner that names a vertex wins. A vertex whose corners disagree
/// has no single coordinate in a per-vertex model, and splitting it into
/// several vertices is a change the reader must not make behind the operator's
/// back.
#[derive(Default)]
pub(crate) struct FaceUvs {
    per_vertex: Vec<Option<[f32; 2]>>,
    /// Corners seen before the vertex element was read.
    ///
    /// A header may declare `face` before `vertex`, and the table is sized from
    /// real vertices rather than from anything the file says, so corners that
    /// arrive first wait here. Like the table, this is bounded by the rows
    /// actually consumed and never by a count in the file.
    pending: Vec<(u32, [f32; 2])>,
}

impl FaceUvs {
    /// Size the table for the vertices actually read.
    ///
    /// Called once the vertex element has been consumed, because a corner index
    /// in the face element is arbitrary input: sizing the table from the
    /// largest index in the file would let a 232-byte file ask for 51 GB, and the
    /// allocation failure that follows aborts a process uncatchably — in the
    /// Explorer thumbnail host that takes every other thumbnail with it. The
    /// declared vertex count is no better a bound, since a header can declare
    /// four billion vertices and carry none.
    pub(crate) fn reserve(&mut self, vertices: usize) {
        self.per_vertex.resize(vertices, None);
        for (vertex, uv) in std::mem::take(&mut self.pending) {
            self.set(vertex, uv);
        }
    }

    /// Record the coordinates of one face corner.
    ///
    /// A corner naming a vertex the builder never produced is dropped: the mesh
    /// cannot reference it either, and `Mesh::new` refuses the index later.
    pub(crate) fn set(&mut self, vertex: u32, uv: [f32; 2]) {
        if self.per_vertex.is_empty() {
            self.pending.push((vertex, uv));
            return;
        }
        let Ok(index) = usize::try_from(vertex) else {
            return;
        };
        if let Some(slot) = self.per_vertex.get_mut(index) {
            if slot.is_none() {
                *slot = Some(uv);
            }
        }
    }

    /// Move the gathered coordinates onto the vertices they belong to.
    pub(crate) fn apply(self, builder: &mut MeshBuilder) {
        for (index, uv) in self.per_vertex.into_iter().enumerate() {
            if let (Some(uv), Ok(index)) = (uv, u32::try_from(index)) {
                builder.set_vertex_uv(index, uv);
            }
        }
    }
}

/// Read a PLY from raw bytes.
///
/// Dispatches to ASCII or binary (LE/BE) based on the header's `format` line.
///
/// # Errors
/// See [`FormatError`]. Parsers never panic.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    read_shaded(bytes, crate::MeshShading::Reconstructed)
}

/// As [`read`], choosing how vertex normals are produced.
///
/// # Errors
/// See [`read`].
pub fn read_shaded(bytes: &[u8], shading: crate::MeshShading) -> Result<Mesh, FormatError> {
    crate::memory::check_estimate(crate::memory::estimate_file_peak_bytes(
        crate::probe::FormatKind::Ply,
        bytes,
        0,
    )?)?;
    read_admitted(bytes, shading)
}

pub(crate) fn read_admitted(
    bytes: &[u8],
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    // A UTF-8 BOM in front of `ply` is metadata a Windows tool added; without
    // this the signature check fails on an otherwise valid file.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let parsed = header::parse(bytes)?;
    let mut mesh = match parsed.format {
        header::Format::Ascii => ascii::read_shaded(&parsed, shading)?,
        header::Format::BinaryLittleEndian => binary::read_le_shaded(&parsed, shading)?,
        header::Format::BinaryBigEndian => binary::read_be_shaded(&parsed, shading)?,
    };
    // Only a mesh with coordinates can apply an image; attaching one to a mesh
    // without them paints a single flat texel over the whole layer.
    if mesh.has_uvs() {
        if let Some(texture) = embedded_texture(&parsed.texture) {
            mesh.set_texture(texture);
        }
    }
    Ok(mesh)
}

pub(crate) fn estimate_declared_bytes(bytes: &[u8]) -> Result<u64, FormatError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let parsed = header::parse(bytes)?;
    let empty_vertex_rows = parsed.elements.iter().any(|element| {
        element.name == "vertex"
            && element.count > 0
            && !element
                .properties
                .iter()
                .any(|property| matches!(property, header::Property::Scalar { .. }))
    });
    if empty_vertex_rows {
        // The reader rejects a row with no scalar input before it iterates or
        // allocates from the declared count, so that count adds no peak memory.
        return Ok(0);
    }
    Ok(declared_memory_bytes(&parsed))
}

pub(crate) fn may_have_uvs(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Ok(parsed) = header::parse(bytes) else {
        return false;
    };
    parsed.elements.iter().any(|element| {
        element.name == "vertex" && element.properties.iter().any(|property| {
            matches!(
                property,
                header::Property::Scalar { name, .. }
                    if matches!(name.as_str(), "s" | "t" | "texture_u" | "texture_v" | "tu" | "tv")
            )
        }) || element.name == "face"
            && element.properties.iter().any(|property| {
                matches!(
                    property,
                    header::Property::List { name, .. } if name == "texcoord"
                )
            })
    })
}

fn declared_memory_bytes(parsed: &header::ParsedHeader<'_>) -> u64 {
    parsed.elements.iter().fold(0_u64, |total, element| {
        let count = u64::try_from(element.count).unwrap_or(u64::MAX);
        let bytes_per_item = match element.name.as_str() {
            "vertex" => {
                size_of::<occluview_core::Vertex>().saturating_add(size_of::<Option<[f32; 2]>>())
            }
            "face" => size_of::<u32>().saturating_mul(3),
            _ => 0,
        };
        total
            .saturating_add(count.saturating_mul(u64::try_from(bytes_per_item).unwrap_or(u64::MAX)))
    })
}

/// The most base64 this reader will decode from a header comment.
///
/// Four thirds of the companion-image cap: the same picture budget, applied
/// before the pixels exist, so a header cannot make the reader allocate from
/// input length alone while the file buffer is still held.
pub(crate) fn max_encoded_chars() -> usize {
    let bytes = usize::try_from(crate::companions::MAX_COMPANION_IMAGE_BYTES).unwrap_or(0);
    bytes / 3 * 4
}

/// Decode the texture an export carried in its own header.
///
/// A header that names a file but carries nothing decodable yields `None`: the
/// image would be beside the file, and this reader deliberately takes bytes and
/// nothing else, so that a file another process is replacing mid-import cannot
/// change what was parsed. A corrupt embedded image is ignored rather than
/// raised — the geometry in the same file is still worth opening.
fn embedded_texture(comments: &header::TextureComments) -> Option<MeshTexture> {
    // The writer says what it put in the payload. A value this build does not
    // know is not decoded on a guess.
    if let Some(format) = comments.format.as_deref() {
        if format != "png" && format != "jpeg" {
            return None;
        }
    }
    // The header parser already refused to accumulate past the ceiling; a
    // payload that tripped it has no bytes to decode.
    if comments.encoded_too_long {
        return None;
    }
    let encoded = comments.encoded.as_deref()?;
    // Belt and braces for a caller that built `TextureComments` itself: the
    // payload is decoded before the raster decoder can bound it, so the base64
    // itself needs a ceiling. A header may carry as much text as the file
    // holds, and decoding all of it while the file buffer is still alive is the
    // one place in this reader that allocates straight from input length. Four
    // thirds of the companion-image cap is the same picture budget applied
    // before the pixels exist.
    if encoded.len() > max_encoded_chars() {
        return None;
    }
    let png = embed::decode(encoded)?;
    crate::texture_decode::decode_embedded_raster(&png, "PLY").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_vertex_fields_are_preserved_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mut bytes = format!(
                "ply\nformat {format} 1.0\nelement vertex 1\n\
                 property int x\nproperty int y\nproperty int z\n\
                 property int nx\nproperty int ny\nproperty int nz\n\
                 property int s\nproperty int t\nend_header\n"
            )
            .into_bytes();
            let values = [2i32, -3, 4, 0, 0, 1, 1, -1];
            if format == "ascii" {
                bytes.extend_from_slice(b"2 -3 4 0 0 1 1 -1\n");
            } else {
                for value in values {
                    bytes.extend_from_slice(&if format == "binary_big_endian" {
                        value.to_be_bytes()
                    } else {
                        value.to_le_bytes()
                    });
                }
            }
            let mesh = read(&bytes).expect("integer vertex fields");
            assert_eq!(mesh.vertices()[0].position, [2.0, -3.0, 4.0], "{format}");
            assert_eq!(mesh.vertices()[0].normal, [0.0, 0.0, 1.0], "{format}");
            assert_eq!(mesh.vertices()[0].uv, [1.0, -1.0], "{format}");
        }
    }

    #[test]
    fn non_integer_ascii_vertex_properties_are_rejected() {
        for value in ["1.5", "NaN", "word"] {
            let bytes = format!(
                "ply\nformat ascii 1.0\nelement vertex 1\nproperty int x\nend_header\n{value}\n"
            );
            assert!(read(bytes.as_bytes()).is_err(), "accepted integer {value}");
        }
    }

    fn vertex_list_file(format: &str) -> Vec<u8> {
        let mut bytes = format!(
            "ply\nformat {format} 1.0\nelement vertex 2\n\
             property list uchar int neighbors\nproperty float x\n\
             property list uchar float weights\nproperty float y\nproperty float z\n\
             property list uchar uchar flags\nend_header\n"
        )
        .into_bytes();
        if format == "ascii" {
            bytes.extend_from_slice(b"2 8 9 1 1 0.5 2 3 2 10 11\n0 4 0 5 6 0\n");
        } else {
            let big = format == "binary_big_endian";
            bytes.push(2);
            for n in [8i32, 9] {
                bytes.extend_from_slice(&if big {
                    n.to_be_bytes()
                } else {
                    n.to_le_bytes()
                });
            }
            let float = |n: f32| {
                if big {
                    n.to_be_bytes()
                } else {
                    n.to_le_bytes()
                }
            };
            bytes.extend_from_slice(&float(1.0));
            bytes.push(1);
            bytes.extend_from_slice(&float(0.5));
            bytes.extend_from_slice(&float(2.0));
            bytes.extend_from_slice(&float(3.0));
            bytes.extend_from_slice(&[2, 10, 11, 0]);
            bytes.extend_from_slice(&float(4.0));
            bytes.push(0);
            bytes.extend_from_slice(&float(5.0));
            bytes.extend_from_slice(&float(6.0));
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn vertex_lists_are_consumed_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mesh = read(&vertex_list_file(format)).expect("valid vertex lists");
            assert_eq!(mesh.vertices()[0].position, [1.0, 2.0, 3.0], "{format}");
            assert_eq!(mesh.vertices()[1].position, [4.0, 5.0, 6.0], "{format}");
            assert!(mesh.is_point_cloud());
        }
    }

    #[test]
    fn truncated_vertex_lists_are_rejected_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mut bytes = vertex_list_file(format);
            bytes.truncate(bytes.len() - 2);
            assert!(read(&bytes).is_err(), "accepted truncated {format} list");
        }
    }
}
