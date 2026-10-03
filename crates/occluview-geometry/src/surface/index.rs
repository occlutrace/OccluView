//! Nearest-surface queries over a triangle soup.
//!
//! The adaptive grid follows triangle size and uses coarse occupancy data to
//! skip empty space. Queries remain equivalent to a full triangle scan,
//! including tie-breaking. Normals come from triangle winding rather than
//! imported vertex data so deviation signs use the indexed geometry.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use glam::{DAffine3, DMat3, DVec3};

use super::{BuildOutcome, GeometryControl, GeometryMemory, GeometryStop, QueryOutcome, Soup};

#[path = "surface_geometry.rs"]
mod surface_geometry;
use surface_geometry::{closest_feature_on_triangle, Feature};
#[path = "surface_helpers.rs"]
mod surface_helpers;
use surface_helpers::{canonical_bits, cell_count, grid_dims, longest_edge, read_triangle};
#[path = "surface_topology.rs"]
mod surface_topology;
use surface_topology::{Topology, TopologyBuilder};
#[path = "surface_index_support.rs"]
mod surface_index_support;
use surface_index_support::{block_count, find, flat_index, radical_inverse, sweep, union};

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
    feature: Feature,
}

/// Shared accounting and upper bound throughout one nested traversal.
struct Traversal<'a> {
    best: Option<Candidate>,
    tests: u64,
    control: &'a GeometryControl,
    // The same query has the same distance/feature for a triangle in every
    // overlapping bucket. A fixed cache removes repeat arithmetic without
    // allocating or changing the traversal, pruning or source-id tie-break.
    tested: &'a mut [u64; 1024],
    generation: u32,
    hint: Option<(usize, [DVec3; 3])>,
}

/// Reusable, bounded scratch for a serial sequence of exact surface queries.
///
/// Admission and all work use the control supplied to [`Self::new`]. Scratch
/// can be used across indices: tags are invalidated at every query. It caches
/// triangles already tested within the current traversal. The preceding facet
/// supplies an upper bound only after its corners match the current index and
/// its distance is recomputed at the current point. No distance or nearest
/// answer survives a query. It is not cloneable.
pub struct SurfaceQueryScratch {
    tested: [u64; 1024],
    generation: u32,
    hint: Option<(usize, [DVec3; 3])>,
    control: GeometryControl,
    _memory: GeometryMemory,
}

impl SurfaceQueryScratch {
    /// Admit 8 KiB once for a sequence of queries; release it on drop.
    ///
    /// # Errors
    /// Returns cancellation, deadline or allocation admission exhaustion.
    pub fn new(control: &GeometryControl) -> Result<Self, GeometryStop> {
        let memory = control.reserve(size_of::<Self>())?;
        Ok(Self {
            tested: [0; 1024],
            generation: 0,
            hint: None,
            control: control.clone(),
            _memory: memory,
        })
    }

    fn begin(&mut self) -> Result<(), GeometryStop> {
        if let Some(stop) = self.control.checkpoint() {
            return Err(stop);
        }
        if self.generation == u32::MAX {
            for chunk in self.tested.chunks_mut(128) {
                self.control.charge_operations(chunk.len() as u64)?;
                chunk.fill(0);
            }
            self.generation = 0;
        }
        self.generation += 1;
        Ok(())
    }
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
    /// Unit local-winding normal mapped by the authored inverse transpose.
    /// With an identity/proper rigid query frame this is the geometric face normal.
    pub normal: DVec3,
    /// Index of that triangle within the source soup.
    pub triangle: u32,
    /// Unit outward normal of the closest feature: the face normal inside a
    /// face, the mean of the two faces along an edge, the corner-angle-weighted
    /// mean of the faces around a vertex. Which side of the surface the query
    /// point lies on is the sign of `(query - point) · pseudo_normal`; with the
    /// face `normal` that sign is wrong outside a sharp edge whenever the face
    /// listed first in the file points away from the query.
    pub pseudo_normal: DVec3,
    /// The closest feature lies on the surface's open border: an edge only one
    /// indexed triangle uses, or a vertex on one (the rim of a masked region
    /// included). The query's counterpart on this surface, if any, lies beyond
    /// what the surface covers.
    pub on_border: bool,
}

/// A deterministic representative of one indexed triangle for bounded
/// fixed-to-moving overlap checks.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct SurfaceSample {
    /// Triangle centroid in the index's own frame.
    pub point: DVec3,
    /// The triangle's geometric normal.
    pub normal: DVec3,
    /// Connected component containing this triangle.
    pub component: usize,
}

/// One roughly uniform surface representative for global shape matching.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct FeaturePoint {
    /// Representative position in the index's frame.
    pub position: DVec3,
    /// Average geometric normal at the representative position.
    pub normal: DVec3,
}

#[doc(hidden)]
/// Grid size, in millimetres, used to deduplicate sampled surface features.
pub const FEATURE_VOXEL_MM: f64 = 0.7;

/// Bounded voxel coordinates keep the feature grid meaningful and prevent
/// overflow in the local neighbourhood walk.
#[doc(hidden)]
#[allow(clippy::cast_possible_truncation)]
pub fn feature_voxel_key(point: DVec3) -> Option<(i32, i32, i32)> {
    let coordinate = |value: f64| {
        let scaled = (value / FEATURE_VOXEL_MM).floor();
        (scaled.is_finite() && scaled.abs() < 1_000_000.0).then_some(scaled as i32)
    };
    Some((
        coordinate(point.x)?,
        coordinate(point.y)?,
        coordinate(point.z)?,
    ))
}

fn bounded_grid(extent: DVec3, mean_edge: f64) -> (f64, [i64; 3]) {
    let diagonal = extent.length().max(1e-3);
    let mut cell = (mean_edge * CELL_EDGE_FACTOR).clamp(1e-3, diagonal);
    let mut dims = grid_dims(extent, cell);
    while cell_count(dims) > MAX_CELLS {
        cell *= 2.0;
        dims = grid_dims(extent, cell);
    }
    (cell, dims)
}

/// A spatial index answering "what is the closest surface point to this?".
#[derive(Debug)]
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
    triangle_components: Vec<usize>,
    topology: Topology,
    surface_area_mm2: f64,
    memory: Vec<GeometryMemory>,
    query_control: Option<GeometryControl>,
    uncontrolled: GeometryControl,
    normal_frame_valid: bool,
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
    pub fn build(soup: Soup<'_>) -> Option<Self> {
        match Self::build_controlled(soup, DAffine3::IDENTITY, &GeometryControl::unlimited()) {
            BuildOutcome::Complete(index) => Some(index),
            BuildOutcome::Partial { .. } | BuildOutcome::Empty => None,
        }
    }

    /// Build the existing grid in a private f64 query frame without baking f32 vertices.
    ///
    /// Every topology/bucket operation is charged. Allocations are admitted
    /// before growth and reserved fallibly. Invalid or masked facets are omitted;
    /// arithmetic overflow returns an explicit partial outcome, never an exact
    /// prefix index. Original source triangle ids and winding are preserved.
    pub fn build_controlled(
        soup: Soup<'_>,
        local_to_query: DAffine3,
        control: &GeometryControl,
    ) -> BuildOutcome<Self> {
        match Self::try_build(soup, local_to_query, control) {
            Ok(Some(index)) => BuildOutcome::Complete(index),
            Ok(None) => BuildOutcome::Empty,
            Err(reason) => BuildOutcome::Partial {
                value: None,
                reason,
            },
        }
    }

    #[allow(clippy::cast_precision_loss)]
    #[expect(
        clippy::too_many_lines,
        reason = "one ordered topology and allocation transaction"
    )]
    fn try_build(
        soup: Soup<'_>,
        affine: DAffine3,
        control: &GeometryControl,
    ) -> Result<Option<Self>, GeometryStop> {
        control.charge_operations(1)?;
        if !affine.is_finite() {
            return Err(GeometryStop::Numerical);
        }
        let determinant = affine.matrix3.determinant();
        let normal_matrix = if determinant.is_finite() && determinant != 0. {
            let inverse = affine.matrix3.inverse().transpose();
            inverse.is_finite().then_some(inverse)
        } else {
            None
        };
        let mut normal_frame_valid = normal_matrix.is_some();
        let vertices = soup.vertex_count();
        let triangles = soup.triangle_count();
        let limits = control.limits();
        if vertices > limits.input_vertices
            || triangles > limits.input_triangles
            || triangles > u32::MAX as usize
        {
            return Err(control.stop(GeometryStop::ResourceLimit));
        }
        // Includes conservative hash-table load/rounding, both topology output
        // and builder storage, components and triangle arrays at their peak.
        let bytes = triangles
            .checked_mul(1024)
            .and_then(|n| vertices.checked_mul(256).and_then(|v| n.checked_add(v)))
            .and_then(|n| n.checked_add(4096))
            .ok_or(GeometryStop::ResourceLimit)?;
        let mut allocation = control.reserve(bytes)?;
        let mut corners = allocated_vec(triangles, control)?;
        let mut normals = allocated_vec(triangles, control)?;
        let mut sources = allocated_vec(triangles, control)?;
        let mut parent: Vec<usize> = allocated_vec(vertices, control)?;
        let mut sizes = allocated_vec(vertices, control)?;
        let mut bounds = allocated_vec(vertices, control)?;
        let mut roots = allocated_vec(vertices, control)?;
        for vertex in 0..vertices {
            control.charge_operations(1)?;
            parent.push(vertex);
            sizes.push(1usize);
            bounds.push(None::<(DVec3, DVec3)>);
            roots.push(usize::MAX);
        }
        let mut positions = HashMap::<[u64; 3], usize>::new();
        positions
            .try_reserve(vertices.min(triangles.saturating_mul(3)))
            .map_err(|_| control.stop(GeometryStop::ResourceLimit))?;
        let mut anchors = allocated_vec(triangles, control)?;
        let mut topology = TopologyBuilder::with_capacity(
            vertices.min(triangles.saturating_mul(3)),
            triangles,
            control,
        )?;
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let mut edge_total = 0.;
        let mut area = 0.;
        for (source, ids) in soup.indices.as_chunks::<3>().0.iter().enumerate() {
            control.charge_operations(1)?;
            if ids.iter().any(|&id| soup.is_excluded(id as usize)) {
                continue;
            }
            let Some(local) = read_triangle(soup.positions, vertices, ids) else {
                continue;
            };
            let points = local.map(|p| affine.transform_point3(p));
            if points.iter().any(|p| !p.is_finite()) {
                return Err(GeometryStop::Numerical);
            }
            let mut welded = [0; 3];
            for corner in 0..3 {
                control.charge_operations(1)?;
                let vertex = ids[corner] as usize;
                let key = local[corner].to_array().map(canonical_bits);
                let first = *positions.entry(key).or_insert(vertex);
                union(&mut parent, &mut sizes, first, vertex);
                welded[corner] = first;
            }
            let cross = (points[1] - points[0]).cross(points[2] - points[0]);
            let length = cross.length();
            if !length.is_finite() {
                return Err(GeometryStop::Numerical);
            }
            if length <= MIN_DOUBLE_AREA {
                topology.add_connector(welded, control)?;
                continue;
            }
            let edge = longest_edge(&points);
            if !(edge_total + edge).is_finite() || !(area + length * 0.5).is_finite() {
                return Err(GeometryStop::Numerical);
            }
            for point in points {
                min = min.min(point);
                max = max.max(point);
            }
            union(&mut parent, &mut sizes, ids[0] as usize, ids[1] as usize);
            union(&mut parent, &mut sizes, ids[1] as usize, ids[2] as usize);
            anchors.push(ids[0] as usize);
            let local_cross = (local[1] - local[0]).cross(local[2] - local[0]);
            let mapped = normal_matrix.and_then(|matrix| mapped_normal(matrix, local_cross));
            normal_frame_valid &= mapped.is_some();
            let normal = mapped.unwrap_or(cross / length);
            topology.add_kept(welded, &points, normal, control)?;
            corners.push(points);
            normals.push(normal);
            sources.push(u32::try_from(source).map_err(|_| GeometryStop::ResourceLimit)?);
            edge_total += edge;
            area += length * 0.5;
        }
        if corners.is_empty() {
            return Ok(None);
        }
        for (points, &anchor) in corners.iter().zip(&anchors) {
            control.charge_operations(1)?;
            let root = find(&mut parent, anchor);
            let low = points[0].min(points[1]).min(points[2]);
            let high = points[0].max(points[1]).max(points[2]);
            let entry = &mut bounds[root];
            *entry = Some(entry.map_or((low, high), |(a, b)| (a.min(low), b.max(high))));
        }
        let mut components = allocated_vec(corners.len().min(vertices), control)?;
        for (root, bound) in bounds.into_iter().enumerate() {
            control.charge_operations(1)?;
            if let Some(bound) = bound {
                roots[root] = components.len();
                components.push(bound);
            }
        }
        let mut triangle_components = allocated_vec(corners.len(), control)?;
        for anchor in anchors {
            control.charge_operations(1)?;
            triangle_components.push(roots[find(&mut parent, anchor)]);
        }
        let extent = max - min;
        if !extent.is_finite() || !extent.length().is_finite() {
            return Err(GeometryStop::Numerical);
        }
        let (cell, dims) = bounded_grid(extent, edge_total / corners.len() as f64);
        let topology = topology.finish(control)?;
        drop(parent);
        drop(sizes);
        drop(positions);
        drop(roots);
        // Builder scratch is gone. Retain conservative capacity accounting for
        // the arrays that remain resident, including component and topology data.
        let resident = corners
            .capacity()
            .saturating_mul(256)
            .saturating_add(topology.vertex_normals.capacity().saturating_mul(16))
            .saturating_add(4096);
        allocation.shrink_to(resident);
        let mut memory = allocated_vec(4, control)?;
        memory.push(allocation);
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
            components,
            triangle_components,
            topology,
            surface_area_mm2: area,
            memory,
            query_control: None,
            uncontrolled: GeometryControl::unlimited(),
            normal_frame_valid,
        };
        Ok(Some(index.with_buckets(control)?.with_gaps(control)?))
    }

    /// Apply shared bounded admission to the legacy nearest-call entry point.
    /// Interrupted answers become absent there; completeness is retained by
    /// [`Self::nearest_controlled`] for evidence consumers.
    pub fn set_query_control(&mut self, control: GeometryControl) {
        self.query_control = Some(control);
    }

    /// Conservative resident bytes for admitting a borrowed cached index.
    /// Original mesh buffers are excluded; query arrays and topology are included.
    pub fn resident_size_bytes(&self) -> u64 {
        self.memory.iter().fold(0u64, |sum, allocation| {
            sum.saturating_add(allocation.bytes())
        })
    }

    /// Shared bounded query lifetime, if attached by a registration job.
    pub fn query_control(&self) -> Option<&GeometryControl> {
        self.query_control.as_ref()
    }

    /// Source triangles in stable source order; points are already f64 query coordinates.
    pub fn triangles(&self) -> impl Iterator<Item = (u32, [DVec3; 3], DVec3)> + '_ {
        self.sources
            .iter()
            .copied()
            .zip(self.corners.iter().copied())
            .zip(self.normals.iter().copied())
            .map(|((source, points), normal)| (source, points, normal))
    }

    /// Whether directed shared edges have consistent winding and manifold incidence.
    pub fn orientation_coherent(&self) -> bool {
        self.topology.orientation_coherent && self.normal_frame_valid
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

    /// Total area of the indexed, unmasked, non-degenerate surface in square
    /// millimetres.
    #[must_use]
    pub fn surface_area_mm2(&self) -> f64 {
        self.surface_area_mm2
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

    /// Deterministically sample surface area and aggregate it into 0.7 mm
    /// voxels. The source mesh's triangle count and order must not decide how
    /// much matching evidence a physical patch contributes.
    #[allow(clippy::cast_precision_loss)]
    #[doc(hidden)]
    pub fn feature_cloud(&self) -> Vec<FeaturePoint> {
        const SAMPLE_COUNT: usize = 30_000;
        if self.corners.len() < 10_000 {
            return Vec::new();
        }
        let mut areas = Vec::new();
        if areas.try_reserve_exact(self.corners.len()).is_err() {
            if let Some(control) = &self.query_control {
                control.stop(GeometryStop::ResourceLimit);
            }
            return Vec::new();
        }
        let mut total = 0.;
        for triangle in &self.corners {
            if self
                .query_control
                .as_ref()
                .is_some_and(|c| c.charge_operations(1).is_err())
            {
                return Vec::new();
            }
            let area = (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .length()
                * 0.5;
            areas.push(area);
            total += area;
        }
        if !total.is_finite() || total <= 0.0 {
            return Vec::new();
        }
        let mut cells: BTreeMap<(i32, i32, i32), (DVec3, DVec3, u32)> = BTreeMap::new();
        let mut triangle_slot = 0;
        let mut preceding_area = 0.0;
        for sample_slot in 0..SAMPLE_COUNT {
            if self
                .query_control
                .as_ref()
                .is_some_and(|c| c.charge_operations(1).is_err())
            {
                return Vec::new();
            }
            let target = (sample_slot as f64 + 0.5) * total / SAMPLE_COUNT as f64;
            while triangle_slot + 1 < areas.len() && preceding_area + areas[triangle_slot] < target
            {
                if self
                    .query_control
                    .as_ref()
                    .is_some_and(|c| c.charge_operations(1).is_err())
                {
                    return Vec::new();
                }
                preceding_area += areas[triangle_slot];
                triangle_slot += 1;
            }
            let triangle = self.corners[triangle_slot];
            let bary_u = radical_inverse(sample_slot + 1, 2).sqrt();
            let bary_v = radical_inverse(sample_slot + 1, 3);
            let point = triangle[0] * (1.0 - bary_u)
                + triangle[1] * (bary_u * (1.0 - bary_v))
                + triangle[2] * (bary_u * bary_v);
            let Some(key) = feature_voxel_key(point) else {
                return Vec::new();
            };
            let entry = cells.entry(key).or_insert((DVec3::ZERO, DVec3::ZERO, 0));
            entry.0 += point;
            entry.1 += self.normals[triangle_slot];
            entry.2 += 1;
        }
        let stride = cells.len().div_ceil(4096).max(1);
        cells
            .into_values()
            .enumerate()
            .filter(|(ordinal, _)| ordinal.is_multiple_of(stride))
            .map(|(_, entry)| entry)
            .filter_map(|(point, normal, count)| {
                let normal = normal.normalize_or_zero();
                (count > 0 && normal.length_squared() > 0.0).then_some(FeaturePoint {
                    position: point / f64::from(count),
                    normal,
                })
            })
            .collect()
    }

    /// Return at most `budget` deterministic triangle representatives.
    ///
    /// The samples are used only as an independent overlap signal; nearest
    /// queries and deviation maps still use the complete indexed surface.
    #[must_use]
    #[doc(hidden)]
    pub fn representative_samples(&self, budget: usize) -> Vec<SurfaceSample> {
        if budget == 0 || self.corners.is_empty() {
            return Vec::new();
        }
        let target = budget.min(self.corners.len());
        let mut selected = vec![false; self.corners.len()];
        let mut slots = Vec::with_capacity(target);

        // A plain source-order stride can spend the whole reciprocal budget
        // in one dense component and miss a small adjacent tooth. Give each
        // component a deterministic representative first, then fill the rest
        // with an even walk through source triangles.
        let component_slots = target.min(self.components.len());
        let mut first_component_slots = vec![None; self.components.len()];
        for (slot, &component) in self.triangle_components.iter().enumerate() {
            if let Some(first) = first_component_slots.get_mut(component) {
                *first = (*first).or(Some(slot));
            }
        }
        for rank in 0..component_slots {
            let component = rank * self.components.len() / component_slots;
            if let Some(Some(slot)) = first_component_slots.get(component).copied() {
                selected[slot] = true;
                slots.push(slot);
            }
        }

        let stride = self.corners.len().div_ceil(target).max(1);
        for slot in (0..self.corners.len()).step_by(stride) {
            if slots.len() == target {
                break;
            }
            if !selected[slot] {
                selected[slot] = true;
                slots.push(slot);
            }
        }
        if slots.len() < target {
            for (slot, is_selected) in selected.iter_mut().enumerate() {
                if slots.len() == target {
                    break;
                }
                if !*is_selected {
                    *is_selected = true;
                    slots.push(slot);
                }
            }
        }
        slots.sort_unstable();
        slots
            .into_iter()
            .filter_map(|slot| {
                let corners = self.corners.get(slot)?;
                let normal = *self.normals.get(slot)?;
                let component = *self.triangle_components.get(slot)?;
                Some(SurfaceSample {
                    point: (corners[0] + corners[1] + corners[2]) / 3.0,
                    normal,
                    component,
                })
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
        let control = self.query_control.as_ref().unwrap_or(&self.uncontrolled);
        match self.nearest_controlled(point, radius, control) {
            QueryOutcome::Complete(hit) => hit,
            QueryOutcome::Interrupted { .. } => None,
        }
    }

    /// Exact nearest answer, or an explicitly interrupted upper bound.
    ///
    /// Charges each distance calculation, every bucket entry, cell and ring.
    /// A fixed 1,024-entry map avoids repeat triangle arithmetic in overlapping
    /// buckets, with no collisions for indexes of at most 1,024 triangles.
    /// Its 8 KiB stack scratch is admitted against resident work memory.
    /// Larger-index cache collisions only repeat work. A dense query cannot exceed
    /// its triangle-test ceiling. Invalid query coordinates/radius give exact
    /// absence; incomplete traversal never masquerades as exact absence.
    pub fn nearest_controlled(
        &self,
        point: DVec3,
        radius: f64,
        control: &GeometryControl,
    ) -> QueryOutcome<SurfaceHit> {
        let mut scratch = match SurfaceQueryScratch::new(control) {
            Ok(scratch) => scratch,
            Err(reason) => return QueryOutcome::Interrupted { best: None, reason },
        };
        self.nearest_with_scratch(point, radius, &mut scratch)
    }

    /// Exact controlled nearest query using previously admitted serial scratch.
    ///
    /// Uses the scratch's original shared control and the same traversal,
    /// triangle-test accounting, tie-breaking and interruption contract as
    /// [`Self::nearest_controlled`]. No evidence persists across calls.
    pub fn nearest_with_scratch(
        &self,
        point: DVec3,
        radius: f64,
        scratch: &mut SurfaceQueryScratch,
    ) -> QueryOutcome<SurfaceHit> {
        if let Err(reason) = scratch.begin() {
            return QueryOutcome::Interrupted { best: None, reason };
        }
        let mut traversal = Traversal {
            best: None,
            tests: 0,
            control: &scratch.control,
            tested: &mut scratch.tested,
            generation: scratch.generation,
            hint: scratch.hint,
        };
        let outcome = self.query(point, radius, &mut traversal);
        let hit = traversal
            .best
            .map(|found: Candidate| self.hit(found.slot, found.source, found.point, found.feature));
        scratch.hint = traversal.best.and_then(|found| {
            self.corners
                .get(found.slot)
                .copied()
                .map(|corners| (found.slot, corners))
        });
        match outcome {
            Ok(()) => QueryOutcome::Complete(hit),
            Err(reason) => {
                if self.query_control.is_some() {
                    scratch.control.stop(reason);
                }
                QueryOutcome::Interrupted { best: hit, reason }
            }
        }
    }

    fn query(
        &self,
        point: DVec3,
        radius: f64,
        traversal: &mut Traversal<'_>,
    ) -> Result<(), GeometryStop> {
        traversal.control.begin_query()?;
        if !point.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return Ok(());
        }
        // Every triangle lies inside the mesh box, so a point farther from that
        // box than the radius cannot reach any of them. One clamp answers the
        // whole query for a vertex sitting off the end of the other scan.
        if (point.clamp(self.min, self.max) - point).length_squared() > radius * radius {
            return Ok(());
        }
        // A point on any current facet bounds the nearest distance from above.
        // Recompute it, then retain the ordinary complete traversal and exact
        // source-id tie rule. Matching corners makes reuse safe across indices,
        // revisions and moved index storage without a pointer-identity token.
        if let Some((slot, corners)) = traversal.hint {
            if self.corners.get(slot) == Some(&corners) {
                traversal.control.triangle_test(&mut traversal.tests)?;
                let (candidate, feature) =
                    closest_feature_on_triangle(point, corners[0], corners[1], corners[2]);
                let distance = (candidate - point).length_squared();
                if !distance.is_finite() || !candidate.is_finite() {
                    return Err(GeometryStop::Numerical);
                }
                if distance <= radius * radius {
                    traversal.best = Some(Candidate {
                        distance,
                        source: self.sources.get(slot).copied().unwrap_or(u32::MAX),
                        point: candidate,
                        slot,
                        feature,
                    });
                }
            }
        }
        let bounded_radius = traversal.best.map_or(radius, |found| {
            (found.distance + 32. * f64::EPSILON * found.distance.max(1.))
                .sqrt()
                .min(radius)
        });
        let reach = DVec3::splat(bounded_radius);
        let query = Query {
            point,
            limit: radius * radius,
            home: self.cell_of(point),
            low: self.cell_of(point - reach),
            high: self.cell_of(point + reach),
        };

        // Shells the coarse level already proved empty are not walked at all.
        // For a point sitting in open space this is the whole answer: the walk
        // starts past the influence radius and never begins.
        for ring in self.first_ring(query.home)..=query.rings() {
            // A shell is skipped only when its own floor already exceeds the
            // best distance so far — never when it merely equals it, because an
            // equal distance is a tie that may still carry a lower source
            // index. That is the same test the per-cell prune makes, so the
            // answer is the one a full sweep of the window would give.
            traversal.control.charge_operations(1)?;
            let ceiling = traversal.best.map_or(query.limit, |found| found.distance);
            if self.ring_floor(&query, ring) > ceiling {
                break;
            }
            self.visit_ring(&query, ring, traversal)?;
        }

        Ok(())
    }

    /// The hit a query reports for the closest point `point` on `slot`.
    fn hit(&self, slot: usize, source: u32, point: DVec3, feature: Feature) -> SurfaceHit {
        let normal = self.normals.get(slot).copied().unwrap_or(DVec3::Z);
        let (pseudo_normal, on_border) = self.topology.at(slot, feature, normal);
        SurfaceHit {
            point,
            normal,
            triangle: source,
            pseudo_normal,
            on_border,
        }
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
    fn visit_ring(
        &self,
        query: &Query,
        ring: i64,
        traversal: &mut Traversal<'_>,
    ) -> Result<(), GeometryStop> {
        let (home, low, high) = (query.home, query.low, query.high);
        let span = |axis: usize| {
            (home[axis].saturating_sub(ring).max(low[axis]))
                ..=(home[axis].saturating_add(ring).min(high[axis]))
        };
        let columns = span(0);
        let run = columns.end() - columns.start() + 1;
        for z in span(2) {
            traversal.control.charge_operations(1)?;
            let z_edge = (z - home[2]).abs() == ring;
            for y in span(1) {
                traversal.control.charge_operations(1)?;
                if z_edge || (y - home[1]).abs() == ring {
                    self.visit_run(query, [*columns.start(), y, z], run, traversal)?;
                } else {
                    for x in [home[0] - ring, home[0] + ring] {
                        if x >= low[0] && x <= high[0] {
                            self.visit_run(query, [x, y, z], 1, traversal)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Test a run of `length` cells along x, starting at `start`.
    ///
    /// Walk the bucket table directly so empty cells require only adjacent
    /// reads.
    fn visit_run(
        &self,
        query: &Query,
        start: [i64; 3],
        length: i64,
        traversal: &mut Traversal<'_>,
    ) -> Result<(), GeometryStop> {
        let Some(base) = self.cell_index(start) else {
            return Ok(());
        };
        // Admit at most 128 cell visits at once. Successful traversals charge
        // exactly the same visits; interrupted chunks may conservatively charge
        // unvisited cells, never perform uncharged work.
        for start_offset in (0..length).step_by(128) {
            let end_offset = start_offset.saturating_add(128).min(length);
            traversal.control.charge_operations(
                u64::try_from(end_offset - start_offset).map_err(|_| GeometryStop::Numerical)?,
            )?;
            for offset in start_offset..end_offset {
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
                    traversal,
                )?;
            }
        }
        Ok(())
    }

    /// Test one cell's triangles against the running best.
    fn visit_cell(
        &self,
        query: &Query,
        cell: [i64; 3],
        bucket: Range<u32>,
        traversal: &mut Traversal<'_>,
    ) -> Result<(), GeometryStop> {
        let ceiling = traversal.best.map_or(query.limit, |found| found.distance);
        if self.cell_distance_squared(query.point, cell) > ceiling {
            return Ok(());
        }
        let Some(bucket) = self.items.get(bucket.start as usize..bucket.end as usize) else {
            return Ok(());
        };
        for chunk in bucket.chunks(128) {
            traversal.control.charge_operations(chunk.len() as u64)?;
            for &slot in chunk {
                // Small indexes have a collision-free direct map. Larger indexes
                // keep fixed scratch and spread both adjacent facets and mesh rows.
                let cache_slot = if self.corners.len() <= traversal.tested.len() {
                    slot as usize
                } else {
                    (slot.wrapping_mul(0x9e37_79b9) >> 22) as usize
                };
                let tag = (u64::from(traversal.generation) << 32) | u64::from(slot);
                if traversal.tested[cache_slot] == tag {
                    continue;
                }
                traversal.tested[cache_slot] = tag;
                let slot = slot as usize;
                let Some(corners) = self.corners.get(slot) else {
                    continue;
                };
                // A bucket covers whole cells and may contain distant facets.
                // Their boxes give conservative lower bounds before expensive
                // triangle distances. Keep equality (and rounding slack) eligible
                // so shared-feature/source-id ties still follow the exact rule.
                let low = corners[0].min(corners[1]).min(corners[2]);
                let high = corners[0].max(corners[1]).max(corners[2]);
                let floor = (query.point.clamp(low, high) - query.point).length_squared();
                let ceiling = traversal.best.map_or(query.limit, |found| found.distance);
                if floor > ceiling + 32. * f64::EPSILON * ceiling.max(1.) {
                    continue;
                }
                traversal.control.triangle_test(&mut traversal.tests)?;
                let (candidate, feature) =
                    closest_feature_on_triangle(query.point, corners[0], corners[1], corners[2]);
                let distance = (candidate - query.point).length_squared();
                if !distance.is_finite() || !candidate.is_finite() {
                    return Err(GeometryStop::Numerical);
                }
                if distance > query.limit {
                    continue;
                }
                let source = self.sources.get(slot).copied().unwrap_or(u32::MAX);
                // The exact equality is deliberate: an exact tie is what a shared
                // edge or a duplicated facet produces, and breaking it on the lower
                // source index is what makes the answer independent of traversal
                // order.
                #[allow(clippy::float_cmp)]
                let better = match traversal.best {
                    None => true,
                    Some(found) => {
                        distance < found.distance
                            || (distance == found.distance && source < found.source)
                    }
                };
                if better {
                    traversal.best = Some(Candidate {
                        distance,
                        source,
                        point: candidate,
                        slot,
                        feature,
                    });
                }
            }
        }
        Ok(())
    }

    /// Count and scatter buckets under shared work and memory admission.
    fn with_buckets(mut self, control: &GeometryControl) -> Result<Self, GeometryStop> {
        let cells = cell_count(self.dims);
        let bytes = cells
            .checked_add(1)
            .and_then(|n| n.checked_mul(12))
            .ok_or(GeometryStop::ResourceLimit)?;
        let temporary = control.reserve(bytes)?;
        let mut counts = allocated_vec(cells + 1, control)?;
        for _ in 0..=cells {
            control.charge_operations(1)?;
            counts.push(0u32);
        }
        for corners in &self.corners {
            self.for_each_cell(corners, control, |cell| {
                counts[cell + 1] = counts[cell + 1]
                    .checked_add(1)
                    .ok_or(GeometryStop::ResourceLimit)?;
                Ok(())
            })?;
        }
        for slot in 1..counts.len() {
            control.charge_operations(1)?;
            counts[slot] = counts[slot]
                .checked_add(counts[slot - 1])
                .ok_or(GeometryStop::ResourceLimit)?;
        }
        let total = counts.last().copied().unwrap_or(0) as usize;
        let resident = control.reserve(
            total
                .checked_add(cells + 1)
                .and_then(|n| n.checked_mul(4))
                .ok_or(GeometryStop::ResourceLimit)?,
        )?;
        let mut items = allocated_vec(total, control)?;
        for _ in 0..total {
            control.charge_operations(1)?;
            items.push(0u32);
        }
        let mut cursor = allocated_vec(counts.len(), control)?;
        for &count in &counts {
            control.charge_operations(1)?;
            cursor.push(count);
        }
        for (triangle, corners) in self.corners.iter().enumerate() {
            let triangle = u32::try_from(triangle).map_err(|_| GeometryStop::ResourceLimit)?;
            self.for_each_cell(corners, control, |cell| {
                let slot = cursor[cell] as usize;
                if let Some(item) = items.get_mut(slot) {
                    *item = triangle;
                }
                cursor[cell] = cursor[cell]
                    .checked_add(1)
                    .ok_or(GeometryStop::ResourceLimit)?;
                Ok(())
            })?;
        }
        self.starts = counts;
        self.items = items;
        self.memory.push(resident);
        drop(cursor);
        drop(temporary);
        Ok(self)
    }

    fn for_each_cell(
        &self,
        corners: &[DVec3; 3],
        control: &GeometryControl,
        mut visit: impl FnMut(usize) -> Result<(), GeometryStop>,
    ) -> Result<(), GeometryStop> {
        let low = self.cell_of(corners[0].min(corners[1]).min(corners[2]));
        let high = self.cell_of(corners[0].max(corners[1]).max(corners[2]));
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    control.charge_operations(1)?;
                    if let Some(cell) = self.cell_index([x, y, z]) {
                        visit(cell)?;
                    }
                }
            }
        }
        Ok(())
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
    fn with_gaps(mut self, control: &GeometryControl) -> Result<Self, GeometryStop> {
        let blocks = [
            block_count(self.dims[0]),
            block_count(self.dims[1]),
            block_count(self.dims[2]),
        ];
        let count = cell_count(blocks);
        let memory = control.reserve(count)?;
        let mut gaps = allocated_vec(count, control)?;
        for _ in 0..count {
            control.charge_operations(1)?;
            gaps.push(u8::MAX);
        }
        let mut cell = 0usize;
        for z in 0..self.dims[2] {
            for y in 0..self.dims[1] {
                for x in 0..self.dims[0] {
                    control.charge_operations(1)?;
                    let occupied = self.starts.get(cell) != self.starts.get(cell + 1);
                    cell += 1;
                    if occupied {
                        if let Some(entry) = flat_index(blocks, [x / BLOCK, y / BLOCK, z / BLOCK])
                            .and_then(|flat| gaps.get_mut(flat))
                        {
                            *entry = 0;
                        }
                    }
                }
            }
        }
        sweep(blocks, &mut gaps, true, control)?;
        sweep(blocks, &mut gaps, false, control)?;
        self.blocks = blocks;
        self.gaps = gaps;
        self.memory.push(memory);
        Ok(self)
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

fn mapped_normal(matrix: DMat3, normal: DVec3) -> Option<DVec3> {
    let transformed = matrix * normal;
    let scale = transformed.abs().max_element();
    if !transformed.is_finite() || scale <= 0. {
        return None;
    }
    let normalized = (transformed / scale).normalize_or_zero();
    (normalized.is_finite() && normalized.length_squared() > 0.).then_some(normalized)
}

fn allocated_vec<T>(capacity: usize, control: &GeometryControl) -> Result<Vec<T>, GeometryStop> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| control.stop(GeometryStop::ResourceLimit))?;
    Ok(values)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
