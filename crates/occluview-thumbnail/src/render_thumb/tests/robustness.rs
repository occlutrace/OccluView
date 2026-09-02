//! Robustness checks for outlier, non-finite, and degenerate meshes.

use super::*;
use crate::fast_thumb::try_read_fast_thumbnail_mesh_for_kind;
use crate::placeholder::{placeholder_thumbnail_kind, PlaceholderKind};
use occluview_formats::FormatKind;

fn spec_256() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    }
}

#[test]
fn stl_far_outlier_above_gate_thumbnails_solid_through_public_entry_point() {
    let spec = spec_256();
    let bytes = fixtures::dense_binary_stl_sphere_with_far_outlier(44 * 1024 * 1024);
    let pixels = render_thumbnail_or_placeholder(Some("stl"), &bytes, spec);

    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_visible_thumbnail_pixels(&pixels, spec);
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(
        holes, 0,
        "a far outlier left {holes} see-through holes in an above-gate dense sphere"
    );
}

#[test]
fn stl_far_outlier_fast_surrogate_stays_a_solid_reduced_surface() {
    let bytes = fixtures::dense_binary_stl_sphere_with_far_outlier(4 * 1024 * 1024);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("outlier STL should cluster into a surface, not collapse");
    assert!(!mesh.is_point_cloud());
    assert!(
        mesh.triangle_count() > 0,
        "the sphere collapsed to no triangles"
    );
    let bbox = mesh.bbox();
    assert!(bbox.size().is_finite(), "outlier poisoned the mesh bbox");
    assert!(
        bbox.max.max_element() < 1.0e3 && bbox.min.min_element() > -1.0e3,
        "outlier vertex was left in the mesh at full scale: bbox {:?}..{:?}",
        bbox.min,
        bbox.max
    );
}

#[test]
fn stl_nonfinite_corners_thumbnail_stays_visible_and_solid() {
    let spec = spec_256();
    let bytes = fixtures::dense_binary_stl_sphere_with_nonfinite(4 * 1024 * 1024);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("a 99.8%-valid sphere must still cluster");
    assert!(
        mesh.bbox_uncached().size().is_finite(),
        "non-finite poisoned bbox"
    );

    let pixels = rendering::render_mesh_thumbnail(
        mesh,
        spec,
        RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT),
    )
    .expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(holes, 0, "non-finite handling left {holes} interior holes");
}

#[test]
fn stl_huge_coordinate_range_thumbnail_stays_visible() {
    let spec = spec_256();
    let bytes = fixtures::dense_binary_stl_huge_coordinate_range(4 * 1024 * 1024);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("huge-range STL should cluster its mm-scale bulk");
    assert!(mesh.bbox_uncached().size().is_finite());
    let pixels = rendering::render_mesh_thumbnail(
        mesh,
        spec,
        RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT),
    )
    .expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
}

#[test]
fn obj_far_outlier_thumbnails_solid() {
    let spec = spec_256();
    let bytes = fixtures::obj_grid_surface_with_far_outlier(150);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Obj, &bytes)
        .expect("outlier OBJ should cluster into a surface");
    assert!(!mesh.is_point_cloud());
    assert!(mesh.triangle_count() > 0);
    assert!(mesh.bbox_uncached().size().is_finite());

    let pixels = rendering::render_mesh_thumbnail(
        mesh,
        spec,
        RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT),
    )
    .expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(holes, 0, "OBJ outlier left {holes} interior holes");
}

#[test]
fn all_degenerate_stl_never_returns_a_transparent_tile() {
    let spec = spec_256();
    let bytes = fixtures::all_degenerate_binary_stl();

    assert!(
        try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes).is_none(),
        "the fast path must decline an all-degenerate STL"
    );
    assert!(
        load_thumbnail_mesh_from_bytes_kind(FormatKind::Stl, &bytes).is_err(),
        "an all-degenerate STL must not load as a renderable mesh"
    );

    let pixels = render_thumbnail_or_placeholder(Some("stl"), &bytes, spec);
    let visible = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] > 0)
        .count();
    assert!(
        visible > 0,
        "the public entry point returned a fully transparent tile for a degenerate file"
    );
    assert_eq!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt),
        "a wholly degenerate recognized file should get the corrupt placeholder"
    );
}

/// A named pipe with a supported extension is rejected without blocking.
#[cfg(unix)]
#[test]
fn a_named_pipe_is_refused_rather_than_opened() {
    use std::os::unix::fs::FileTypeExt;

    let dir = std::env::temp_dir().join(format!(
        "occluview-fifo-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("pipe.stl");
    let _ = fs::remove_file(&path);
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    if !made.is_ok_and(|status| status.success()) {
        return;
    }
    assert!(fs::metadata(&path)
        .expect("the pipe exists")
        .file_type()
        .is_fifo());

    let spec = ThumbnailSpec {
        size_px: 32,
        ..Default::default()
    };
    let started = Instant::now();
    let attempt = try_render_thumbnail_file(&path, spec, Duration::from_secs(6));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the pipe was opened instead of refused: {:?}",
        started.elapsed()
    );
    assert!(
        matches!(attempt, ThumbnailAttempt::Bitmap(_)),
        "a pipe is not a scan and never will be, so the verdict is cacheable"
    );
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir(&dir);
}

/// A file that changes during loading is treated as transient.
#[test]
fn a_file_that_changed_since_it_was_measured_is_transient_not_corrupt() {
    let mut truncated = fixtures::binary_stl_cube();
    truncated.truncate(truncated.len() - 20);
    let path = fixtures::write_temp_fixture("stl", &truncated);
    let spec = ThumbnailSpec {
        size_px: 32,
        ..Default::default()
    };
    let measured = cache::thumbnail_file_metadata(&path).expect("fixture metadata");
    let keys = || FileThumbnailCacheKeys {
        path: ThumbnailFileCacheKey::new(&path, &measured),
        content: None,
    };

    let settled = render_file_thumbnail_job(
        path.clone(),
        measured,
        keys(),
        spec,
        ThumbnailRenderRequest::new(Duration::from_secs(6)),
    );
    assert!(
        matches!(settled, ThumbnailAttempt::Bitmap(_)),
        "a file that is simply short is a verdict about the file"
    );

    let stale = ThumbnailFileMetadata {
        byte_len: measured.byte_len + 1024,
        modified_nanos: measured.modified_nanos,
    };
    let moving = render_file_thumbnail_job(
        path.clone(),
        stale,
        keys(),
        spec,
        ThumbnailRenderRequest::new(Duration::from_secs(6)),
    );
    assert!(
        matches!(moving, ThumbnailAttempt::TransientFailure),
        "a file that changed while it was read must be asked about again"
    );
    let _ = fs::remove_file(path);
}
