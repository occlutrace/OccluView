//! Format detection by extension + magic bytes.

use crate::error::FormatError;

/// The kind of format detected for a file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FormatKind {
    /// Binary or ASCII STL.
    Stl,
    /// PLY (binary or ASCII).
    Ply,
    /// Wavefront OBJ (+ companion `.mtl`).
    Obj,
    /// glTF 2.0 — JSON `.gltf` or binary `.glb`.
    Gltf,
    /// 3MF (XML-in-ZIP).
    Threemf,
    /// Object File Format (Princeton). Binary or ASCII.
    Off,
}

/// Map a file extension (lowercase, no dot) to a [`FormatKind`].
#[must_use]
pub fn by_extension(ext: &str) -> Option<FormatKind> {
    match ext {
        "stl" => Some(FormatKind::Stl),
        "ply" => Some(FormatKind::Ply),
        "obj" => Some(FormatKind::Obj),
        "gltf" | "glb" => Some(FormatKind::Gltf),
        "3mf" => Some(FormatKind::Threemf),
        "off" => Some(FormatKind::Off),
        _ => None,
    }
}

/// Probe both the extension and the leading magic bytes and return the most
/// likely [`FormatKind`].
///
/// Magic bytes take precedence over a filename extension. The extension is
/// the fallback for formats without an unambiguous signature.
///
/// # Errors
/// - [`FormatError::Unsupported`] if neither extension nor magic match.
pub fn probe(extension: Option<&str>, magic: &[u8]) -> Result<FormatKind, FormatError> {
    // The BOM is judged per signature, never by stripping the buffer up front.
    //
    // A binary STL's 80-byte header is free-form by contract and may itself
    // begin with those three bytes; on that file the size formula reads the
    // count at offset 80, and testing it against a three-byte-short slice would
    // break the match and report an unnamed stream as Unsupported. Text
    // signatures (`ply`, `solid`, `{`, `OFF`) are where a leading
    // BOM actually hides the prefix, so only they are tested with it removed.
    let text_magic = magic.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(magic);

    // Binary STL: raw bytes, before any text consideration.
    if crate::stl::binary_layout_matches(magic) {
        return Ok(FormatKind::Stl);
    }
    let magic = text_magic;

    // Magic-byte first, since scanners sometimes mislabel files.
    if magic.len() >= 4 {
        if &magic[..4] == b"glTF" {
            return Ok(FormatKind::Gltf);
        }
        // PK\x03\x04 = ZIP — used by 3MF.
        if magic[..4] == [0x50, 0x4B, 0x03, 0x04] {
            return Ok(FormatKind::Threemf);
        }
    }
    if magic.starts_with(b"ply\n")
        || magic.starts_with(b"ply\r\n")
        || magic.starts_with(b"ply\t")
        || magic.starts_with(b"ply ")
    {
        return Ok(FormatKind::Ply);
    }
    if magic.starts_with(b"OFF") {
        // "OFF", "OFF BINARY", "OFFST", "OFF\n" - all variants.
        return Ok(FormatKind::Off);
    }
    if magic.starts_with(b"solid") {
        // "solid" is the ASCII STL header — but binary STL can also start with
        // arbitrary 80-byte headers that occasionally begin with "solid".
        // Disambiguation happens in the STL reader; here we hint STL.
        return Ok(FormatKind::Stl);
    }
    // (The binary size formula was already evaluated above, on the raw bytes —
    // see why the BOM is decided per signature.)
    // glTF .gltf (JSON) is a JSON object at the top level: skip leading
    // whitespace and look for the opening brace.
    if magic.get(skip_ascii_ws(magic, 0)) == Some(&b'{') {
        return Ok(FormatKind::Gltf);
    }

    // Fall back to the extension.
    if let Some(ext) = extension {
        if let Some(kind) = by_extension(ext) {
            return Ok(kind);
        }
    }

    Err(FormatError::Unsupported {
        extension: extension.unwrap_or("").to_string(),
    })
}

#[must_use]
fn skip_ascii_ws(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_map_covers_v1_formats() {
        assert_eq!(by_extension("stl"), Some(FormatKind::Stl));
        assert_eq!(by_extension("STL"), None); // caller lowercases; document behavior
        assert_eq!(by_extension("ply"), Some(FormatKind::Ply));
        assert_eq!(by_extension("obj"), Some(FormatKind::Obj));
        assert_eq!(by_extension("gltf"), Some(FormatKind::Gltf));
        assert_eq!(by_extension("glb"), Some(FormatKind::Gltf));
        assert_eq!(by_extension("3mf"), Some(FormatKind::Threemf));
        assert_eq!(by_extension("foo"), None);
    }

    #[test]
    fn probe_glb_magic_wins_without_extension() {
        let magic = b"glTF\x02\x00\x00\x00";
        assert_eq!(probe(None, magic).unwrap(), FormatKind::Gltf);
    }

    #[test]
    fn probe_ply_magic() {
        assert_eq!(
            probe(None, b"ply\nformat ascii 1.0\n").unwrap(),
            FormatKind::Ply
        );
    }

    #[test]
    fn probe_3mf_zip_magic() {
        let magic = [0x50, 0x4B, 0x03, 0x04, 0x00, 0x00];
        assert_eq!(probe(None, &magic).unwrap(), FormatKind::Threemf);
    }

    #[test]
    fn probe_falls_back_to_extension_when_magic_silent() {
        // Binary STL with an empty (zero) header — no recognizable magic.
        let magic = [0u8; 84];
        assert_eq!(probe(Some("stl"), &magic).unwrap(), FormatKind::Stl);
    }

    #[test]
    fn probe_returns_unsupported_for_unknown() {
        assert!(probe(Some("xyz"), &[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn probe_detects_binary_stl_with_arbitrary_header() {
        // Real OccluTrace export: binary STL with header "OccluTrace Native
        // binary STL", declared as one triangle. Even without the "solid"
        // prefix, the size formula (84 + 50*1 == 134) identifies it.
        let mut header = [0u8; 80];
        let label = b"OccluTrace Native binary STL";
        header[..label.len()].copy_from_slice(label);
        // 1 triangle record (50 bytes).
        let bytes = occluview_core::test_support::binary_stl_with_header(&header, &[[0.0; 12]]);
        assert_eq!(
            probe(Some("ply"), &bytes).unwrap(),
            FormatKind::Stl,
            "magic-first must beat the .ply extension"
        );
    }

    #[test]
    fn binary_layout_matches_size_formula() {
        // Exact-size match -> yes.
        let mut bytes = occluview_core::test_support::binary_stl(&[[0.0; 12]; 3]);
        assert!(crate::stl::binary_layout_matches(&bytes));

        // Off-by-one size -> no.
        bytes.pop();
        assert!(!crate::stl::binary_layout_matches(&bytes));

        // Too short -> no.
        assert!(!crate::stl::binary_layout_matches(&[0u8; 10]));

        // Absurd count (would imply >200M triangles) -> no.
        let mut bad = vec![0u8; 84];
        bad[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!crate::stl::binary_layout_matches(&bad));
    }
}
