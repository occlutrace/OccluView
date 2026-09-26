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
    read_shaded(bytes, crate::MeshShading::Reconstructed)
}

/// As [`read`], choosing how vertex normals are produced.
///
/// # Errors
/// See [`read`].
pub fn read_shaded(bytes: &[u8], shading: crate::MeshShading) -> Result<Mesh, FormatError> {
    // Binary first, judged on the raw bytes by the exact size formula
    // (`len == 84 + 50 * count`). The 80-byte header of a binary STL is
    // free-form by contract, so it may itself begin with the three BOM bytes —
    // and stripping them unconditionally would move the triangle count from
    // offset 80 to 83, turning a valid file into an empty mesh or a Truncated
    // error. The formula is what distinguishes the two, not the text prefix:
    // an ASCII file essentially never satisfies it.
    if binary_layout_matches(bytes) {
        return binary::read_shaded(bytes, shading);
    }
    // A UTF-8 BOM in front of `solid` is metadata a Windows tool added. Without
    // this the ASCII reader sees no `solid` and the bytes fall through to the
    // binary reader, which reports a malformed file for a perfectly good one.
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    // The BOM may also sit in front of a binary file. The raw check above cannot
    // see that one — it read the count from offset 80 of the BOM-shifted buffer —
    // so the formula is asked again of the stripped bytes. Without this second
    // question a BOM-prefixed binary STL would fall through to the raw binary
    // reader and be reported Truncated, because the count would still be read
    // three bytes late. Raw first, then stripped: a header that merely begins
    // with those bytes still wins on its own layout and is never shifted.
    if stripped.len() != bytes.len() && binary_layout_matches(stripped) {
        return binary::read_shaded(stripped, shading);
    }
    if ascii::looks_like_ascii(stripped) {
        ascii::read_shaded(stripped, shading)
    } else {
        // Neither the formula nor the text prefix decided it. Hand it to the
        // binary reader on the raw bytes so its own truncation reporting is the
        // one the operator sees, and so a binary file whose header merely fails
        // the formula is still read from offset 80.
        binary::read_shaded(bytes, shading)
    }
}

/// Whether `bytes` has exactly the size a binary STL with its declared count
/// must have.
///
/// This is the standard three.js `STLLoader` heuristic and it is what the probe
/// already trusts. It is checked here on the raw bytes so the binary decision
/// never depends on stripping anything.
fn binary_layout_matches(bytes: &[u8]) -> bool {
    const HEADER: usize = 80;
    const COUNT: usize = 4;
    const TRIANGLE: usize = 50;
    if bytes.len() < HEADER + COUNT {
        return false;
    }
    let Ok(raw) = bytes[HEADER..HEADER + COUNT].try_into() else {
        return false;
    };
    let count = u32::from_le_bytes(raw) as usize;
    bytes.len() == HEADER + COUNT + count * TRIANGLE
}
