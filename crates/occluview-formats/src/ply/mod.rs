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
pub mod header;

use crate::error::FormatError;
use occluview_core::Mesh;

/// Read a PLY from raw bytes.
///
/// Dispatches to ASCII or binary (LE/BE) based on the header's `format` line.
///
/// # Errors
/// See [`FormatError`]. Parsers never panic.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let parsed = header::parse(bytes)?;
    match parsed.format {
        header::Format::Ascii => ascii::read(&parsed),
        header::Format::BinaryLittleEndian => binary::read_le(&parsed),
        header::Format::BinaryBigEndian => binary::read_be(&parsed),
    }
}
