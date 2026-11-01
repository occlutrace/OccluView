//! Import memory estimates shared by direct readers and scene loading.

use crate::error::FormatError;
use crate::probe::FormatKind;

/// Maximum estimated scene memory held by a scene and its active import.
pub const SCENE_IMPORT_MEMORY_BUDGET_BYTES: u64 = 2 << 30;

/// Estimate the peak bytes held while dispatching one input file.
///
/// The estimate includes the owned source buffer, parser working data, decoded
/// geometry, and the maximum decoded texture when the format can attach one.
/// Text readers share the geometry growth ceiling in `MeshBuilder`; formats
/// with additional working arrays reserve space for those arrays as well.
pub(crate) fn estimate_file_peak_bytes(
    kind: FormatKind,
    bytes: &[u8],
    reserved_bytes: u64,
) -> Result<u64, FormatError> {
    let source_bytes = byte_count(bytes.len());
    let geometry_limit = occluview_core::MeshBuilder::MAX_GEOMETRY_BYTES_PER_INPUT_BYTE;
    let parser_bytes = match kind {
        FormatKind::Stl => crate::stl::estimate_peak_bytes(bytes)?,
        FormatKind::Ply => {
            let input_growth = source_bytes.saturating_mul(geometry_limit.saturating_add(2));
            check_estimate(reserved_bytes.saturating_add(input_growth))?;
            let declared = crate::ply::estimate_declared_bytes(bytes).unwrap_or(0);
            input_growth.max(source_bytes.saturating_add(declared))
        }
        FormatKind::Obj => crate::obj::estimate_peak_bytes(bytes),
        FormatKind::Off => source_bytes.saturating_mul(geometry_limit.saturating_add(2)),
        FormatKind::Gltf => crate::gltf::estimate_peak_bytes(bytes, reserved_bytes)?,

        FormatKind::Threemf => source_bytes,
    };
    let texture_bytes = if may_decode_texture(kind, bytes) {
        crate::texture_decode::MAX_TEXTURE_RGBA_BYTES.saturating_mul(2)
    } else {
        0
    };
    Ok(parser_bytes.saturating_add(texture_bytes))
}

pub(crate) fn estimate_companion_peak_bytes(kind: FormatKind, _bytes: &[u8]) -> u64 {
    // Path-aware lookup can decode a same-stem image without a TextureFile
    // header or UVs. Attachment decides later whether the mesh can use it.
    // Reserve for every format whose companion lookup may read an image.
    if matches!(kind, FormatKind::Obj | FormatKind::Ply) {
        crate::companions::MAX_COMPANION_IMAGE_BYTES
            .saturating_add(crate::texture_decode::MAX_TEXTURE_RGBA_BYTES.saturating_mul(2))
    } else {
        0
    }
}

/// Refuse an estimate before a parser can allocate from file-controlled data.
pub(crate) fn check_estimate(estimated_bytes: u64) -> Result<(), FormatError> {
    if estimated_bytes > SCENE_IMPORT_MEMORY_BUDGET_BYTES {
        return Err(FormatError::MemoryBudgetExceeded {
            estimated_bytes,
            limit: SCENE_IMPORT_MEMORY_BUDGET_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn check_scene_estimate(estimated_bytes: u64) -> Result<(), FormatError> {
    check_estimate(estimated_bytes)
}

fn may_decode_texture(kind: FormatKind, bytes: &[u8]) -> bool {
    match kind {
        FormatKind::Stl | FormatKind::Off | FormatKind::Threemf | FormatKind::Obj => false,
        FormatKind::Ply => contains(bytes, b"OccluViewTexture") && crate::ply::may_have_uvs(bytes),
        FormatKind::Gltf => {
            let Ok((json, _)) = crate::gltf::glb::split(bytes) else {
                return false;
            };
            contains_any(&json, &[b"\"images\"", b"\"textures\""])
        }
    }
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn contains_any(bytes: &[u8], needles: &[&[u8]]) -> bool {
    needles.iter().any(|needle| contains(bytes, needle))
}

fn byte_count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_formats_reserve_compressed_and_decoded_image_memory() {
        let obj = b"v 0 0 0\nvt 0.5 0.5\n";
        let obj_peak = estimate_file_peak_bytes(FormatKind::Obj, obj, 0).expect("OBJ estimate");
        assert!(obj_peak < 1024 * 1024);
        assert!(
            estimate_companion_peak_bytes(FormatKind::Obj, obj)
                >= crate::companions::MAX_COMPANION_IMAGE_BYTES.saturating_add(
                    crate::texture_decode::MAX_TEXTURE_RGBA_BYTES.saturating_mul(2)
                ),
            "path-aware companion decoding must be inside the import estimate"
        );
        let untextured_obj = estimate_file_peak_bytes(FormatKind::Obj, b"v 0 0 0\n", 0)
            .expect("untextured OBJ estimate");
        assert!(untextured_obj < 1024 * 1024);
        assert!(
            estimate_companion_peak_bytes(FormatKind::Obj, b"v 0 0 0\n")
                >= crate::companions::MAX_COMPANION_IMAGE_BYTES.saturating_add(
                    crate::texture_decode::MAX_TEXTURE_RGBA_BYTES.saturating_mul(2)
                )
        );
    }

    #[test]
    fn same_stem_ply_companions_do_not_require_a_texture_file_header() {
        let ply = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty float s\nproperty float t\nend_header\n0 0 0 0 0\n";
        assert!(!contains(ply, b"TextureFile"));
        assert!(crate::ply::may_have_uvs(ply));
        let reserve = crate::companions::MAX_COMPANION_IMAGE_BYTES
            .saturating_add(crate::texture_decode::MAX_TEXTURE_RGBA_BYTES.saturating_mul(2));
        assert_eq!(estimate_companion_peak_bytes(FormatKind::Ply, ply), reserve);
        assert_eq!(
            estimate_companion_peak_bytes(FormatKind::Ply, b"ply\n"),
            reserve
        );
        assert_eq!(estimate_companion_peak_bytes(FormatKind::Stl, ply), 0);
    }
}
