//! Nearest-surface queries over a triangle soup.
//!
//! A uniform grid whose cell size follows the mesh's own triangle size, so one
//! query costs the same on a small denture and on a full arch. A fixed
//! millimetre cell — the shape this replaces — either wastes memory on fine
//! meshes or drops a hundred triangles into one bucket on coarse ones.
//!
//! Normals are computed from triangle geometry and never read from the file:
//! the sign of the whole deviation map hangs on them, and scanner exports
//! routinely carry normals that disagree with their own winding.

use glam::DVec3;

use crate::Soup;

/// Triangles whose doubled area falls below this are dropped at build time:
/// they have no usable normal and no interior to project onto.
const MIN_DOUBLE_AREA: f64 = 1e-12;

/// Cell size as a multiple of the mean longest triangle edge. Two keeps a
/// typical triangle inside a small constant number of cells while leaving
/// buckets short.
const CELL_EDGE_FACTOR: f64 = 2.0;

/// Upper bound on total grid cells. A very thin or very large mesh would
/// otherwise allocate an absurd grid; the cell grows until the count fits.
const MAX_CELLS: usize = 4_000_000;

/// The nearest point found on a surface, with the geometry that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    /// The closest point on the surface, in the surface's own frame.
    pub point: DVec3,
    /// Unit geometric normal of the triangle carrying `point`.
    pub normal: DVec3,
    /// Index of that triangle within the source soup.
    pub triangle: u32,
}

/// A spatial index answering "what is the closest surface point to this?".
#[derive(Clone, Debug)]
pub struct SurfaceIndex {
    corners: Vec<[DVec3; 3]>,
    normals: Vec<DVec3>,
    sources: Vec<u32>,
    min: DVec3,
    cell: f64,
    dims: [i64; 3],
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl SurfaceIndex {
    /// Build an index over every usable triangle in `soup`.
    ///
    /// Triangles with out-of-range indices, non-finite vertices, or no usable
    /// area are dropped rather than poisoning a query. Returns `None` when
    /// nothing usable survives.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn build(soup: Soup<'_>) -> Option<Self> {
        let vertex_count = soup.vertex_count();
        let mut corners: Vec<[DVec3; 3]> = Vec::new();
        let mut normals: Vec<DVec3> = Vec::new();
        let mut sources: Vec<u32> = Vec::new();
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let mut edge_total = 0.0f64;

        for (triangle, slice) in soup.indices.chunks_exact(3).enumerate() {
            let Some(vertices) = read_triangle(soup.positions, vertex_count, slice) else {
                continue;
            };
            let normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
            let length = normal.length();
            if !length.is_finite() || length < MIN_DOUBLE_AREA {
                continue;
            }
            for corner in vertices {
                min = min.min(corner);
                max = max.max(corner);
            }
            edge_total += longest_edge(&vertices);
            corners.push(vertices);
            normals.push(normal / length);
            sources.push(u32::try_from(triangle).unwrap_or(u32::MAX));
        }

        if corners.is_empty() {
            return None;
        }

        let extent = max - min;
        let diagonal = extent.length().max(1e-3);
        let mean_edge = edge_total / corners.len() as f64;
        let mut cell = (mean_edge * CELL_EDGE_FACTOR).clamp(1e-3, diagonal);
        let mut dims = grid_dims(extent, cell);
        while cell_count(dims) > MAX_CELLS {
            cell *= 2.0;
            dims = grid_dims(extent, cell);
        }

        let index = Self {
            corners,
            normals,
            sources,
            min,
            cell,
            dims,
            starts: Vec::new(),
            items: Vec::new(),
        };
        Some(index.with_buckets())
    }

    /// The grid's cell size in millimetres — exposed so callers can reason
    /// about query cost and so the adaptive rule stays testable.
    #[must_use]
    pub fn cell_size(&self) -> f64 {
        self.cell
    }

    /// Number of triangles the index actually kept.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.corners.len()
    }

    /// The closest surface point to `point` within `radius`, or `None` when
    /// nothing is in reach.
    ///
    /// Deterministic: cells are visited in a fixed order and ties break on the
    /// lower source triangle index, so the answer does not depend on traversal
    /// order or on how the caller parallelizes its queries.
    #[must_use]
    pub fn nearest(&self, point: DVec3, radius: f64) -> Option<SurfaceHit> {
        if !point.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        let reach = DVec3::splat(radius);
        let low = self.cell_of(point - reach);
        let high = self.cell_of(point + reach);
        let mut best: Option<(f64, u32, DVec3, usize)> = None;
        let limit = radius * radius;

        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let cell_floor = self.cell_distance_squared(point, [x, y, z]);
                    let ceiling = best.map_or(limit, |(distance, _, _, _)| distance);
                    if cell_floor > ceiling {
                        continue;
                    }
                    let Some(bucket) = self.bucket([x, y, z]) else {
                        continue;
                    };
                    for &slot in bucket {
                        let slot = slot as usize;
                        let Some(corners) = self.corners.get(slot) else {
                            continue;
                        };
                        let candidate =
                            closest_point_on_triangle(point, corners[0], corners[1], corners[2]);
                        let distance = (candidate - point).length_squared();
                        if distance > limit {
                            continue;
                        }
                        let source = self.sources.get(slot).copied().unwrap_or(u32::MAX);
                        // The exact equality is deliberate: an exact tie is
                        // what a shared edge or a duplicated facet produces,
                        // and breaking it on the lower source index is what
                        // makes the answer independent of traversal order.
                        #[allow(clippy::float_cmp)]
                        let better = match best {
                            None => true,
                            Some((best_distance, best_source, _, _)) => {
                                distance < best_distance
                                    || (distance == best_distance && source < best_source)
                            }
                        };
                        if better {
                            best = Some((distance, source, candidate, slot));
                        }
                    }
                }
            }
        }

        best.map(|(_, triangle, point, slot)| SurfaceHit {
            point,
            normal: self.normals.get(slot).copied().unwrap_or(DVec3::Z),
            triangle,
        })
    }

    /// Bucket every triangle into the cells its bounding box overlaps, as a
    /// counted-then-scattered CSR list: no per-cell `Vec`, no rehashing, and a
    /// layout that is identical for identical input.
    #[allow(clippy::cast_sign_loss)]
    fn with_buckets(mut self) -> Self {
        let cells = cell_count(self.dims);
        let mut counts = vec![0u32; cells + 1];
        for corners in &self.corners {
            self.for_each_cell(corners, |cell| {
                counts[cell + 1] = counts[cell + 1].saturating_add(1);
            });
        }
        for slot in 1..counts.len() {
            counts[slot] += counts[slot - 1];
        }
        let total = counts.last().copied().unwrap_or(0) as usize;
        let mut items = vec![0u32; total];
        let mut cursor = counts.clone();
        for (triangle, corners) in self.corners.iter().enumerate() {
            let triangle = u32::try_from(triangle).unwrap_or(u32::MAX);
            self.for_each_cell(corners, |cell| {
                let slot = cursor[cell] as usize;
                if let Some(entry) = items.get_mut(slot) {
                    *entry = triangle;
                }
                cursor[cell] += 1;
            });
        }
        self.starts = counts;
        self.items = items;
        self
    }

    /// Call `visit` once per grid cell overlapped by this triangle's box.
    fn for_each_cell(&self, corners: &[DVec3; 3], mut visit: impl FnMut(usize)) {
        let low = self.cell_of(corners[0].min(corners[1]).min(corners[2]));
        let high = self.cell_of(corners[0].max(corners[1]).max(corners[2]));
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    if let Some(cell) = self.cell_index([x, y, z]) {
                        visit(cell);
                    }
                }
            }
        }
    }

    /// Grid coordinates of `point`, clamped into the grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn cell_of(&self, point: DVec3) -> [i64; 3] {
        let local = (point - self.min) / self.cell;
        let mut out = [0i64; 3];
        for ((slot, raw), dim) in out.iter_mut().zip(local.to_array()).zip(self.dims) {
            let value = if raw.is_finite() { raw.floor() } else { 0.0 };
            *slot = value.clamp(0.0, (dim - 1) as f64) as i64;
        }
        out
    }

    /// Flat index of a grid coordinate, or `None` when it falls outside.
    fn cell_index(&self, cell: [i64; 3]) -> Option<usize> {
        if cell
            .iter()
            .zip(self.dims)
            .any(|(&value, dim)| value < 0 || value >= dim)
        {
            return None;
        }
        let flat = (cell[2] * self.dims[1] + cell[1]) * self.dims[0] + cell[0];
        usize::try_from(flat).ok()
    }

    /// The triangles bucketed into one cell.
    fn bucket(&self, cell: [i64; 3]) -> Option<&[u32]> {
        let index = self.cell_index(cell)?;
        let start = *self.starts.get(index)? as usize;
        let end = *self.starts.get(index + 1)? as usize;
        self.items.get(start..end)
    }

    /// Squared distance from `point` to a cell's own box — the cheap test that
    /// lets a query skip a bucket without touching its triangles.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn cell_distance_squared(&self, point: DVec3, cell: [i64; 3]) -> f64 {
        let low = self.min + DVec3::new(cell[0] as f64, cell[1] as f64, cell[2] as f64) * self.cell;
        let high = low + DVec3::splat(self.cell);
        (point.clamp(low, high) - point).length_squared()
    }
}

/// Read one triangle's vertices, rejecting out-of-range or non-finite input.
fn read_triangle(positions: &[f32], vertex_count: usize, slice: &[u32]) -> Option<[DVec3; 3]> {
    let mut out = [DVec3::ZERO; 3];
    for (slot, &raw) in slice.iter().enumerate() {
        let vertex = usize::try_from(raw).ok()?;
        if vertex >= vertex_count {
            return None;
        }
        let xyz = positions.get(vertex * 3..vertex * 3 + 3)?;
        let point = DVec3::new(f64::from(xyz[0]), f64::from(xyz[1]), f64::from(xyz[2]));
        if !point.is_finite() {
            return None;
        }
        out[slot] = point;
    }
    Some(out)
}

/// Length of a triangle's longest edge.
fn longest_edge(corners: &[DVec3; 3]) -> f64 {
    let a = (corners[1] - corners[0]).length();
    let b = (corners[2] - corners[1]).length();
    let c = (corners[0] - corners[2]).length();
    a.max(b).max(c)
}

/// Grid dimensions covering `extent` at `cell`, at least one cell per axis.
#[allow(clippy::cast_possible_truncation)]
fn grid_dims(extent: DVec3, cell: f64) -> [i64; 3] {
    let mut dims = [1i64; 3];
    for (dim, raw) in dims.iter_mut().zip(extent.to_array()) {
        let span = if raw.is_finite() { raw.max(0.0) } else { 0.0 };
        *dim = ((span / cell).floor() as i64 + 1).max(1);
    }
    dims
}

/// Total cell count, saturating instead of overflowing on absurd dimensions.
fn cell_count(dims: [i64; 3]) -> usize {
    let product = dims[0]
        .saturating_mul(dims[1])
        .saturating_mul(dims[2])
        .max(1);
    usize::try_from(product).unwrap_or(usize::MAX)
}

/// Closest point on a triangle to `point` — Ericson's region test, which
/// handles the face, the three edges, and the three corners without branching
/// on a projection that may fall outside.
fn closest_point_on_triangle(point: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let denominator = d1 - d3;
        if denominator.abs() > f64::EPSILON {
            return a + ab * (d1 / denominator);
        }
        return a;
    }
    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let denominator = d2 - d6;
        if denominator.abs() > f64::EPSILON {
            return a + ac * (d2 / denominator);
        }
        return a;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let denominator = (d4 - d3) + (d5 - d6);
        if denominator.abs() > f64::EPSILON {
            return b + (c - b) * ((d4 - d3) / denominator);
        }
        return b;
    }
    let total = va + vb + vc;
    if total.abs() <= f64::EPSILON {
        return a;
    }
    a + ab * (vb / total) + ac * (vc / total)
}

#[cfg(test)]
mod tests {
    use super::SurfaceIndex;
    use crate::Soup;
    use glam::DVec3;

    /// A flat `n` x `n` grid of quads on z = 0, spacing `step`, as a soup.
    fn plane(n: usize, step: f64) -> (Vec<f32>, Vec<u32>) {
        let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
        for j in 0..=n {
            for i in 0..=n {
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                {
                    positions.push((i as f64 * step) as f32);
                    positions.push((j as f64 * step) as f32);
                    positions.push(0.0);
                }
            }
        }
        let mut indices = Vec::with_capacity(n * n * 6);
        let stride = u32::try_from(n + 1).unwrap();
        let span = u32::try_from(n).unwrap();
        for j in 0..span {
            for i in 0..span {
                let a = j * stride + i;
                indices.extend_from_slice(&[a, a + 1, a + stride]);
                indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
            }
        }
        (positions, indices)
    }

    fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
        Soup {
            positions,
            indices,
            mask: None,
        }
    }

    #[test]
    fn nearest_on_a_plane_is_the_foot_of_the_perpendicular() {
        let (positions, indices) = plane(8, 1.0);
        let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        let hit = index.nearest(DVec3::new(3.3, 4.7, 2.5), 10.0).unwrap();
        assert!((hit.point.x - 3.3).abs() < 1e-6);
        assert!((hit.point.y - 4.7).abs() < 1e-6);
        assert!(hit.point.z.abs() < 1e-6);
    }

    #[test]
    fn the_normal_is_the_geometric_plane_normal() {
        let (positions, indices) = plane(4, 1.0);
        let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        let hit = index.nearest(DVec3::new(1.5, 1.5, 3.0), 10.0).unwrap();
        assert!((hit.normal.dot(DVec3::Z).abs() - 1.0).abs() < 1e-9);
        assert!((hit.normal.length() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_is_returned_beyond_the_radius() {
        let (positions, indices) = plane(4, 1.0);
        let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        assert!(index.nearest(DVec3::new(2.0, 2.0, 50.0), 1.0).is_none());
    }

    #[test]
    fn a_query_beside_the_sheet_snaps_to_its_edge() {
        let (positions, indices) = plane(4, 1.0);
        let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        let hit = index.nearest(DVec3::new(-3.0, 2.0, 0.0), 10.0).unwrap();
        assert!(
            hit.point.x.abs() < 1e-6,
            "expected the x = 0 border, got {hit:?}"
        );
    }

    #[test]
    fn build_refuses_empty_and_degenerate_input() {
        assert!(SurfaceIndex::build(soup(&[], &[])).is_none());
        let positions = [0.0; 9];
        let indices = [0, 1, 2];
        assert!(SurfaceIndex::build(soup(&positions, &indices)).is_none());
    }

    #[test]
    fn build_ignores_out_of_range_and_non_finite_triangles() {
        let (mut positions, mut indices) = plane(4, 1.0);
        indices.extend_from_slice(&[9999, 10000, 10001]);
        let base = u32::try_from(positions.len() / 3).unwrap();
        positions.extend_from_slice(&[f32::NAN, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        indices.extend_from_slice(&[base, base + 1, base + 2]);
        let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        assert!(index.nearest(DVec3::new(1.5, 1.5, 1.0), 5.0).is_some());
    }

    #[test]
    fn the_cell_size_follows_triangle_size() {
        let (coarse_positions, coarse_indices) = plane(4, 4.0);
        let (fine_positions, fine_indices) = plane(32, 0.25);
        let coarse = SurfaceIndex::build(soup(&coarse_positions, &coarse_indices)).unwrap();
        let fine = SurfaceIndex::build(soup(&fine_positions, &fine_indices)).unwrap();
        assert!(
            coarse.cell_size() > fine.cell_size() * 4.0,
            "coarse {} vs fine {}",
            coarse.cell_size(),
            fine.cell_size()
        );
        assert!(coarse.nearest(DVec3::new(6.0, 6.0, 1.0), 10.0).is_some());
        assert!(fine.nearest(DVec3::new(4.0, 4.0, 1.0), 10.0).is_some());
    }

    #[test]
    fn repeated_builds_answer_identically() {
        let (positions, indices) = plane(16, 0.5);
        let first = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        let second = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
        for k in 0..50 {
            let q = DVec3::new(f64::from(k) * 0.17, f64::from(k) * 0.09, 0.4);
            let a = first.nearest(q, 5.0);
            let b = second.nearest(q, 5.0);
            assert_eq!(a.map(|hit| hit.triangle), b.map(|hit| hit.triangle));
            assert_eq!(
                a.map(|hit| hit.point.to_array()),
                b.map(|hit| hit.point.to_array())
            );
        }
    }
}
