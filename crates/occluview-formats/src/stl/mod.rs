//! STL reader.
//!
//! STL is the dental workhorse: triangle-only, no color, almost always binary.
//! Two variants share this module:
//!
//! - [`binary`] — 80-byte header + `u32` triangle count + N × (normal + 3 verts
//!   + `u16` attribute), 50 bytes per triangle.
//! - [`ascii`] — `solid … endsolid` text, whitespace-separated floats.
//!
//! Real-world quirks we tolerate:
//!
//! - The 80-byte header sometimes contains non-ASCII bytes; never assume text.
//! - The declared triangle count is occasionally wrong; detect by EOF, not by
//!   count alone.
//! - ASCII files are sometimes mislabeled as binary (no reliable magic); the
//!   probe path hints, and [`read`] re-checks.

pub mod ascii;
pub mod binary;

use crate::error::FormatError;
use occluview_core::Mesh;

/// Read an STL from raw bytes.
///
/// Dispatches to ASCII or binary by inspecting the content (the probe path
/// gives a *hint*; we confirm here, because STL has no reliable magic byte).
///
/// # Errors
/// See [`FormatError`]. Parsers never panic.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    if ascii::looks_like_ascii(bytes) {
        ascii::read(bytes)
    } else {
        binary::read(bytes)
    }
}
