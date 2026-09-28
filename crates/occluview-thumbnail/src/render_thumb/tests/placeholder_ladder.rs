//! The thumbnail failure -> placeholder decision ladder.
//!
//! Covers which placeholder flavor each failure class produces, that corrupt
//! files get the "!" badge while over-budget / no-key / unsupported stay a
//! quiet plain cube, that point-cloud PLYs choose the splat path, and that a
//! healthy mesh still renders a real thumbnail (not a placeholder).

use super::*;
use crate::placeholder::{placeholder_thumbnail_kind, PlaceholderKind};
use occluview_core::CoreError;
use occluview_formats::FormatError;
use occluview_render::RenderError;

fn spec_256() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    }
}

#[test]
fn broken_recognized_formats_get_the_corrupt_badge() {
    let corrupt = [
        ThumbnailError::Format(FormatError::Truncated {
            format: "STL (binary)",
            expected: 134,
            got: 12,
        }),
        ThumbnailError::Format(FormatError::BadSignature {
            format: "PLY",
            offset: 0,
        }),
        ThumbnailError::Format(FormatError::Malformed {
            format: "OBJ",
            offset: 4,
            reason: "bad face".to_string(),
        }),
        ThumbnailError::Format(FormatError::Core(CoreError::IndexCountNotMultipleOfThree {
            index_count: 7,
        })),
    ];
    for error in &corrupt {
        assert_eq!(
            placeholder_kind_for_error(error),
            PlaceholderKind::Corrupt,
            "{error:?} should map to the corrupt badge"
        );
    }
}

#[test]
fn deferred_unsupported_and_infra_failures_stay_plain() {
    let plain = [
        // No-key / encrypted HPS surfaces as Deferred.
        ThumbnailError::Format(FormatError::Deferred {
            format: "HPS CE",
            reason: "encrypted CE schema needs a configured key provider".to_string(),
        }),
        ThumbnailError::Format(FormatError::Unsupported {
            extension: "xyz".to_string(),
        }),
        ThumbnailError::Format(FormatError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        ))),
        // Over-budget sentinel: a synthetic Malformed tagged "thumbnail …".
        ThumbnailError::Format(FormatError::Malformed {
            format: "thumbnail file",
            offset: 0,
            reason: "too big".to_string(),
        }),
        ThumbnailError::Format(FormatError::MemoryBudgetExceeded {
            estimated_bytes: 3_u64 << 30,
            limit: 2_u64 << 30,
        }),
        // GPU / renderer / win32 failures are not the file's fault.
        ThumbnailError::Render(RenderError::NoAdapter),
        ThumbnailError::Render(RenderError::Surface("lost the offscreen lease".to_string())),
    ];
    for error in &plain {
        assert_eq!(
            placeholder_kind_for_error(error),
            PlaceholderKind::Plain,
            "{error:?} should map to the plain cube"
        );
    }
}

#[test]
fn corrupt_file_render_returns_the_badged_placeholder() {
    // A .stl that claims millions of triangles but is truncated garbage.
    let mut bytes = vec![0u8; 84];
    bytes[..7].copy_from_slice(b"corrupt");
    bytes[80..84].copy_from_slice(&5_000_000u32.to_le_bytes());
    bytes.extend_from_slice(b"not-triangles");
    let path = fixtures::write_temp_fixture("stl", &bytes);

    let spec = spec_256();
    let pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_eq!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt),
        "a corrupt recognized file should yield the badged placeholder cube"
    );
    assert_ne!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Plain),
        "the corrupt placeholder must be visually distinct from the plain one"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn unsupported_extension_returns_the_plain_placeholder() {
    let path = fixtures::write_temp_fixture("xyz", b"not a mesh at all");
    let spec = spec_256();
    let pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_eq!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Plain),
        "an unsupported extension should yield the quiet plain placeholder cube"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn over_budget_input_returns_the_plain_placeholder() {
    let spec = spec_256();
    let pixels = placeholder_for_oversize_input(spec, 900 * 1024 * 1024);
    assert_eq!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Plain),
        "over-budget files should get the plain placeholder, not the corrupt badge"
    );
}

#[test]
fn point_cloud_ply_loads_as_point_cloud_for_the_splat_path() {
    // A faceless PLY (vertex element only) must load as a point cloud so the
    // renderer draws splats instead of an empty triangle mesh.
    let ply = b"ply\n\
format ascii 1.0\n\
element vertex 4\n\
property float x\n\
property float y\n\
property float z\n\
end_header\n\
0 0 0\n\
1 0 0\n\
0 1 0\n\
1 1 0\n";
    let path = fixtures::write_temp_fixture("ply", ply);
    let metadata = cache::thumbnail_file_metadata(&path).expect("temp PLY metadata");
    let mesh = load_thumbnail_mesh_from_file(&path, metadata).expect("faceless PLY thumbnail mesh");
    assert!(
        mesh.is_point_cloud(),
        "a faceless PLY must load as a point cloud so the splat pipeline runs"
    );
    assert!(mesh.indices().is_empty(), "a point cloud has no triangles");
    assert!(!mesh.vertices().is_empty(), "point cloud kept its points");
    let _ = fs::remove_file(path);
}

#[test]
fn healthy_mesh_renders_a_real_thumbnail_not_a_placeholder() {
    let path = fixtures::write_temp_fixture("stl", &fixtures::binary_stl_cube());
    let spec = spec_256();
    let pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_ne!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Plain),
        "a healthy mesh must render a real thumbnail, not the plain placeholder"
    );
    assert_ne!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt),
        "a healthy mesh must render a real thumbnail, not the corrupt placeholder"
    );
    assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
    let _ = fs::remove_file(path);
}
