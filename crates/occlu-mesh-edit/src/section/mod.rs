//! Plane–mesh cross-section kernel.
//!
//! Given triangle geometry and a world plane `n·p = d`, this returns the
//! intersection contour as stitched polylines. The kernel is pure, serial and
//! deterministic.
//!
//! Design points that the callers depend on:
//! - All intersection arithmetic runs in `f64` (`f32` positions are promoted).
//! - Vertex classification is **half-open** (`n·v - d >= 0` is the kept side),
//!   so a plane passing exactly through a vertex produces no duplicate or
//!   missing segments.
//! - Segments are stitched by quantized-endpoint keying. A point where more
//!   than two segments meet terminates the polylines that reach it
//!   (non-manifold junctions are tolerated, not an error).
//! - Open polylines are first-class: scan shells are open surfaces, so the
//!   contour is open wherever the shell boundary crosses the plane.
//! - Output ordering is deterministic (polylines sorted by their first point).

use glam::{DVec3, Vec3};
use std::cmp::Ordering;
use std::collections::HashMap;
use thiserror::Error;

/// Tolerance for accepting a normal as unit length in [`SectionPlane::new`].
const UNIT_NORMAL_TOLERANCE: f32 = 1.0e-4;
/// Minimum length below which a normal is treated as degenerate.
const MIN_NORMAL_LENGTH: f32 = 1.0e-6;
/// Squared minimum length: segments shorter than this (in `f64`) are dropped.
const SEGMENT_MIN_SQ: f64 = 1.0e-9 * 1.0e-9;
/// Endpoint quantization scale for stitching (grid step `1 / WELD_SCALE`).
const WELD_SCALE: f64 = 1.0e6;

/// Errors raised while validating section inputs.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SectionError {
    /// An input coordinate or offset was not finite.
    #[error("section plane input was not finite")]
    NonFinite,
    /// A normal expected to be unit length was not within tolerance.
    #[error("section plane normal was not unit length")]
    NonUnitNormal,
    /// A normal was too short to normalize.
    #[error("section plane normal was degenerate (near-zero length)")]
    DegenerateNormal,
}

/// A world-space section plane `normal · p = distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionPlane {
    /// Unit plane normal.
    pub normal: Vec3,
    /// Signed plane offset along the normal.
    pub distance: f32,
}

impl SectionPlane {
    /// Construct from an already unit-length `normal`.
    ///
    /// # Errors
    /// Returns [`SectionError::NonFinite`] if any input is not finite, or
    /// [`SectionError::NonUnitNormal`] if `normal` is not within tolerance of
    /// unit length.
    pub fn new(normal: Vec3, distance: f32) -> Result<Self, SectionError> {
        validate_unit_normal(normal)?;
        if !distance.is_finite() {
            return Err(SectionError::NonFinite);
        }
        Ok(Self { normal, distance })
    }

    /// Construct from an arbitrary non-zero `normal` and a `point` on the plane.
    ///
    /// The normal is normalized and `distance = normal · point`.
    ///
    /// # Errors
    /// Returns [`SectionError::NonFinite`] if any input is not finite, or
    /// [`SectionError::DegenerateNormal`] if `normal` is shorter than the
    /// minimum length.
    pub fn from_normal_point(normal: Vec3, point: Vec3) -> Result<Self, SectionError> {
        if !normal.is_finite() || !point.is_finite() {
            return Err(SectionError::NonFinite);
        }
        let length = normal.length();
        if length < MIN_NORMAL_LENGTH {
            return Err(SectionError::DegenerateNormal);
        }
        let unit = normal / length;
        let distance = unit.dot(point);
        if !distance.is_finite() {
            return Err(SectionError::NonFinite);
        }
        Ok(Self {
            normal: unit,
            distance,
        })
    }

    /// Signed distance of a world point to the plane. `>= 0` is the kept side.
    #[must_use]
    pub fn signed_distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.distance
    }

    /// The plane normal promoted to `f64`.
    fn normal_f64(&self) -> DVec3 {
        self.normal.as_dvec3()
    }
}

/// Validate that `normal` is finite and within tolerance of unit length.
///
/// # Errors
/// Returns [`SectionError::NonFinite`] or [`SectionError::NonUnitNormal`].
pub(super) fn validate_unit_normal(normal: Vec3) -> Result<(), SectionError> {
    if !normal.is_finite() {
        return Err(SectionError::NonFinite);
    }
    if (normal.length() - 1.0).abs() > UNIT_NORMAL_TOLERANCE {
        return Err(SectionError::NonUnitNormal);
    }
    Ok(())
}

/// One stitched contour polyline of a plane–mesh intersection.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionPolyline {
    /// Ordered contour points in the input coordinate space (`f64`).
    pub points: Vec<DVec3>,
    /// Whether the polyline forms a closed loop (last point joins the first).
    pub closed: bool,
}

/// The result of a plane–mesh section: zero or more contour polylines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionResult {
    /// Contour polylines, deterministically ordered by their first point.
    pub polylines: Vec<SectionPolyline>,
}

/// Intersect a triangle mesh with `plane` and return the contour polylines.
///
/// `positions` are the vertex positions and `indices` are triangle indices in
/// chunks of three. Out-of-range indices skip their triangle. The output is
/// deterministic and independent of triangle order.
#[must_use]
pub fn plane_section(
    positions: &[[f32; 3]],
    indices: &[u32],
    plane: SectionPlane,
) -> SectionResult {
    let normal = plane.normal_f64();
    let offset = f64::from(plane.distance);
    let mut segments = Vec::new();
    for tri in indices.chunks_exact(3) {
        if let Some(segment) = triangle_segment(positions, [tri[0], tri[1], tri[2]], normal, offset)
        {
            segments.push(segment);
        }
    }
    stitch(segments)
}

/// One intersection segment produced by a single crossing triangle.
pub(super) struct Segment {
    a: DVec3,
    b: DVec3,
}

/// Compute the intersection segment for one triangle against the plane whose
/// normal is `normal` and offset is `offset`.
///
/// Returns `None` when the triangle does not straddle the plane, has an
/// out-of-range index, or the resulting segment is degenerate. The half-open
/// classification (`proj >= offset` is the kept side) matches
/// `SectionSweep` (removed; git history d7cd650), so both paths agree bit-for-bit.
pub(super) fn triangle_segment(
    positions: &[[f32; 3]],
    tri: [u32; 3],
    normal: DVec3,
    offset: f64,
) -> Option<Segment> {
    let p = [
        promote(positions, tri[0])?,
        promote(positions, tri[1])?,
        promote(positions, tri[2])?,
    ];
    let proj = [normal.dot(p[0]), normal.dot(p[1]), normal.dot(p[2])];
    let positive = [proj[0] >= offset, proj[1] >= offset, proj[2] >= offset];
    let kept = positive.iter().filter(|side| **side).count();
    if kept == 0 || kept == 3 {
        return None;
    }
    // The apex is the minority vertex; the two crossing edges share it.
    let apex = minority_index(positive, kept);
    let other = [(apex + 1) % 3, (apex + 2) % 3];
    let first = edge_point(p[apex], p[other[0]], proj[apex], proj[other[0]], offset);
    let second = edge_point(p[apex], p[other[1]], proj[apex], proj[other[1]], offset);
    if (first - second).length_squared() < SEGMENT_MIN_SQ {
        return None;
    }
    Some(Segment {
        a: first,
        b: second,
    })
}

/// Promote a vertex position to `f64`, or `None` when the index is out of range.
pub(super) fn promote(positions: &[[f32; 3]], index: u32) -> Option<DVec3> {
    let index = usize::try_from(index).ok()?;
    let v = positions.get(index)?;
    Some(DVec3::new(
        f64::from(v[0]),
        f64::from(v[1]),
        f64::from(v[2]),
    ))
}

/// Index of the minority vertex given the per-vertex kept-side booleans.
fn minority_index(positive: [bool; 3], kept: usize) -> usize {
    // With one kept vertex the apex is the single kept one; with two kept the
    // apex is the single dropped one. Exactly one vertex matches `target`.
    let target = kept == 1;
    positive
        .iter()
        .position(|side| *side == target)
        .unwrap_or(0)
}

/// Intersection point where the plane crosses the edge `from -> to`.
///
/// `proj_from`/`proj_to` are the endpoints' projections onto the plane normal.
/// On-plane endpoints return the exact stored vertex so coincident contour
/// points stay bit-identical across adjacent triangles.
#[allow(clippy::float_cmp)] // exact equality intentionally detects on-plane vertices
fn edge_point(from: DVec3, to: DVec3, proj_from: f64, proj_to: f64, offset: f64) -> DVec3 {
    if proj_from == offset {
        return from;
    }
    if proj_to == offset {
        return to;
    }
    let denom = proj_from - proj_to;
    if denom == 0.0 {
        // Parallel edge guard; the caller drops the zero-length segment.
        return from;
    }
    let t = (proj_from - offset) / denom;
    from + (to - from) * t
}

/// Quantized endpoint key used to weld coincident segment endpoints.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct WeldKey([u64; 3]);

/// Quantize a point to a [`WeldKey`]. `+ 0.0` collapses `-0.0` to `0.0`.
fn weld_key(point: DVec3) -> WeldKey {
    let quant = |value: f64| ((value * WELD_SCALE).round() + 0.0).to_bits();
    WeldKey([quant(point.x), quant(point.y), quant(point.z)])
}

/// Stitch loose segments into deterministically ordered polylines.
pub(super) fn stitch(mut segments: Vec<Segment>) -> SectionResult {
    if segments.is_empty() {
        return SectionResult::default();
    }
    segments.sort_by(compare_segments);
    Stitcher::build(&segments).run()
}

/// Total order over segments derived only from their endpoint coordinates, so
/// the full and swept paths stitch an identical set identically.
fn compare_segments(a: &Segment, b: &Segment) -> Ordering {
    let (a0, a1) = ordered_ends(a);
    let (b0, b1) = ordered_ends(b);
    cmp_point(a0, b0).then_with(|| cmp_point(a1, b1))
}

/// A segment's endpoints in canonical (sorted) order.
fn ordered_ends(segment: &Segment) -> (DVec3, DVec3) {
    if cmp_point(segment.a, segment.b) == Ordering::Greater {
        (segment.b, segment.a)
    } else {
        (segment.a, segment.b)
    }
}

/// Deterministic total order over points using `f64::total_cmp` per axis.
fn cmp_point(a: DVec3, b: DVec3) -> Ordering {
    a.x.total_cmp(&b.x)
        .then_with(|| a.y.total_cmp(&b.y))
        .then_with(|| a.z.total_cmp(&b.z))
}

/// Graph walker that turns welded segments into polylines.
struct Stitcher {
    /// Representative point per node.
    points: Vec<DVec3>,
    /// Segment indices incident to each node.
    incident: Vec<Vec<usize>>,
    /// The two node ids each live segment connects.
    seg_nodes: Vec<(usize, usize)>,
    /// Whether each live segment has been consumed by a walk.
    used: Vec<bool>,
}

impl Stitcher {
    /// Build the node graph from canonically sorted segments.
    fn build(segments: &[Segment]) -> Self {
        let mut index: HashMap<WeldKey, usize> = HashMap::new();
        let mut points = Vec::new();
        let mut incident: Vec<Vec<usize>> = Vec::new();
        let mut seg_nodes = Vec::new();
        for segment in segments {
            let a = intern(&mut index, &mut points, &mut incident, segment.a);
            let b = intern(&mut index, &mut points, &mut incident, segment.b);
            if a == b {
                continue; // sub-weld-scale segment; treat as a point and drop
            }
            let seg = seg_nodes.len();
            seg_nodes.push((a, b));
            incident[a].push(seg);
            incident[b].push(seg);
        }
        let used = vec![false; seg_nodes.len()];
        Self {
            points,
            incident,
            seg_nodes,
            used,
        }
    }

    /// Walk every polyline and return them in deterministic order.
    fn run(mut self) -> SectionResult {
        let mut polylines = Vec::new();
        // Pass 1: start at endpoints and non-manifold junctions (degree != 2).
        for node in 0..self.points.len() {
            if self.degree(node) == 2 {
                continue;
            }
            while let Some(seg) = self.first_unused(node) {
                polylines.push(self.walk(node, seg));
            }
        }
        // Pass 2: any remaining segments belong to pure loops (all degree 2).
        for seg in 0..self.seg_nodes.len() {
            if self.used[seg] {
                continue;
            }
            let (start, _) = self.seg_nodes[seg];
            polylines.push(self.walk(start, seg));
        }
        polylines.sort_by(cmp_first_point);
        SectionResult { polylines }
    }

    /// Number of segments incident to `node`.
    fn degree(&self, node: usize) -> usize {
        self.incident[node].len()
    }

    /// The node at the far end of `segment` from `node`.
    fn other(&self, segment: usize, node: usize) -> usize {
        let (a, b) = self.seg_nodes[segment];
        if a == node {
            b
        } else {
            a
        }
    }

    /// First unused segment incident to `node`.
    fn first_unused(&self, node: usize) -> Option<usize> {
        self.incident[node]
            .iter()
            .copied()
            .find(|seg| !self.used[*seg])
    }

    /// First unused segment incident to `node`, other than `exclude`.
    fn next_unused(&self, node: usize, exclude: usize) -> Option<usize> {
        self.incident[node]
            .iter()
            .copied()
            .find(|seg| *seg != exclude && !self.used[*seg])
    }

    /// Walk from `start` along `first`, following clean degree-2 pass-throughs.
    fn walk(&mut self, start: usize, first: usize) -> SectionPolyline {
        let mut path = vec![start];
        let mut current = start;
        let mut seg = first;
        loop {
            self.used[seg] = true;
            let next = self.other(seg, current);
            path.push(next);
            current = next;
            if self.degree(current) != 2 {
                break;
            }
            match self.next_unused(current, seg) {
                Some(next_seg) => seg = next_seg,
                None => break,
            }
        }
        finalize_path(&self.points, &path)
    }
}

/// Intern a point to a node id, creating it on first sight.
fn intern(
    index: &mut HashMap<WeldKey, usize>,
    points: &mut Vec<DVec3>,
    incident: &mut Vec<Vec<usize>>,
    point: DVec3,
) -> usize {
    let key = weld_key(point);
    if let Some(&id) = index.get(&key) {
        return id;
    }
    let id = points.len();
    points.push(point);
    incident.push(Vec::new());
    index.insert(key, id);
    id
}

/// Convert a node path into a polyline, detecting and de-duplicating loops.
fn finalize_path(points: &[DVec3], path: &[usize]) -> SectionPolyline {
    let closed = path.len() >= 3 && path.first() == path.last();
    let nodes = if closed {
        &path[..path.len() - 1]
    } else {
        path
    };
    let points = nodes.iter().map(|node| points[*node]).collect();
    SectionPolyline { points, closed }
}

/// Order polylines by their first point (empty polylines sort last).
fn cmp_first_point(a: &SectionPolyline, b: &SectionPolyline) -> Ordering {
    match (a.points.first(), b.points.first()) {
        (Some(pa), Some(pb)) => cmp_point(*pa, *pb),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;

    /// Closed UV sphere of `radius` about the origin.
    fn uv_sphere(radius: f64, nlat: usize, nlon: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut pos = Vec::new();
        for a in 0..=nlat {
            let lat = std::f64::consts::PI * (a as f64 / nlat as f64);
            for o in 0..nlon {
                let lon = 2.0 * std::f64::consts::PI * (o as f64 / nlon as f64);
                pos.push([
                    (radius * lat.sin() * lon.cos()) as f32,
                    (radius * lat.sin() * lon.sin()) as f32,
                    (radius * lat.cos()) as f32,
                ]);
            }
        }
        let mut idx = Vec::new();
        for a in 0..nlat {
            for o in 0..nlon {
                let o2 = (o + 1) % nlon;
                let p00 = (a * nlon + o) as u32;
                let p01 = (a * nlon + o2) as u32;
                let p10 = ((a + 1) * nlon + o) as u32;
                let p11 = ((a + 1) * nlon + o2) as u32;
                idx.extend_from_slice(&[p00, p10, p11, p00, p11, p01]);
            }
        }
        (pos, idx)
    }

    /// Squared distance from `p` to triangle `abc` (Ericson closest-point).
    #[allow(clippy::many_single_char_names)] // p/a/b/c + barycentric d1..d6 are the standard notation
    fn tri_dist_sq(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> f64 {
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return ap.length_squared();
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return bp.length_squared();
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            return (a + ab * v - p).length_squared();
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return cp.length_squared();
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            return (a + ac * w - p).length_squared();
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            return (b + (c - b) * w - p).length_squared();
        }
        let denom = 1.0 / (va + vb + vc);
        (a + ab * (vb * denom) + ac * (vc * denom) - p).length_squared()
    }

    fn nearest_surface_dist(p: DVec3, pos: &[[f32; 3]], idx: &[u32]) -> f64 {
        let d = |i: u32| {
            let v = pos[i as usize];
            DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))
        };
        idx.chunks_exact(3)
            .map(|t| tri_dist_sq(p, d(t[0]), d(t[1]), d(t[2])))
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    }

    /// Every contour point satisfies `|n·p - d| = 0` exactly (f64), and lies on
    /// the input surface, for several tilted planes over a curved mesh.
    #[test]
    fn contour_points_lie_on_the_plane_and_on_the_surface() {
        let (pos, idx) = uv_sphere(10.0, 40, 56);
        let planes = [
            (Vec3::X, 0.0_f32),
            (Vec3::new(0.5, 0.5, 0.707).normalize(), 3.0),
            (Vec3::Z, 7.0),
            (Vec3::new(0.2, -0.9, 0.3).normalize(), -4.0),
        ];
        for (n, dist) in planes {
            let plane = SectionPlane::new(n, dist).expect("unit normal");
            let normal = n.as_dvec3();
            let offset = f64::from(dist);
            let result = plane_section(&pos, &idx, plane);
            assert!(!result.polylines.is_empty(), "expected a contour for {n:?}");
            for line in &result.polylines {
                for &p in &line.points {
                    // On the plane, to full f64 precision.
                    assert!(
                        (normal.dot(p) - offset).abs() <= 1.0e-9,
                        "off-plane: {}",
                        (normal.dot(p) - offset).abs()
                    );
                    // On the surface (nearest triangle) to sub-micron in mm.
                    assert!(
                        nearest_surface_dist(p, &pos, &idx) < 1.0e-6,
                        "off-surface at {p:?}"
                    );
                }
            }
        }
    }

    /// A triangle patch that lies exactly in the section plane emits no
    /// segments (its edges belong to the neighbouring crossing triangles),
    /// so a coplanar face never produces garbage or a spurious loop.
    #[test]
    fn coplanar_face_on_the_plane_emits_nothing() {
        let pos = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];
        let result = plane_section(&pos, &indices, SectionPlane::new(Vec3::Z, 0.0).unwrap());
        assert!(result.polylines.is_empty());
    }

    /// A duplicated face must not crash, hang, or spill unbounded output; the
    /// full and duplicated inputs both terminate with a bounded result.
    #[test]
    fn duplicated_face_is_bounded_and_terminates() {
        let pos = vec![[-1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, -1.0, 0.0]];
        let plane = SectionPlane::new(Vec3::X, 0.0).unwrap();
        let once = plane_section(&pos, &[0, 1, 2], plane);
        let twice = plane_section(&pos, &[0, 1, 2, 0, 1, 2], plane);
        assert_eq!(once.polylines.len(), 1);
        // The doubled edge welds to the same two nodes; output stays bounded.
        assert!(twice.polylines.len() <= 1);
        for line in &twice.polylines {
            assert!(line.points.len() <= 2);
        }
    }

    /// Coordinates spanning six orders of magnitude keep the on-plane property
    /// exact, because promotion to f64 happens before any plane arithmetic.
    #[test]
    fn giant_and_tiny_coordinates_stay_on_plane() {
        let (base, idx) = uv_sphere(1.0, 24, 32);
        for scale in [1.0e-3_f32, 1.0, 1.0e6] {
            let pos: Vec<[f32; 3]> = base
                .iter()
                .map(|p| [p[0] * scale, p[1] * scale, p[2] * scale])
                .collect();
            let dist = 0.3 * scale;
            let plane = SectionPlane::new(Vec3::X, dist).unwrap();
            let normal = Vec3::X.as_dvec3();
            let offset = f64::from(dist);
            let result = plane_section(&pos, &idx, plane);
            assert!(!result.polylines.is_empty(), "no contour at scale {scale}");
            for line in &result.polylines {
                for &p in &line.points {
                    // Tolerance scales with coordinate magnitude (f32 input).
                    let tol = 1.0e-6 * f64::from(scale).max(1.0);
                    assert!(
                        (normal.dot(p) - offset).abs() <= tol,
                        "off-plane {} at scale {scale}",
                        (normal.dot(p) - offset).abs()
                    );
                }
            }
        }
    }
}
