use super::*;
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod api;
mod architecture;
mod cache_and_jobs;
mod fixtures;
mod framing;
mod placeholder_ladder;
mod robustness;

fn mod_source() -> &'static str {
    include_str!("../mod.rs")
}

fn cache_source() -> &'static str {
    include_str!("../cache.rs")
}

fn concurrency_source() -> &'static str {
    include_str!("../concurrency.rs")
}

fn loading_source() -> &'static str {
    include_str!("../loading.rs")
}

fn rendering_source() -> &'static str {
    include_str!("../rendering.rs")
}

fn offscreen_factory_source() -> &'static str {
    include_str!("../../offscreen_factory.rs")
}

fn assert_tint_eq(actual: [f32; 4], expected: [f32; 4]) {
    assert!(
        actual
            .into_iter()
            .zip(expected)
            .all(|(left, right)| left.to_bits() == right.to_bits()),
        "actual={actual:?} expected={expected:?}"
    );
}

/// Count transparent pixels strictly inside the per-row silhouette span. For a
/// convex silhouette (a sphere renders as a filled disc) every such pixel is a
/// see-through hole — the speckle artifact. A solid surface scores 0.
fn interior_hole_count(pixels: &[u8], size: usize) -> usize {
    let mut holes = 0usize;
    for y in 0..size {
        let mut left = None;
        let mut right = 0usize;
        for x in 0..size {
            if pixels[(y * size + x) * 4 + 3] > 0 {
                if left.is_none() {
                    left = Some(x);
                }
                right = x;
            }
        }
        if let Some(left) = left {
            for x in left..=right {
                if pixels[(y * size + x) * 4 + 3] == 0 {
                    holes += 1;
                }
            }
        }
    }
    holes
}

fn assert_transparent_thumbnail_with_mesh_pixels(pixels: &[u8], spec: ThumbnailSpec) {
    let pixel_count = usize::from(spec.size_px) * usize::from(spec.size_px);
    let expected_len = pixel_count * 4;
    assert_eq!(pixels.len(), expected_len);

    let transparent = pixels.chunks_exact(4).filter(|px| px[3] == 0).count();
    let opaque = pixels.chunks_exact(4).filter(|px| px[3] == 255).count();

    assert!(
        transparent > pixel_count / 16,
        "thumbnail should keep transparent background pixels"
    );
    assert!(
        opaque > (pixel_count / 64).max(4),
        "thumbnail should contain a visible rendered mesh"
    );
}

/// Like [`assert_transparent_thumbnail_with_mesh_pixels`], but for fixtures
/// whose geometry is deliberately thin/sparse (stress fixtures scattering
/// many sub-pixel triangles, or a heavily decimated fast-surrogate mesh).
/// Once supersampling covers a given thumbnail size (see
/// `MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX` in `rendering.rs`), such geometry
/// legitimately antialiases to partial (non-255) edge alpha everywhere -
/// there may be no fully interior, fully-opaque pixel at all. This checks for
/// *any* visible coverage instead of requiring hard-opaque pixels.
fn assert_visible_thumbnail_pixels(pixels: &[u8], spec: ThumbnailSpec) {
    let pixel_count = usize::from(spec.size_px) * usize::from(spec.size_px);
    let expected_len = pixel_count * 4;
    assert_eq!(pixels.len(), expected_len);

    let transparent = pixels.chunks_exact(4).filter(|px| px[3] == 0).count();
    let visible = pixels.chunks_exact(4).filter(|px| px[3] > 0).count();

    assert!(
        transparent > pixel_count / 16,
        "thumbnail should keep transparent background pixels"
    );
    assert!(
        visible > (pixel_count / 64).max(4),
        "thumbnail should contain a visible rendered mesh"
    );
}
