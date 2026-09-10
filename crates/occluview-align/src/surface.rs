//! Nearest-surface queries over a triangle soup.
//!
//! The adaptive grid follows triangle size and uses coarse occupancy data to
//! skip empty space. Queries remain equivalent to a full triangle scan,
//! including tie-breaking. Normals come from triangle winding rather than
//! imported vertex data so deviation signs use the indexed geometry.

use std::collections::BTreeMap;
use std::ops::Range;

use glam::DVec3;

use crate::Soup;

#[path = "surface_geometry.rs"]
mod surface_geometry;
pub(super) use surface_geometry::closest_point_on_triangle;
#[path = "surface_helpers.rs"]
mod surface_helpers;
use surface_helpers::{canonical_bits, cell_count, grid_dims, longest_edge, read_triangle};

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

/// Fine cells per coarse block along each axis. The coarse level exists only to
/// tell a query how much empty space surrounds it, so four is plenty: it costs
/// a sixty-fourth of the grid in bytes and still resolves emptiness to a couple
/// of millimetres on a dental scan.
const BLOCK: i64 = 4;

/// The half of the 3x3x3 neighbourhood a forward raster sweep has already
/// visited, ending with the cell before this one on the same row.
const EARLIER: [[i64; 3]; 13] = [
    [-1, -1, -1],
    [0, -1, -1],
    [1, -1, -1],
    [-1, 0, -1],
    [0, 0, -1],
    [1, 0, -1],
    [-1, 1, -1],
    [0, 1, -1],
    [1, 1, -1],
    [-1, -1, 0],
    [0, -1, 0],
    [1, -1, 0],
    [-1, 0, 0],
];

/// The running answer during one query.
///
/// Kept squared: the comparison and the tie-break both work on squared
/// distances, so a query never pays for a square root it does not report.
#[derive(Clone, Copy)]
struct Candidate {
    distance: f64,
    source: u32,
    point: DVec3,
    slot: usize,
}

/// One query's fixed terms: the point, the squared radius, and the cell window
/// the radius allows. Bundled so the traversal helpers keep short signatures.
struct Query {
    point: DVec3,
    limit: f64,
    home: [i64; 3],
    low: [i64; 3],
    high: [i64; 3],
}

impl Query {
    /// The largest shell that can still hold a cell inside the window.
    fn rings(&self) -> i64 {
        let mut rings = 0;
        for axis in 0..3 {
            rings = rings
                .max(self.home[axis] - self.low[axis])
                .max(self.high[axis] - self.home[axis]);
        }
        rings
    }
}

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

/// A deterministic representative of one indexed triangle for bounded
/// fixed-to-moving overlap checks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceSample {
    /// Triangle centroid in the index's own frame.
    pub(crate) point: DVec3,
    /// The triangle's geometric normal.
    pub(crate) normal: DVec3,
}

/// A spatial index answering "what is the closest surface point to this?".
#[derive(Clone, Debug)]
pub struct SurfaceIndex {
    corners: Vec<[DVec3; 3]>,
    normals: Vec<DVec3>,
    sources: Vec<u32>,
    min: DVec3,
    max: DVec3,
    cell: f64,
    dims: [i64; 3],
    starts: Vec<u32>,
    items: Vec<u32>,
    blocks: [i64; 3],
    gaps: Vec<u8>,
    components: Vec<(DVec3, DVec3)>,
}

impl SurfaceIndex {
    /// Build an index over every usable triangle in `soup`.
    ///
    /// Triangles with invalid indices, non-finite vertices, degenerate area, or
    /// excluded corners are omitted from the index. Excluded fixed geometry
    /// cannot participate in correspondence or deviation measurements.
    ///
    /// Returns `None` when nothing usable survives.
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
        let mut parent: Vec<usize> = (0..vertex_count).collect();
        let mut welded_positions: BTreeMap<[u64; 3], usize> = BTreeMap::new();
        // One vertex anchor per retained triangle is enough to recover its
        // component after all unions have been completed. Keeping all three
        // ids here needlessly triples temporary memory on a dense scan.
        let mut triangle_anchors = Vec::new();

        for (triangle, slice) in soup.indices.as_chunks::<3>().0.iter().enumerate() {
            // Any masked corner takes the whole triangle out. A triangle with
            // one corner inside a painted region straddles the boundary, and
            // half a triangle is not a surface a query can land on.
            if slice.iter().any(|index| soup.is_excluded(*index as usize)) {
                continue;
            }
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
            let [a, b, c] = *slice;
            let (Ok(a), Ok(b), Ok(c)) =
                (usize::try_from(a), usize::try_from(b), usize::try_from(c))
            else {
                continue;
            };
            // STL and a few preview loaders duplicate every facet corner. The
            // index still keeps those corners separate for exact nearest-hit
            // behaviour, but component discovery must weld equal positions or
            // every triangle becomes a false one-triangle component.
            for (vertex, point) in [a, b, c].into_iter().zip(vertices) {
                let key = [
                    canonical_bits(point.x),
                    canonical_bits(point.y),
                    canonical_bits(point.z),
                ];
                if let Some(&other) = welded_positions.get(&key) {
                    union(&mut parent, other, vertex);
                } else {
                    welded_positions.insert(key, vertex);
                }
            }
            union(&mut parent, a, b);
            union(&mut parent, b, c);
            triangle_anchors.push(a);
            edge_total += longest_edge(&vertices);
            corners.push(vertices);
            normals.push(normal / length);
            sources.push(u32::try_from(triangle).unwrap_or(u32::MAX));
        }

        if corners.is_empty() {
            return None;
        }

        let mut component_bounds: BTreeMap<usize, (DVec3, DVec3)> = BTreeMap::new();
        for (vertices, &anchor) in corners.iter().zip(&triangle_anchors) {
            let root = find(&mut parent, anchor);
            let triangle_min = vertices[0].min(vertices[1]).min(vertices[2]);
            let triangle_max = vertices[0].max(vertices[1]).max(vertices[2]);
            component_bounds
                .entry(root)
                .and_modify(|(low, high)| {
                    *low = low.min(triangle_min);
                    *high = high.max(triangle_max);
                })
                .or_insert((triangle_min, triangle_max));
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
            max,
            cell,
            dims,
            starts: Vec::new(),
            items: Vec::new(),
            blocks: [1; 3],
            gaps: Vec::new(),
            components: component_bounds.into_values().collect(),
        };
        Some(index.in_cell_order().with_buckets().with_gaps())
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

    /// The axis-aligned bounds of the indexed surface in its own frame.
    #[must_use]
    pub fn bounds(&self) -> (DVec3, DVec3) {
        (self.min, self.max)
    }

    /// Bounds for each connected component, in deterministic source order.
    ///
    /// Alignment uses these as bounded coarse hypotheses. A disconnected fixed
    /// scan (for example an arch with separate teeth) must not have its global
    /// bounding-box centre pull a moving tooth onto an adjacent component.
    #[must_use]
    pub fn component_bounds(&self) -> &[(DVec3, DVec3)] {
        &self.components
    }

    /// Return at most `budget` deterministic triangle representatives.
    ///
    /// The samples are used only as an independent overlap signal; nearest
    /// queries and deviation maps still use the complete indexed surface.
    #[must_use]
    pub(crate) fn representative_samples(&self, budget: usize) -> Vec<SurfaceSample> {
        if budget == 0 || self.corners.is_empty() {
            return Vec::new();
        }
        let stride = self.corners.len().div_ceil(budget).max(1);
        self.corners
            .iter()
            .zip(&self.normals)
            .step_by(stride)
            .map(|(corners, &normal)| SurfaceSample {
                point: (corners[0] + corners[1] + corners[2]) / 3.0,
                normal,
            })
            .collect()
    }

    /// The closest surface point to `point` within `radius`, or `None` when
    /// nothing is in reach.
    ///
    /// Cells are visited as shells expanding out of the one holding `point`,
    /// and the walk stops as soon as the next shell cannot possibly beat what
    /// has been found. For a query near a surface — which is every query this
    /// crate makes — the answer sits in the first or second shell, so the cost
    /// follows the distance to the surface rather than the influence radius.
    ///
    /// Deterministic: cells are visited in a fixed order and ties break on the
    /// lower source triangle index, so the answer does not depend on traversal
    /// order or on how the caller parallelizes its queries.
    #[must_use]
    pub fn nearest(&self, point: DVec3, radius: f64) -> Option<SurfaceHit> {
        if !point.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        // Every triangle lies inside the mesh box, so a point farther from that
        // box than the radius cannot reach any of them. One clamp answers the
        // whole query for a vertex sitting off the end of the other scan.
        if (point.clamp(self.min, self.max) - point).length_squared() > radius * radius {
            return None;
        }
        let reach = DVec3::splat(radius);
        let query = Query {
            point,
            limit: radius * radius,
            home: self.cell_of(point),
            low: self.cell_of(point - reach),
            high: self.cell_of(point + reach),
        };
        let mut best: Option<Candidate> = None;

        // Shells the coarse level already proved empty are not walked at all.
        // For a point sitting in open space this is the whole answer: the walk
        // starts past the influence radius and never begins.
        for ring in self.first_ring(query.home)..=query.rings() {
            // A shell is skipped only when its own floor already exceeds the
            // best distance so far — never when it merely equals it, because an
            // equal distance is a tie that may still carry a lower source
            // index. That is the same test the per-cell prune makes, so the
            // answer is the one a full sweep of the window would give.
            let ceiling = best.map_or(query.limit, |found| found.distance);
            if self.ring_floor(&query, ring) > ceiling {
                break;
            }
            self.visit_ring(&query, ring, &mut best);
        }

        best.map(|found| SurfaceHit {
            point: found.point,
            normal: self.normals.get(found.slot).copied().unwrap_or(DVec3::Z),
            triangle: found.source,
        })
    }

    /// Squared distance from the query point to the nearest cell of shell
    /// `ring`, or infinity when that shell holds no cell inside the window.
    ///
    /// A cell in shell `ring` sits beyond one of six planes, so the bound is
    /// the closest of those planes. Directions whose plane has already left the
    /// window are not counted: that keeps the bound tight for a point sitting
    /// off the end of the grid, where the window collapses to a slab.
    #[allow(clippy::cast_precision_loss)]
    fn ring_floor(&self, query: &Query, ring: i64) -> f64 {
        let point = query.point.to_array();
        let origin = self.min.to_array();
        let mut gap = f64::INFINITY;
        for axis in 0..3 {
            let home = query.home[axis];
            if home + ring <= query.high[axis] {
                let plane = origin[axis] + (home + ring) as f64 * self.cell;
                gap = gap.min((plane - point[axis]).max(0.0));
            }
            if home - ring >= query.low[axis] {
                let plane = origin[axis] + (home - ring + 1) as f64 * self.cell;
                gap = gap.min((point[axis] - plane).max(0.0));
            }
        }
        if gap.is_finite() {
            gap * gap
        } else {
            f64::INFINITY
        }
    }

    /// Visit every cell of shell `ring` that lies inside the query window.
    ///
    /// A shell is the surface of a cube: rows on the near and far z planes are
    /// full rectangles, and the rows between them contribute only their two
    /// end columns. `ring` zero is the single home cell, which the `z_edge`
    /// branch covers.
    fn visit_ring(&self, query: &Query, ring: i64, best: &mut Option<Candidate>) {
        let (home, low, high) = (query.home, query.low, query.high);
        let span = |axis: usize| {
            (home[axis].saturating_sub(ring).max(low[axis]))
                ..=(home[axis].saturating_add(ring).min(high[axis]))
        };
        let columns = span(0);
        let run = columns.end() - columns.start() + 1;
        for z in span(2) {
            let z_edge = (z - home[2]).abs() == ring;
            for y in span(1) {
                if z_edge || (y - home[1]).abs() == ring {
                    self.visit_run(query, [*columns.start(), y, z], run, best);
                } else {
                    for x in [home[0] - ring, home[0] + ring] {
                        if x >= low[0] && x <= high[0] {
                            self.visit_run(query, [x, y, z], 1, best);
                        }
                    }
                }
            }
        }
    }

    /// Test a run of `length` cells along x, starting at `start`.
    ///
    /// Walk the bucket table directly so empty cells require only adjacent
    /// reads.
    fn visit_run(&self, query: &Query, start: [i64; 3], length: i64, best: &mut Option<Candidate>) {
        let Some(base) = self.cell_index(start) else {
            return;
        };
        for offset in 0..length {
            let Some(cell) = usize::try_from(offset).ok().map(|step| base + step) else {
                continue;
            };
            let (Some(&from), Some(&to)) = (self.starts.get(cell), self.starts.get(cell + 1))
            else {
                continue;
            };
            if from == to {
                continue;
            }
            self.visit_cell(
                query,
                [start[0] + offset, start[1], start[2]],
                from..to,
                best,
            );
        }
    }

    /// Test one cell's triangles against the running best.
    fn visit_cell(
        &self,
        query: &Query,
        cell: [i64; 3],
        bucket: Range<u32>,
        best: &mut Option<Candidate>,
    ) {
        let ceiling = best.map_or(query.limit, |found| found.distance);
        if self.cell_distance_squared(query.point, cell) > ceiling {
            return;
        }
        let Some(bucket) = self.items.get(bucket.start as usize..bucket.end as usize) else {
            return;
        };
        for &slot in bucket {
            let slot = slot as usize;
            let Some(corners) = self.corners.get(slot) else {
                continue;
            };
            let candidate =
                closest_point_on_triangle(query.point, corners[0], corners[1], corners[2]);
            let distance = (candidate - query.point).length_squared();
            if distance > query.limit {
                continue;
            }
            let source = self.sources.get(slot).copied().unwrap_or(u32::MAX);
            // The exact equality is deliberate: an exact tie is what a shared
            // edge or a duplicated facet produces, and breaking it on the lower
            // source index is what makes the answer independent of traversal
            // order.
            #[allow(clippy::float_cmp)]
            let better = match best {
                None => true,
                Some(found) => {
                    distance < found.distance
                        || (distance == found.distance && source < found.source)
                }
            };
            if better {
                *best = Some(Candidate {
                    distance,
                    source,
                    point: candidate,
                    slot,
                });
            }
        }
    }

    /// Reorder the triangle arrays so triangles sharing a cell sit together in
    /// memory. A query reads every triangle of a handful of neighbouring cells;
    /// in file order those reads are scattered over tens of megabytes, and the
    /// walk spends its time waiting for memory rather than testing triangles.
    ///
    /// This moves triangles, never renames them: a hit still reports the source
    /// index, and the tie-break still compares source indices, so the order the
    /// arrays happen to be in cannot change an answer.
    ///
    /// Counted and scattered rather than sorted, for the same reason the
    /// buckets are: it is linear, it allocates once, and it lays out identically
    /// for identical input.
    fn in_cell_order(mut self) -> Self {
        let mut counts = vec![0u32; cell_count(self.dims) + 1];
        let home: Vec<u32> = self
            .corners
            .iter()
            .map(|corners| {
                let low = corners[0].min(corners[1]).min(corners[2]);
                let cell = self.cell_index(self.cell_of(low)).unwrap_or(0);
                u32::try_from(cell).unwrap_or(0)
            })
            .collect();
        for &cell in &home {
            if let Some(entry) = counts.get_mut(cell as usize + 1) {
                *entry = entry.saturating_add(1);
            }
        }
        for slot in 1..counts.len() {
            counts[slot] += counts[slot - 1];
        }
        let mut order = vec![0u32; self.corners.len()];
        for (triangle, &cell) in home.iter().enumerate() {
            let Some(cursor) = counts.get_mut(cell as usize) else {
                continue;
            };
            let slot = *cursor as usize;
            *cursor += 1;
            if let Some(entry) = order.get_mut(slot) {
                *entry = u32::try_from(triangle).unwrap_or(0);
            }
        }
        self.corners = gather(&self.corners, &order);
        self.normals = gather(&self.normals, &order);
        self.sources = gather(&self.sources, &order);
        self
    }

    /// Bucket triangles into a deterministic counted-and-scattered CSR list.
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
        flat_index(self.dims, cell)
    }

    /// The first shell that can hold anything, given what the coarse level
    /// knows about the empty space around `home`.
    ///
    /// The recorded gap is a count of blocks, so the shells it clears are the
    /// ones inside the block box it covers. Every fine cell nearer than that is
    /// empty, which is why skipping them changes no answer.
    fn first_ring(&self, home: [i64; 3]) -> i64 {
        let block = [home[0] / BLOCK, home[1] / BLOCK, home[2] / BLOCK];
        let Some(gap) = flat_index(self.blocks, block).and_then(|flat| self.gaps.get(flat)) else {
            return 0;
        };
        if *gap == 0 {
            return 0;
        }
        let reach = i64::from(*gap) - 1;
        let mut clear = i64::MAX;
        for axis in 0..3 {
            let low = (block[axis] - reach) * BLOCK;
            let high = (block[axis] + reach) * BLOCK + BLOCK - 1;
            clear = clear.min(home[axis] - low).min(high - home[axis]);
        }
        clear.saturating_add(1).max(0)
    }

    /// Record, per coarse block, how many blocks away the nearest occupied one
    /// is. This is what lets a query in open space skip straight past the void
    /// it sits in instead of sweeping every cell of it.
    fn with_gaps(mut self) -> Self {
        let blocks = [
            block_count(self.dims[0]),
            block_count(self.dims[1]),
            block_count(self.dims[2]),
        ];
        let mut gaps = vec![u8::MAX; cell_count(blocks)];
        let mut cell = 0usize;
        for z in 0..self.dims[2] {
            for y in 0..self.dims[1] {
                for x in 0..self.dims[0] {
                    let occupied = self.starts.get(cell) != self.starts.get(cell + 1);
                    cell += 1;
                    if !occupied {
                        continue;
                    }
                    if let Some(entry) = flat_index(blocks, [x / BLOCK, y / BLOCK, z / BLOCK])
                        .and_then(|flat| gaps.get_mut(flat))
                    {
                        *entry = 0;
                    }
                }
            }
        }
        sweep(blocks, &mut gaps, true);
        sweep(blocks, &mut gaps, false);
        self.blocks = blocks;
        self.gaps = gaps;
        self
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

/// Pick `values` out in the order `order` names them.
fn gather<T: Copy>(values: &[T], order: &[u32]) -> Vec<T> {
    order
        .iter()
        .filter_map(|&slot| values.get(slot as usize).copied())
        .collect()
}

/// Coarse blocks spanning `cells` fine cells, rounding up.
fn block_count(cells: i64) -> i64 {
    (cells + BLOCK - 1) / BLOCK
}

/// Flat index of a coordinate in a grid of `dims`, or `None` when it falls
/// outside.
fn flat_index(dims: [i64; 3], cell: [i64; 3]) -> Option<usize> {
    if cell
        .iter()
        .zip(dims)
        .any(|(&value, dim)| value < 0 || value >= dim)
    {
        return None;
    }
    usize::try_from((cell[2] * dims[1] + cell[1]) * dims[0] + cell[0]).ok()
}

/// The coordinate a flat index stands for.
fn unflatten(dims: [i64; 3], flat: i64) -> [i64; 3] {
    let plane = dims[0] * dims[1];
    [flat % dims[0], (flat / dims[0]) % dims[1], flat / plane]
}

/// One raster sweep of the chessboard distance transform over `gaps`.
///
/// A chessboard distance is exactly the number of single steps through the
/// 3x3x3 neighbourhood, so a forward sweep against the already-visited half of
/// that neighbourhood followed by a backward sweep against the other half is
/// exact — no iteration to a fixed point, and the same answer every run.
fn sweep(dims: [i64; 3], gaps: &mut [u8], forward: bool) {
    let total = i64::try_from(gaps.len()).unwrap_or(0);
    for step in 0..total {
        let flat = if forward { step } else { total - 1 - step };
        let Some(&current) = usize::try_from(flat).ok().and_then(|slot| gaps.get(slot)) else {
            continue;
        };
        if current == 0 {
            continue;
        }
        let cell = unflatten(dims, flat);
        let sign = if forward { 1 } else { -1 };
        let mut best = current;
        for offset in EARLIER {
            let neighbour = [
                cell[0] + sign * offset[0],
                cell[1] + sign * offset[1],
                cell[2] + sign * offset[2],
            ];
            if let Some(&found) = flat_index(dims, neighbour).and_then(|slot| gaps.get(slot)) {
                best = best.min(found.saturating_add(1));
            }
        }
        if let Some(entry) = usize::try_from(flat)
            .ok()
            .and_then(|slot| gaps.get_mut(slot))
        {
            *entry = best;
        }
    }
}

/// Find a connected-component root with path compression.
fn find(parent: &mut [usize], node: usize) -> usize {
    if parent[node] == node {
        return node;
    }
    let root = find(parent, parent[node]);
    parent[node] = root;
    root
}

/// Join two indexed vertices into one surface component.
fn union(parent: &mut [usize], left: usize, right: usize) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        parent[right_root] = left_root;
    }
}

#[cfg(test)]
mod tests;
