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
}

/// Map a file extension (lowercase, no dot) to a [`FormatKind`].
///
/// # Errors
/// Returns [`FormatError::Unsupported`] if the extension is unknown.
#[must_use]
pub fn by_extension(ext: &str) -> Option<FormatKind> {
    match ext {
        "stl" => Some(FormatKind::Stl),
        "ply" => Some(FormatKind::Ply),
        "obj" => Some(FormatKind::Obj),
        "gltf" | "glb" => Some(FormatKind::Gltf),
        "3mf" => Some(FormatKind::Threemf),
        _ => None,
    }
}

/// Probe both the extension and the leading magic bytes and return the most
/// likely [`FormatKind`]. Magic bytes win when extension and magic disagree
/// (some scanners mislabel files).
///
/// # Errors
/// - [`FormatError::Unsupported`] if neither extension nor magic match.
pub fn probe(extension: Option<&str>, magic: &[u8]) -> Result<FormatKind, FormatError> {
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
    if magic.starts_with(b"solid") {
        // "solid" is the ASCII STL header — but binary STL can also start with
        // arbitrary 80-byte headers that occasionally begin with "solid".
        // Disambiguation happens in the STL reader; here we hint STL.
        return Ok(FormatKind::Stl);
    }
    // Binary STL with an arbitrary (non-"solid") 80-byte header: many scanners
    // and CAD tools (including OccluTrace exports) write a free-form ASCII label
    // in the header. Detect via the size formula: file_len == 84 + 50 * count.
    // This is the standard three.js STLLoader heuristic and is very reliable in
    // practice (a PLY/OBJ/glTF accidentally matching is astronomically unlikely).
    if looks_like_binary_stl(magic) {
        return Ok(FormatKind::Stl);
    }
    // glTF .gltf (JSON) — probe for leading `{` or whitespace then `"asset"`.
    if magic.iter().take_while(|b| b.is_ascii_whitespace()).count() < magic.len()
        && magic.first() == Some(&b'{')
    {
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

/// Heuristic: does `bytes` look like a binary STL with an arbitrary header?
///
/// Binary STL is `84 + 50 * triangle_count` bytes long, where `triangle_count`
/// is a little-endian `u32` at offset 80. If that identity holds exactly and
/// the count is plausible, this is almost certainly a binary STL (a PLY/OBJ/
/// glTF accidentally matching the size formula is astronomically unlikely).
/// This is the same heuristic three.js's `STLLoader` uses to disambiguate.
#[must_use]
fn looks_like_binary_stl(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count_bytes: [u8; 4] = match bytes[80..84].try_into() {
        Ok(arr) => arr,
        Err(_) => return false, // unreachable given the length check above
    };
    let triangle_count = u32::from_le_bytes(count_bytes) as usize;
    // Reject implausible counts: real dental scans are 0..~10M triangles; a
    // garbage u32 from a non-STL file's bytes 80..84 would either overflow the
    // size formula or be absurdly large.
    if triangle_count > 200_000_000 {
        return false;
    }
    let expected_len = 84 + triangle_count * 50;
    bytes.len() == expected_len
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
        let mut bytes = vec![0u8; 84];
        let label = b"OccluTrace Native binary STL";
        bytes[..label.len()].copy_from_slice(label);
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        // 1 triangle record (50 bytes).
        bytes.extend_from_slice(&[0u8; 50]);
        assert_eq!(
            probe(Some("ply"), &bytes).unwrap(),
            FormatKind::Stl,
            "magic-first must beat the .ply extension"
        );
    }

    #[test]
    fn looks_like_binary_stl_size_formula() {
        // Exact-size match -> yes.
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend(std::iter::repeat(0u8).take(3 * 50));
        assert!(looks_like_binary_stl(&bytes));

        // Off-by-one size -> no.
        bytes.pop();
        assert!(!looks_like_binary_stl(&bytes));

        // Too short -> no.
        assert!(!looks_like_binary_stl(&[0u8; 10]));

        // Absurd count (would imply >200M triangles) -> no.
        let mut bad = vec![0u8; 84];
        bad[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!looks_like_binary_stl(&bad));
    }
}
