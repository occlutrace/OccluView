//! Runtime CPU rasteriser for the Explorer preview context-menu icons.
//!
//! Win32 popup menus take an `HBITMAP` per item (`MENUITEMINFOW::hbmpItem`), not
//! a font glyph, so we draw our own 16 px icons. There are no asset files: each
//! glyph is composed from a handful of anti-aliased primitives (thick segments,
//! discs, convex fills) sampled on a tiny coverage buffer. That keeps the icons
//! deterministic, DPI-scalable (the caller passes the pixel size), and unit
//! testable on any host — no GPU, no Windows.
//!
//! The output is **straight** RGBA where the glyph is white (`255,255,255`) and
//! alpha carries the coverage mask. The Windows layer tints that mask with the
//! system menu-text colour and premultiplies it when it builds the 32bpp DIB
//! (menus alpha-blend `hbmpItem`), so the icons follow light/dark menu themes
//! for free. This module stays presentation-only and Win32-free.

// The rasteriser is inherently full of `f32`↔pixel-index casts and short math
// identifiers; these pedantic lints are noise here and are relaxed for the
// whole module (mirroring `occluview-app`'s hand-drawn icon module).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::too_many_arguments
)]

/// One preview-menu glyph. Maps 1:1 onto an actionable menu command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewMenuIcon {
    /// Open externally: a tray/box with an arrow leaving it.
    Open,
    /// Edit: a pencil on the diagonal.
    Edit,
    /// Front face of an isometric cube highlighted.
    CubeFront,
    /// Top face of an isometric cube highlighted.
    CubeTop,
    /// Right (side) face of an isometric cube highlighted.
    CubeSide,
    /// Isometric cube outline, no face highlighted.
    CubeIsometric,
    /// Four inward corner brackets: fit to frame.
    FitFrame,
    /// A subdivided triangle: wireframe mesh.
    Wireframe,
    /// Two overlapping frames with a picture mark: copy image.
    CopyImage,
}

impl PreviewMenuIcon {
    /// Rasterise this glyph to a top-down straight-RGBA buffer of
    /// `size_px * size_px * 4` bytes (white glyph, coverage in alpha).
    pub(crate) fn rasterize(self, size_px: u32) -> Vec<u8> {
        let mut raster = Raster::new(size_px.max(1) as usize);
        self.paint(&mut raster);
        raster.into_rgba()
    }

    fn paint(self, r: &mut Raster) {
        // Uniform stroke half-width in normalized units; ~1.4 px at 16 px.
        let hw = 0.045;
        match self {
            Self::Open => paint_open(r, hw),
            Self::Edit => paint_edit(r, hw),
            Self::CubeFront => paint_cube(r, hw, Some(CubeFace::Front)),
            Self::CubeTop => paint_cube(r, hw, Some(CubeFace::Top)),
            Self::CubeSide => paint_cube(r, hw, Some(CubeFace::Side)),
            Self::CubeIsometric => paint_cube(r, hw, None),
            Self::FitFrame => paint_fit(r, hw),
            Self::Wireframe => paint_wireframe(r, hw),
            Self::CopyImage => paint_copy(r, hw),
        }
    }
}

type P = (f32, f32);

fn paint_open(r: &mut Raster, hw: f32) {
    // An open outbound tray (three sides) with an arrow leaving up-and-right.
    r.polyline(
        &[(0.12, 0.44), (0.12, 0.86), (0.72, 0.86), (0.72, 0.52)],
        hw,
    );
    // Arrow shaft from inside the tray toward the upper-right.
    r.stroke((0.40, 0.58), (0.86, 0.16), hw);
    // Arrowhead barbs.
    r.stroke((0.86, 0.16), (0.62, 0.18), hw);
    r.stroke((0.86, 0.16), (0.84, 0.42), hw);
}

fn paint_edit(r: &mut Raster, hw: f32) {
    // Pencil: a thick shaft, a filled nib at the lower-left, a cap at the top.
    r.stroke((0.30, 0.72), (0.74, 0.28), hw);
    r.stroke((0.24, 0.66), (0.68, 0.22), hw);
    // Nib (filled triangle) at the writing end.
    r.fill_poly(&[(0.14, 0.86), (0.30, 0.74), (0.26, 0.62)]);
    // Flat cap (eraser) at the far end.
    r.stroke((0.68, 0.22), (0.80, 0.34), hw);
}

#[derive(Clone, Copy)]
enum CubeFace {
    Top,
    Front,
    Side,
}

fn paint_cube(r: &mut Raster, hw: f32, highlight: Option<CubeFace>) {
    // Isometric cube: a hexagon outline plus a "Y" of internal edges meeting at
    // the centre. Highlighting one rhombic face names a view direction.
    let m: P = (0.50, 0.50);
    let p0: P = (0.50, 0.10); // top
    let p1: P = (0.15, 0.30); // upper-left
    let p2: P = (0.15, 0.70); // lower-left
    let p3: P = (0.50, 0.90); // bottom
    let p4: P = (0.85, 0.70); // lower-right
    let p5: P = (0.85, 0.30); // upper-right

    if let Some(face) = highlight {
        let quad: [P; 4] = match face {
            CubeFace::Top => [p0, p1, m, p5],
            CubeFace::Front => [p1, p2, p3, m],
            CubeFace::Side => [p5, m, p3, p4],
        };
        r.fill_poly(&quad);
    }

    // Hexagon outline.
    r.polyline(&[p0, p1, p2, p3, p4, p5, p0], hw);
    // Internal "Y" edges.
    r.stroke(m, p0, hw);
    r.stroke(m, p2, hw);
    r.stroke(m, p4, hw);
}

fn paint_fit(r: &mut Raster, hw: f32) {
    // Four inward corner brackets = "fit to frame".
    let arm = 0.22;
    let (lo, hi) = (0.14, 0.86);
    // Top-left.
    r.stroke((lo, lo), (lo + arm, lo), hw);
    r.stroke((lo, lo), (lo, lo + arm), hw);
    // Top-right.
    r.stroke((hi, lo), (hi - arm, lo), hw);
    r.stroke((hi, lo), (hi, lo + arm), hw);
    // Bottom-left.
    r.stroke((lo, hi), (lo + arm, hi), hw);
    r.stroke((lo, hi), (lo, hi - arm), hw);
    // Bottom-right.
    r.stroke((hi, hi), (hi - arm, hi), hw);
    r.stroke((hi, hi), (hi, hi - arm), hw);
}

fn paint_wireframe(r: &mut Raster, hw: f32) {
    // A triangle subdivided at the edge midpoints into four faces.
    let a: P = (0.50, 0.12);
    let b: P = (0.12, 0.84);
    let c: P = (0.88, 0.84);
    let ab: P = midpoint(a, b);
    let bc: P = midpoint(b, c);
    let ca: P = midpoint(c, a);
    r.polyline(&[a, b, c, a], hw);
    r.polyline(&[ab, bc, ca, ab], hw);
}

fn paint_copy(r: &mut Raster, hw: f32) {
    // Two overlapping frames; the front one carries a small picture mark.
    r.rect((0.14, 0.14), (0.60, 0.60), hw); // back frame
    r.rect((0.38, 0.38), (0.86, 0.86), hw); // front frame
                                            // Picture mark inside the front frame: a sun disc and a mountain.
    r.disc((0.50, 0.52), 0.045);
    r.fill_poly(&[(0.44, 0.80), (0.60, 0.56), (0.78, 0.80)]);
}

fn midpoint(a: P, b: P) -> P {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// A single-channel anti-aliased coverage buffer in `[0, 1]`, sampled at pixel
/// centres. Shapes are max-blended so overlaps do not darken.
struct Raster {
    size: usize,
    cov: Vec<f32>,
}

impl Raster {
    fn new(size: usize) -> Self {
        Self {
            size,
            cov: vec![0.0; size * size],
        }
    }

    fn blend(&mut self, x: usize, y: usize, c: f32) {
        if x < self.size && y < self.size && c > 0.0 {
            let i = y * self.size + x;
            if c > self.cov[i] {
                self.cov[i] = c;
            }
        }
    }

    /// A thick, round-capped segment between two normalized points.
    fn stroke(&mut self, a: P, b: P, hw_norm: f32) {
        let s = self.size as f32;
        let (ax, ay) = (a.0 * s, a.1 * s);
        let (bx, by) = (b.0 * s, b.1 * s);
        let hw = hw_norm * s;
        let (min_x, min_y, max_x, max_y) = self.bounds(
            ax.min(bx) - hw,
            ay.min(by) - hw,
            ax.max(bx) + hw,
            ay.max(by) + hw,
        );
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = dist_point_segment(px, py, ax, ay, bx, by);
                let c = (hw + 0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, c);
            }
        }
    }

    /// A connected polyline of normalized points.
    fn polyline(&mut self, pts: &[P], hw_norm: f32) {
        for window in pts.windows(2) {
            self.stroke(window[0], window[1], hw_norm);
        }
    }

    /// A hollow rectangle outline from two opposite corners.
    fn rect(&mut self, min: P, max: P, hw_norm: f32) {
        let tl = (min.0, min.1);
        let tr = (max.0, min.1);
        let br = (max.0, max.1);
        let bl = (min.0, max.1);
        self.polyline(&[tl, tr, br, bl, tl], hw_norm);
    }

    /// A filled disc.
    fn disc(&mut self, center: P, radius_norm: f32) {
        let s = self.size as f32;
        let (cx, cy) = (center.0 * s, center.1 * s);
        let radius = radius_norm * s;
        let (min_x, min_y, max_x, max_y) =
            self.bounds(cx - radius, cy - radius, cx + radius, cy + radius);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = (px - cx).hypot(py - cy);
                let c = (radius + 0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, c);
            }
        }
    }

    /// A filled convex polygon (winding-agnostic: interior is the centroid side
    /// of every edge).
    fn fill_poly(&mut self, pts: &[P]) {
        if pts.len() < 3 {
            return;
        }
        let s = self.size as f32;
        let px: Vec<P> = pts.iter().map(|&(x, y)| (x * s, y * s)).collect();
        let cx = px.iter().map(|p| p.0).sum::<f32>() / px.len() as f32;
        let cy = px.iter().map(|p| p.1).sum::<f32>() / px.len() as f32;
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in &px {
            lo_x = lo_x.min(x);
            lo_y = lo_y.min(y);
            hi_x = hi_x.max(x);
            hi_y = hi_y.max(y);
        }
        let (min_x, min_y, max_x, max_y) =
            self.bounds(lo_x - 1.0, lo_y - 1.0, hi_x + 1.0, hi_y + 1.0);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (sx, sy) = (x as f32 + 0.5, y as f32 + 0.5);
                let mut inside = f32::MAX;
                for i in 0..px.len() {
                    let a = px[i];
                    let b = px[(i + 1) % px.len()];
                    inside = inside.min(signed_edge_distance(sx, sy, a, b, cx, cy));
                }
                let c = (inside + 0.5).clamp(0.0, 1.0);
                self.blend(x, y, c);
            }
        }
    }

    /// Clamp a float bounding box to valid pixel index ranges.
    fn bounds(
        &self,
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
    ) -> (usize, usize, usize, usize) {
        let last = self.size.saturating_sub(1);
        let clamp = |v: f32| (v.floor().max(0.0) as usize).min(last);
        (clamp(min_x), clamp(min_y), clamp(max_x), clamp(max_y))
    }

    fn into_rgba(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size * self.size * 4);
        for c in self.cov {
            let a = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.extend_from_slice(&[255, 255, 255, a]);
        }
        out
    }
}

/// Distance from a point to a line segment.
fn dist_point_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f32::EPSILON {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    (px - cx).hypot(py - cy)
}

/// Signed distance from `(px, py)` to the edge `a..b`, positive on the side
/// where the polygon centroid `(cx, cy)` lies.
fn signed_edge_distance(px: f32, py: f32, a: P, b: P, cx: f32, cy: f32) -> f32 {
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let len = ex.hypot(ey);
    if len <= f32::EPSILON {
        return 0.0;
    }
    // Left-hand normal of the edge.
    let (nx, ny) = (-ey / len, ex / len);
    let side = (px - a.0) * nx + (py - a.1) * ny;
    let centroid_side = (cx - a.0) * nx + (cy - a.1) * ny;
    if centroid_side < 0.0 {
        -side
    } else {
        side
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ICONS: [PreviewMenuIcon; 9] = [
        PreviewMenuIcon::Open,
        PreviewMenuIcon::Edit,
        PreviewMenuIcon::CubeFront,
        PreviewMenuIcon::CubeTop,
        PreviewMenuIcon::CubeSide,
        PreviewMenuIcon::CubeIsometric,
        PreviewMenuIcon::FitFrame,
        PreviewMenuIcon::Wireframe,
        PreviewMenuIcon::CopyImage,
    ];

    fn alpha_at(pixels: &[u8], size: usize, x: usize, y: usize) -> u8 {
        pixels[(y * size + x) * 4 + 3]
    }

    #[test]
    fn every_icon_rasterizes_to_the_requested_size() {
        for size in [16u32, 20, 32] {
            for icon in ALL_ICONS {
                let pixels = icon.rasterize(size);
                assert_eq!(
                    pixels.len(),
                    (size * size * 4) as usize,
                    "icon {icon:?} at {size}px should fill the buffer"
                );
            }
        }
    }

    #[test]
    fn icons_draw_something_but_leave_transparent_padding() {
        let size = 16usize;
        for icon in ALL_ICONS {
            let pixels = icon.rasterize(size as u32);
            let total_alpha: u32 = pixels
                .iter()
                .skip(3)
                .step_by(4)
                .map(|&a| u32::from(a))
                .sum();
            assert!(
                total_alpha > 0,
                "icon {icon:?} should draw at least one pixel"
            );
            for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                assert_eq!(
                    alpha_at(&pixels, size, x, y),
                    0,
                    "icon {icon:?} should keep corner ({x},{y}) transparent"
                );
            }
        }
    }

    #[test]
    fn drawn_pixels_are_white_with_coverage_in_alpha() {
        for icon in ALL_ICONS {
            let pixels = icon.rasterize(16);
            for chunk in pixels.chunks_exact(4) {
                if chunk[3] > 0 {
                    assert_eq!(
                        [chunk[0], chunk[1], chunk[2]],
                        [255, 255, 255],
                        "icon {icon:?} must emit a white straight-alpha mask"
                    );
                }
            }
        }
    }

    #[test]
    fn rasterization_is_deterministic() {
        for icon in ALL_ICONS {
            assert_eq!(
                icon.rasterize(16),
                icon.rasterize(16),
                "icon {icon:?} must rasterise identically every time"
            );
        }
    }

    #[test]
    fn cube_face_highlights_are_distinct() {
        // The three face highlights must differ from each other and from the
        // plain isometric outline, so the view presets are visually separable.
        let plain = PreviewMenuIcon::CubeIsometric.rasterize(16);
        let top = PreviewMenuIcon::CubeTop.rasterize(16);
        let front = PreviewMenuIcon::CubeFront.rasterize(16);
        let side = PreviewMenuIcon::CubeSide.rasterize(16);
        assert_ne!(plain, top);
        assert_ne!(top, front);
        assert_ne!(front, side);
        assert_ne!(top, side);
    }
}
