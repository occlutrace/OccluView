//! Thumbnail-specific format inference.

use occluview_formats::{probe, FormatError, FormatKind, LEGACY_HPS_EXTENSION};

/// Infer the format a thumbnail render should use.
///
/// Explorer commonly initializes thumbnail providers through
/// `IInitializeWithStream`, which carries bytes but not a file path. The shared
/// formats probe handles magic-byte formats, while this shell layer adds the
/// conservative text probes and v1 thumbnail policy that are specific to shell
/// rendering.
///
/// # Errors
/// Returns [`FormatError::Unsupported`] for unknown or deferred thumbnail
/// formats, and propagates probe errors from `occluview-formats`.
pub fn infer_thumbnail_format(
    extension: Option<&str>,
    bytes: &[u8],
) -> Result<FormatKind, FormatError> {
    let extension = extension
        .map(normalize_extension)
        .filter(|ext| !ext.is_empty());

    if bytes.starts_with(b"glTF") {
        return Ok(FormatKind::Gltf);
    }
    if is_zip_magic(bytes) {
        match extension.as_deref() {
            Some("3mf") => return deferred("3mf"),
            Some(extension) if extension == LEGACY_HPS_EXTENSION || extension == "hps" => {
                return Ok(FormatKind::Hps);
            }
            None => return Ok(FormatKind::Hps),
            _ => {}
        }
        return deferred("3mf");
    }
    if looks_like_obj_text(bytes) {
        return Ok(FormatKind::Obj);
    }

    if matches!(extension.as_deref(), Some("3mf")) {
        return deferred("3mf");
    }
    if matches!(extension.as_deref(), Some("gltf")) {
        return deferred("gltf");
    }
    match probe(extension.as_deref(), bytes)? {
        FormatKind::Threemf => deferred("3mf"),
        FormatKind::Gltf if !bytes.starts_with(b"glTF") => deferred("gltf"),
        kind => Ok(kind),
    }
}

fn normalize_extension(extension: &str) -> String {
    extension.trim_start_matches('.').to_ascii_lowercase()
}

fn deferred(extension: &str) -> Result<FormatKind, FormatError> {
    Err(FormatError::Unsupported {
        extension: extension.to_string(),
    })
}

fn is_zip_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == [0x50, 0x4B, 0x03, 0x04]
}

/// Whether the first meaningful line reads as an OBJ record.
///
/// Only the lines it has to look at are decoded. Decoding the whole input
/// first cost 796 ms and about 165 MB of transient allocation on a 94 MB STL,
/// which is most of that file's thumbnail: this runs on Explorer's own thread,
/// before a render lane is taken, and outside the six-second deadline. It
/// fires for every STL over 40 MiB, every PLY over 4 MiB -- which is nearly
/// every real dental PLY -- and on every stream request.
fn looks_like_obj_text(bytes: &[u8]) -> bool {
    // A record line in a real OBJ is short; anything longer is not one.
    const MAX_RECORD_BYTES: usize = 4096;
    // And the whole probe reads at most this much. Splitting on newlines over
    // the full input is still linear when the input has none -- which is what
    // a binary STL of a flat surface looks like -- so the window is what makes
    // the cost independent of file size. A text OBJ's first record is in the
    // first few hundred bytes; a file whose first 64 KiB hold no line at all
    // is not one.
    const PROBE_WINDOW_BYTES: usize = 64 * 1024;

    let window = &bytes[..bytes.len().min(PROBE_WINDOW_BYTES)];
    for line in window.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = &line[..line.len().min(MAX_RECORD_BYTES)];
        let text = String::from_utf8_lossy(line);
        let text = text.trim_start().trim_start_matches('\u{feff}');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        return is_obj_record(text);
    }
    false
}

fn is_obj_record(line: &str) -> bool {
    matches!(
        line.split_ascii_whitespace().next(),
        Some(
            "v" | "vn"
                | "vt"
                | "f"
                | "o"
                | "g"
                | "s"
                | "usemtl"
                | "mtllib"
                | "newmtl"
                | "vp"
                | "bevel"
                | "cstype"
                | "deg"
                | "curv"
                | "curv2"
                | "surf"
                | "parm"
                | "trim"
                | "hole"
                | "scrv"
                | "sp"
                | "end"
                | "con"
                | "bmat"
                | "step"
        )
    )
}

#[cfg(test)]
mod tests {
    /// The OBJ probe must look at the first line, not at the file.
    ///
    /// Decoding the whole input cost 660 ms on a 100 MB binary STL, measured
    /// on the machine this was written on, against 12 us for the line-wise
    /// form. This runs on Explorer's thread before a render lane is taken.
    #[test]
    fn the_obj_probe_does_not_read_the_whole_file() {
        // 32 MB of binary STL: the old form takes about 210 ms on it.
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&600_000_u32.to_le_bytes());
        bytes.resize(32 * 1024 * 1024, 0x7f);

        let started = std::time::Instant::now();
        let answer = looks_like_obj_text(&bytes);
        let elapsed = started.elapsed();

        assert!(!answer, "binary STL is not an OBJ");
        assert!(
            elapsed < std::time::Duration::from_millis(40),
            "the probe took {elapsed:?} on 32 MB; it is reading past the first \
             line again"
        );
    }

    /// And it still answers correctly for the format it exists to spot.
    #[test]
    fn a_text_obj_is_still_recognised_after_comments_and_a_bom() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice("\u{feff}".as_bytes());
        bytes.extend_from_slice(b"# exported by a scanner\r\n\r\n");
        bytes.extend_from_slice(b"v 1.0 2.0 3.0\n");
        assert!(looks_like_obj_text(&bytes));

        assert!(!looks_like_obj_text(
            b"ply\nformat binary_little_endian 1.0\n"
        ));
        assert!(!looks_like_obj_text(b""));
        assert!(!looks_like_obj_text(b"# only a comment\n"));
    }

    use super::*;

    fn one_triangle_binary_stl() -> Vec<u8> {
        let mut out = vec![0u8; 84];
        out[80..84].copy_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 50]);
        out
    }

    #[test]
    fn glb_magic_wins_without_extension() {
        assert!(matches!(
            infer_thumbnail_format(None, b"glTF\x02\x00\x00\x00"),
            Ok(FormatKind::Gltf)
        ));
    }

    #[test]
    fn binary_stl_magic_wins_over_wrong_extension() {
        assert!(matches!(
            infer_thumbnail_format(Some("obj"), &one_triangle_binary_stl()),
            Ok(FormatKind::Stl)
        ));
    }

    #[test]
    fn obj_text_is_detected_without_extension() {
        let obj = b"# scan export\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        assert!(matches!(
            infer_thumbnail_format(None, obj),
            Ok(FormatKind::Obj)
        ));
    }

    #[test]
    fn obj_text_with_bom_and_non_utf8_metadata_is_detected_without_extension() {
        let obj = b"\xef\xbb\xbf# scanner metadata \xFF\xFE\nmtllib scan.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

        assert!(matches!(
            infer_thumbnail_format(None, obj),
            Ok(FormatKind::Obj)
        ));
    }

    #[test]
    fn obj_text_with_material_prologue_is_detected_without_extension() {
        let obj = b"# scanner export\nnewmtl\tenamel\nvp 0 0 1\ncurv 0 1 1 2\nv\t0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

        assert!(matches!(
            infer_thumbnail_format(None, obj),
            Ok(FormatKind::Obj)
        ));
    }

    #[test]
    fn extension_selects_obj_when_magic_is_silent() {
        assert!(matches!(
            infer_thumbnail_format(Some(".OBJ"), b"not enough obj syntax"),
            Ok(FormatKind::Obj)
        ));
    }

    #[test]
    fn gltf_json_is_deferred_for_thumbnails() {
        assert!(matches!(
            infer_thumbnail_format(Some("gltf"), br#"{"asset":{"version":"2.0"}}"#),
            Err(FormatError::Unsupported { extension }) if extension == "gltf"
        ));
    }

    #[test]
    fn threemf_is_deferred_for_thumbnails() {
        assert!(matches!(
            infer_thumbnail_format(Some("3mf"), &[0x50, 0x4B, 0x03, 0x04]),
            Err(FormatError::Unsupported { extension }) if extension == "3mf"
        ));
    }

    #[test]
    fn hps_reaches_parser_for_thumbnails() {
        assert!(matches!(
            infer_thumbnail_format(Some("dcm"), &[0x50, 0x4B, 0x03, 0x04]),
            Ok(FormatKind::Hps)
        ));
        assert!(matches!(
            infer_thumbnail_format(None, &[0x50, 0x4B, 0x03, 0x04]),
            Ok(FormatKind::Hps)
        ));
        assert!(matches!(
            infer_thumbnail_format(Some("hps"), br"<HPS><Schema>CC</Schema></HPS>"),
            Ok(FormatKind::Hps)
        ));
        assert!(matches!(
            infer_thumbnail_format(None, br"<HPS><Schema>CC</Schema></HPS>"),
            Ok(FormatKind::Hps)
        ));
    }

    #[test]
    fn unknown_input_is_rejected() {
        assert!(matches!(
            infer_thumbnail_format(None, b"not a mesh"),
            Err(FormatError::Unsupported { .. })
        ));
    }
}
