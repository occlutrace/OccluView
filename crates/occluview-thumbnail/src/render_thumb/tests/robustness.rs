//! Outlier / non-finite / degenerate robustness of the fast thumbnail path.
//!
//! Regression cover for two clusterer defects and the loader safety net:
//! - a lone far outlier vertex must not stretch the grid and collapse a dense
//!   surface into a transparent tile (robust grid bounds);
//! - a triangle with any non-finite (NaN/Inf) corner must be dropped whole, not
//!   poison the mesh bbox and blank the tile;
//! - a wholly non-drawable file must yield a placeholder, never a transparent
//!   tile, and never panic.

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
    // 44 MB dense sphere + one outlier triangle at 1e6 mm: above the STL fidelity
    // gate, so it routes through the fast (grid-clustered) surrogate. Robust grid
    // bounds must trim the outlier so the sphere still renders as a SOLID disc
    // (substantial visible coverage, zero interior see-through holes) rather than
    // the fully transparent tile the raw-bbox grid produced.
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
    // Same defect at the mesh level (fast path runs regardless of file size):
    // the clustered surrogate must keep real triangles and a bounded, finite
    // bbox (no 1e6 mm vertex parked in the mesh).
    let bytes = fixtures::dense_binary_stl_sphere_with_far_outlier(4 * 1024 * 1024);
    let mut mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("outlier STL should cluster into a surface, not collapse");
    assert!(!mesh.is_point_cloud());
    assert!(
        mesh.triangle_count() > 0,
        "the sphere collapsed to no triangles"
    );
    let bbox = mesh.bbox();
    assert!(bbox.size().is_finite(), "outlier poisoned the mesh bbox");
    // The outlier's representative is clamped into the robust box, so the mesh
    // stays at sphere scale (radius ~10 mm) instead of spanning to 1e6 mm.
    assert!(
        bbox.max.max_element() < 1.0e3 && bbox.min.min_element() > -1.0e3,
        "outlier vertex was left in the mesh at full scale: bbox {:?}..{:?}",
        bbox.min,
        bbox.max
    );
}

#[test]
fn stl_nonfinite_corners_thumbnail_stays_visible_and_solid() {
    // ~0.2% NaN/Inf corners in an otherwise valid dense sphere. Dropping those
    // triangles whole keeps the mesh finite and the tile a solid, visible disc.
    let spec = spec_256();
    let bytes = fixtures::dense_binary_stl_sphere_with_nonfinite(4 * 1024 * 1024);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("a 99.8%-valid sphere must still cluster");
    assert!(
        mesh.bbox_uncached().size().is_finite(),
        "non-finite poisoned bbox"
    );

    let pixels = rendering::render_mesh_thumbnail(mesh, spec).expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(holes, 0, "non-finite handling left {holes} interior holes");
}

#[test]
fn stl_huge_coordinate_range_thumbnail_stays_visible() {
    // Coordinate range spans 1e-3..1e6 mm around an mm-scale bulk. Robust bounds
    // trim both extremes and frame the bulk; the tile stays visible.
    let spec = spec_256();
    let bytes = fixtures::dense_binary_stl_huge_coordinate_range(4 * 1024 * 1024);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
        .expect("huge-range STL should cluster its mm-scale bulk");
    assert!(mesh.bbox_uncached().size().is_finite());
    let pixels = rendering::render_mesh_thumbnail(mesh, spec).expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
}

#[test]
fn obj_far_outlier_thumbnails_solid() {
    // OBJ counterpart of the STL outlier regression: a grid plane plus one far
    // outlier triangle must still cluster into a solid, visible surface.
    let spec = spec_256();
    let bytes = fixtures::obj_grid_surface_with_far_outlier(150);
    let mesh = try_read_fast_thumbnail_mesh_for_kind(FormatKind::Obj, &bytes)
        .expect("outlier OBJ should cluster into a surface");
    assert!(!mesh.is_point_cloud());
    assert!(mesh.triangle_count() > 0);
    assert!(mesh.bbox_uncached().size().is_finite());

    let pixels = rendering::render_mesh_thumbnail(mesh, spec).expect("render");
    assert_visible_thumbnail_pixels(&pixels, spec);
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(holes, 0, "OBJ outlier left {holes} interior holes");
}

#[test]
fn all_degenerate_stl_never_returns_a_transparent_tile() {
    // Only zero-area and non-finite triangles: nothing is drawable. The public
    // entry point must return a placeholder (never a transparent bitmap) and
    // must not panic.
    let spec = spec_256();
    let bytes = fixtures::all_degenerate_binary_stl();

    // The fast surrogate declines rather than emitting a 0-triangle "surface".
    assert!(
        try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes).is_none(),
        "the fast path must decline an all-degenerate STL"
    );
    // The mesh loader surfaces a corrupt-file error so the placeholder path runs.
    assert!(
        load_thumbnail_mesh_from_bytes_kind(FormatKind::Stl, &bytes).is_err(),
        "an all-degenerate STL must not load as a renderable mesh"
    );

    let pixels = render_thumbnail_or_placeholder(Some("stl"), &bytes, spec);
    let visible = pixels.chunks_exact(4).filter(|px| px[3] > 0).count();
    assert!(
        visible > 0,
        "the public entry point returned a fully transparent tile for a degenerate file"
    );
    // It is the honest corrupt placeholder, not a real (empty) render.
    assert_eq!(
        pixels,
        placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt),
        "a wholly degenerate recognized file should get the corrupt placeholder"
    );
}
