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
    /// largest index in the file let a 232-byte file ask for 51 GB, and the
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
