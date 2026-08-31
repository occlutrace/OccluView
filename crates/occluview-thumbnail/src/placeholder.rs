//! Deterministic placeholder thumbnails for shell fallback paths.
//!
//! When a file cannot be turned into a real 3D preview — corrupt/truncated
//! geometry, an unsupported payload, an encrypted HPS without a key, or a
//! file over the thumbnail size budget — the shell still needs *a* bitmap.
//! Returning nothing makes Explorer fall back to a generic/blank icon, and a
//! failing exit code makes the freedesktop thumbnailer show a broken-image
//! glyph. Both read to the user as "the thumbnails are broken".
//!
//! Instead we draw a small, neutral, studio-shaded 3D cube: a quiet, obviously
//! deliberate placeholder that composites into Explorer's tile exactly like a
//! real thumbnail (transparent background, opaque shaded body). Two variants:
//!
//! * [`PlaceholderKind::Plain`] — a clean cube. Used for content we chose not
//!   to (or cannot) render: over-budget files, unsupported payloads, encrypted
//!   HPS without a key, render/setup timeouts.
//! * [`PlaceholderKind::Corrupt`] — the same cube with a small "!" badge. Used
//!   when a *recognized* format failed to decode (truncated / malformed / bad
//!   signature), i.e. the file itself looks broken.
//!
//! The renderer is pure CPU: it never touches the GPU, filesystem, fonts, or
//! platform APIs, so it is a safe last resort even when the GPU renderer itself
//! is the thing that failed, and it is bit-for-bit deterministic for golden
//! tests.

// Placeholder rasterization is small integer/float pixel math; these casts are
// intentional and bounded (0..=size, 0..=255), and the geometry uses the
// conventional short coordinate names (px/py, dx/dy, bcx/bcy).
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names
)]

use occluview_render::ThumbnailSpec;

/// Which flavor of placeholder to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceholderKind {
    /// A neutral shaded cube. Over-budget, unsupported, encrypted-without-key,
    /// or timed-out inputs — nothing is wrong with the *file*, we just did not
    /// render it.
    Plain,
    /// The cube plus a small "!" badge — a recognized format failed to decode,
    /// so the file content looks broken/corrupt.
    Corrupt,
}

/// Studio-shaded cube face colors (top brightest, then right, then left in
/// shadow) and the badge palette. Neutral gray so the placeholder never reads
/// as a real (warm-clay-tinted) rendered mesh.
const FACE_TOP: [u8; 3] = [201, 206, 212];
const FACE_RIGHT: [u8; 3] = [170, 176, 183];
const FACE_LEFT: [u8; 3] = [140, 146, 153];
const BADGE_DISC: [u8; 3] = [84, 90, 99];
const BADGE_GLYPH: [u8; 3] = [228, 231, 235];

/// Half-width of an isometric cube rhombus as a fraction of its "radius":
/// `cos(30°)`. Kept as a constant so the hexagon stays a true iso cube.
const SQRT3_OVER_2: f32 = 0.866_025_4;

/// Supersampling factor for the placeholder rasterizer. 3×3 is enough to keep
/// the cube's diagonal edges and the badge smooth at 256px while staying
/// trivially cheap (placeholders must not do full-quality render work).
const SUPERSAMPLE: usize = 3;

/// Build an opaque-body / transparent-background RGBA placeholder thumbnail.
///
/// Back-compatible entry point: draws the [`PlaceholderKind::Plain`] cube.
#[must_use]
pub fn placeholder_thumbnail(spec: ThumbnailSpec) -> Vec<u8> {
    placeholder_thumbnail_kind(spec, PlaceholderKind::Plain)
}

/// Build an RGBA placeholder thumbnail of the requested [`PlaceholderKind`].
///
/// Rows are top-to-bottom, length `spec.size_px * spec.size_px * 4`. The cube
/// body is opaque; the surrounding background is transparent so it composites
/// like a real thumbnail. Pure and deterministic.
#[must_use]
pub fn placeholder_thumbnail_kind(spec: ThumbnailSpec, kind: PlaceholderKind) -> Vec<u8> {
    let size = usize::from(spec.size_px.max(1));
    let mut pixels = vec![0u8; size * size * 4];
    draw_studio_cube(&mut pixels, size);
    if kind == PlaceholderKind::Corrupt {
        draw_corrupt_badge(&mut pixels, size);
    }
    pixels
}

/// A single isometric cube face expressed as a convex screen-space quad plus
/// its flat shade.
struct CubeFace {
    quad: [[f32; 2]; 4],
    color: [u8; 3],
}

fn cube_faces(size: usize) -> [CubeFace; 3] {
    let s = size as f32;
    let cx = s * 0.5;
    let cy = s * 0.5;
    let r = s * 0.34;
    let hx = SQRT3_OVER_2 * r;

    // Pointy-top hexagon vertices (screen y grows downward) + the center vertex
    // where the three visible faces meet.
    let top = [cx, cy - r];
    let upper_right = [cx + hx, cy - r * 0.5];
    let lower_right = [cx + hx, cy + r * 0.5];
    let bottom = [cx, cy + r];
    let lower_left = [cx - hx, cy + r * 0.5];
    let upper_left = [cx - hx, cy - r * 0.5];
    let mid = [cx, cy];

    [
        CubeFace {
            quad: [top, upper_right, mid, upper_left],
            color: FACE_TOP,
        },
        CubeFace {
            quad: [upper_right, lower_right, bottom, mid],
            color: FACE_RIGHT,
        },
        CubeFace {
            quad: [upper_left, mid, bottom, lower_left],
            color: FACE_LEFT,
        },
    ]
}

fn draw_studio_cube(pixels: &mut [u8], size: usize) {
    let faces = cube_faces(size);
    let ss = SUPERSAMPLE;
    let sub_total = (ss * ss) as u32;
    let inv = 1.0 / ss as f32;

    for y in 0..size {
        for x in 0..size {
            let mut covered = 0u32;
            let mut sum = [0u32; 3];
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) * inv;
                    let py = y as f32 + (sy as f32 + 0.5) * inv;
                    if let Some(color) = face_color_at(&faces, px, py) {
                        sum[0] += u32::from(color[0]);
                        sum[1] += u32::from(color[1]);
                        sum[2] += u32::from(color[2]);
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let idx = (y * size + x) * 4;
            pixels[idx] = (sum[0] / covered) as u8;
            pixels[idx + 1] = (sum[1] / covered) as u8;
            pixels[idx + 2] = (sum[2] / covered) as u8;
            pixels[idx + 3] = ((covered * 255) / sub_total) as u8;
        }
    }
}

fn face_color_at(faces: &[CubeFace; 3], px: f32, py: f32) -> Option<[u8; 3]> {
    faces
        .iter()
        .find(|face| point_in_quad(&face.quad, px, py))
        .map(|face| face.color)
}

/// Convex point-in-quad test: the point is inside when it stays on one side of
/// every edge (all edge signs share a sign, ignoring exact-on-edge zeros).
fn point_in_quad(quad: &[[f32; 2]; 4], px: f32, py: f32) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let a = quad[i];
        let b = quad[(i + 1) % 4];
        let cross = (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
        if cross > 0.0 {
            positive = true;
        } else if cross < 0.0 {
            negative = true;
        }
        if positive && negative {
            return false;
        }
    }
    true
}

/// Overlay a small "!" badge in the lower-right, alpha-composited over whatever
/// is already there (cube body or transparent background).
fn draw_corrupt_badge(pixels: &mut [u8], size: usize) {
    let s = size as f32;
    let bcx = s * 0.73;
    let bcy = s * 0.73;
    let br = (s * 0.13).max(3.0);
    let ss = SUPERSAMPLE;
    let sub_total = (ss * ss) as f32;
    let inv = 1.0 / ss as f32;

    // Bounding box of the badge (+1px slack for the antialiased rim).
    let lo_x = ((bcx - br - 1.0).floor().max(0.0)) as usize;
    let lo_y = ((bcy - br - 1.0).floor().max(0.0)) as usize;
    let hi_x = ((bcx + br + 1.0).ceil() as usize).min(size);
    let hi_y = ((bcy + br + 1.0).ceil() as usize).min(size);

    for y in lo_y..hi_y {
        for x in lo_x..hi_x {
            let mut disc_hits = 0.0_f32;
            let mut glyph_hits = 0.0_f32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) * inv;
                    let py = y as f32 + (sy as f32 + 0.5) * inv;
                    let dx = px - bcx;
                    let dy = py - bcy;
                    if dx * dx + dy * dy > br * br {
                        continue;
                    }
                    disc_hits += 1.0;
                    if glyph_hit(dx, dy, br) {
                        glyph_hits += 1.0;
                    }
                }
            }
            if disc_hits == 0.0 {
                continue;
            }
            // Glyph pixels are fully opaque badge-glyph color; the rest of the
            // disc is badge-disc color. The disc coverage is the composite
            // alpha over the existing pixel.
            let glyph_fraction = glyph_hits / disc_hits;
            let color = [
                mix(BADGE_DISC[0], BADGE_GLYPH[0], glyph_fraction),
                mix(BADGE_DISC[1], BADGE_GLYPH[1], glyph_fraction),
                mix(BADGE_DISC[2], BADGE_GLYPH[2], glyph_fraction),
            ];
            let alpha = disc_hits / sub_total;
            blend_over(pixels, (y * size + x) * 4, color, alpha);
        }
    }
}

/// Is the badge-local point `(dx, dy)` inside the "!" glyph? A rounded vertical
/// stem plus a dot below it.
fn glyph_hit(dx: f32, dy: f32, br: f32) -> bool {
    let stem = dx.abs() <= br * 0.15 && dy >= -br * 0.52 && dy <= br * 0.14;
    let dot_dy = dy - br * 0.40;
    let dot = dx * dx + dot_dy * dot_dy <= (br * 0.17) * (br * 0.17);
    stem || dot
}

fn mix(from: u8, to: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (f32::from(from) * (1.0 - t) + f32::from(to) * t).round() as u8
}

/// Straight-alpha "source over destination" composite of an opaque-ish source
/// color at coverage `alpha` onto the RGBA pixel at `idx`.
fn blend_over(pixels: &mut [u8], idx: usize, src: [u8; 3], alpha: f32) {
    let alpha = alpha.clamp(0.0, 1.0);
    let dst_a = f32::from(pixels[idx + 3]) / 255.0;
    let out_a = alpha + dst_a * (1.0 - alpha);
    if out_a <= f32::EPSILON {
        return;
    }
    for c in 0..3 {
        let src_c = f32::from(src[c]);
        let dst_c = f32::from(pixels[idx + c]);
        let value = (src_c * alpha + dst_c * dst_a * (1.0 - alpha)) / out_a;
        pixels[idx + c] = value.round().clamp(0.0, 255.0) as u8;
    }
    pixels[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(size: u16) -> ThumbnailSpec {
        ThumbnailSpec {
            size_px: size,
            ..Default::default()
        }
    }

    fn pixel(pixels: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let idx = (y * size + x) * 4;
        [
            pixels[idx],
            pixels[idx + 1],
            pixels[idx + 2],
            pixels[idx + 3],
        ]
    }

    #[test]
    fn placeholder_has_requested_rgba_size() {
        assert_eq!(placeholder_thumbnail(spec(16)).len(), 16 * 16 * 4);
        assert_eq!(
            placeholder_thumbnail_kind(spec(24), PlaceholderKind::Corrupt).len(),
            24 * 24 * 4
        );
    }

    #[test]
    fn plain_and_corrupt_are_deterministic() {
        for kind in [PlaceholderKind::Plain, PlaceholderKind::Corrupt] {
            let a = placeholder_thumbnail_kind(spec(48), kind);
            let b = placeholder_thumbnail_kind(spec(48), kind);
            assert_eq!(a, b, "{kind:?} placeholder is not deterministic");
        }
    }

    #[test]
    fn background_is_transparent_and_cube_body_is_opaque() {
        let size = 64usize;
        let pixels = placeholder_thumbnail(spec(size as u16));
        // Corners are outside the centered cube: transparent.
        for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
            assert_eq!(
                pixel(&pixels, size, x, y)[3],
                0,
                "corner ({x},{y}) should be transparent background"
            );
        }
        // The center is on the cube body: opaque.
        assert_eq!(pixel(&pixels, size, size / 2, size / 2)[3], 255);
    }

    #[test]
    fn cube_shows_three_distinct_shaded_faces() {
        let size = 128usize;
        let pixels = placeholder_thumbnail(spec(size as u16));
        // Sample a point inside each visible face by construction.
        let top = pixel(&pixels, size, size / 2, size * 5 / 16);
        let right = pixel(&pixels, size, size * 10 / 16, size * 9 / 16);
        let left = pixel(&pixels, size, size * 6 / 16, size * 9 / 16);
        for sample in [top, right, left] {
            assert_eq!(sample[3], 255, "face sample should be opaque");
        }
        // Studio shading: top brighter than right, right brighter than left.
        assert!(
            top[0] > right[0] && right[0] > left[0],
            "expected top>right>left shading, got top={top:?} right={right:?} left={left:?}"
        );
    }

    #[test]
    fn corrupt_differs_from_plain_only_by_the_badge() {
        let size = 128usize;
        let plain = placeholder_thumbnail_kind(spec(size as u16), PlaceholderKind::Plain);
        let corrupt = placeholder_thumbnail_kind(spec(size as u16), PlaceholderKind::Corrupt);
        assert_ne!(plain, corrupt, "corrupt badge should change the image");

        // Every differing pixel must lie in the lower-right badge region.
        let mut diffs = 0usize;
        for y in 0..size {
            for x in 0..size {
                if pixel(&plain, size, x, y) != pixel(&corrupt, size, x, y) {
                    diffs += 1;
                    assert!(
                        x > size / 2 && y > size / 2,
                        "unexpected diff outside badge at ({x},{y})"
                    );
                }
            }
        }
        assert!(diffs > 0, "corrupt badge produced no visible difference");
    }

    #[test]
    fn placeholder_does_not_look_like_a_blue_rendered_cube() {
        let pixels = placeholder_thumbnail(spec(32));
        let saturated_blue_pixels = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0)
            .filter(|px| px[2] > px[0].saturating_add(45) && px[2] > px[1].saturating_add(25))
            .count();
        assert_eq!(saturated_blue_pixels, 0);
    }

    #[test]
    fn tiny_sizes_still_produce_a_visible_body() {
        for size in [1u16, 2, 4, 8] {
            let pixels = placeholder_thumbnail(spec(size));
            assert_eq!(pixels.len(), usize::from(size) * usize::from(size) * 4);
            let any_opaque = pixels.as_chunks::<4>().0.iter().any(|px| px[3] > 0);
            assert!(any_opaque, "size {size} placeholder rendered nothing");
        }
    }
}
