//! Thumbnail-specific format inference.

use occluview_formats::{probe, FormatError, FormatKind};

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

fn looks_like_obj_text(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    text.lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .is_some_and(is_obj_record)
}

fn is_obj_record(line: &str) -> bool {
    line == "v"
        || line.starts_with("v ")
        || line.starts_with("vn ")
        || line.starts_with("vt ")
        || line.starts_with("f ")
        || line.starts_with("o ")
        || line.starts_with("g ")
        || line.starts_with("s ")
        || line.starts_with("usemtl ")
        || line.starts_with("mtllib ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_triangle_binary_stl() -> Vec<u8> {
        let mut out = vec![0u8; 84];
        out[80..84].copy_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 50]);
        out
    }

    #[test]
    fn glb_magic_wins_without_extension() {
        assert_eq!(
            infer_thumbnail_format(None, b"glTF\x02\x00\x00\x00").unwrap(),
            FormatKind::Gltf
        );
    }

    #[test]
    fn binary_stl_magic_wins_over_wrong_extension() {
        assert_eq!(
            infer_thumbnail_format(Some("obj"), &one_triangle_binary_stl()).unwrap(),
            FormatKind::Stl
        );
    }

    #[test]
    fn obj_text_is_detected_without_extension() {
        let obj = b"# scan export\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        assert_eq!(infer_thumbnail_format(None, obj).unwrap(), FormatKind::Obj);
    }

    #[test]
    fn extension_selects_obj_when_magic_is_silent() {
        assert_eq!(
            infer_thumbnail_format(Some(".OBJ"), b"not enough obj syntax").unwrap(),
            FormatKind::Obj
        );
    }

    #[test]
    fn gltf_json_is_deferred_for_thumbnails() {
        let err = infer_thumbnail_format(Some("gltf"), br#"{"asset":{"version":"2.0"}}"#)
            .expect_err("JSON glTF is not a v1 thumbnail format");
        assert!(err.to_string().contains("gltf"));
    }

    #[test]
    fn threemf_is_deferred_for_thumbnails() {
        let err = infer_thumbnail_format(Some("3mf"), &[0x50, 0x4B, 0x03, 0x04])
            .expect_err("3MF is not a v1 thumbnail format");
        assert!(err.to_string().contains("3mf"));
    }

    #[test]
    fn unknown_input_is_rejected() {
        let err = infer_thumbnail_format(None, b"not a mesh")
            .expect_err("unknown bytes without extension should be rejected");
        assert!(err.to_string().contains("unsupported"));
    }
}
