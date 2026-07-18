//! Freeform sculpting brushes (issue #11): an interactive Add/Remove clay knife
//! and a Smooth relaxer, applied over a soft-falloff disc on the surface — the
//! geometry half of exocad-style freeforming applied to intraoral SCAN meshes.
//!
//! # Session shape
//!
//! A [`BrushSession`] is prepared ONCE per layer when the operator first
//! touches it (welds STL soup, builds adjacency/incidence/boundary/spatial-grid
//! — the one-time O(n) cost an interactive drag amortizes over many dabs) and
//! is reused across every stroke on that layer. Each dab is one [`BrushStroke`]
//! via [`BrushSession::apply_stroke`], returning only the touched vertex ids so
//! the caller can push a PARTIAL GPU buffer update instead of re-uploading the
//! whole scan. [`BrushSession::finish`] can bake the accumulated edits into a
//! [`MeshEditResult`] for callers that prefer the batch commit path.
//!
//! # Why it stays clean (no potholes / no spikes)
//!
//! The naive "move every vertex along its OWN normal" carves potholes (adjacent
//! per-vertex normals diverge) and spikes (a lone vertex outruns its ring).
//! Instead Add/Remove moves the whole brushed region COHERENTLY along a single
//! averaged brush normal. That normal is computed the way Blender's sculpt mode
//! does it — bucket the sampled normals by whether they face the camera and
//! average only the front bucket, falling back to the pure camera direction
//! when the surface can't be trusted — so a scan's inverted-normal patches never
//! flip the push direction. Each dab is followed by a TANGENTIAL relaxation
//! pass that slides vertices sideways to even out the triangulation WITHOUT
//! undoing the sculpted height. Smooth is a strong uniform-Laplacian relaxer run
//! as several whole passes (fractional Taubin per frame is imperceptible — the
//! reason the old smooth did nothing). Both pin open scan boundaries so the
//! scan's outer edge never erodes, and never reach across the gap between two
//! disconnected surfaces (adjacency is per welded component).
//!
//! # Soup correctness
//!
//! STL stores each triangle's corners as independent vertices, so the vertex
//! ARRAY still has orphaned duplicates at a moved corner even after the session
//! welds INDEX topology for adjacency (`weld_soup_topology` rewrites triangle
//! indices, never the vertex array). Every touched vertex's new position and
//! normal are propagated to every other vertex slot that started at the exact
//! same position (`position_siblings`), or a soup scan would crack at every
//! touched corner.

use glam::Vec3;
use std::collections::{HashMap, HashSet};

use super::brush_index::VertexGrid;
use super::cap_support::build_vertex_adjacency;
use super::topology::{canonical_position_key, weld_soup_topology};
use super::{
    validate_face_edit_buffers, EditVertex, MeshEditBuffers, MeshEditError, MeshEditReport,
    MeshEditResult, MeshTopology,
};

/// Uniform-Laplacian blend factor per Smooth pass. Strong enough to visibly
/// relax in a few passes, below the ~0.8 where irregular valence starts to
/// oscillate. Smoothing STRENGTH is expressed as the number of whole passes
/// (see [`smooth_pass_count`]), never as a smaller factor — a fractional single
/// pass is what made the old smooth imperceptible.
const SMOOTH_LAMBDA: f32 = 0.6;
/// Most Laplacian passes a single forced (Shift) dab runs.
const MAX_SMOOTH_PASSES: usize = 8;
/// Add/Remove displacement per fully-weighted dab, as a fraction of the brush
/// radius. Scaling by radius (not a fixed mm) keeps the brush feeling the same
/// on a coarse or a fine scan and at any zoom; buildup accumulates over the many
/// arc-length-spaced dabs of a drag, so this stays small per dab.
const ADD_REMOVE_GAIN: f32 = 0.08;
/// Strength of the tangential auto-relax that follows every Add/Remove dab: how
/// far each vertex slides toward its ring's tangential centroid to keep the
/// triangulation even. Applied only in-plane, so it never erases height.
const AUTOSMOOTH_FACTOR: f32 = 0.5;
/// Largest displacement step as a fraction of a vertex's shortest incident
/// (welded) edge — the anti-inversion guard. Coherent brush motion keeps
/// neighbours moving together, so this binds mainly at the brush rim.
const MAX_STEP_FRACTION_OF_EDGE: f32 = 0.5;
/// The toward-camera normal bucket must hold at least this share of the sampled
/// weight for the averaged surface normal to be trusted; below it the patch is
/// too inverted/noisy and the brush builds straight toward the camera instead.
const FRONT_BUCKET_TRUST_FRACTION: f32 = 0.6;
/// A vertex with fewer than this many welded neighbors is a needle/spike tip,
/// not a real interior vertex; Smooth and the auto-relax leave it alone.
const MIN_RING_FOR_RELAX: usize = 3;
/// Grid-drift rebuild threshold, as a fraction of the grid's own cell size.
/// Once a vertex has drifted more than this fraction of a cell width, its true
/// position may have crossed into a cell `query_radius` no longer searches, so
/// the index is rebuilt before it could silently drop a moved vertex.
const GRID_REBUILD_DRIFT_FRACTION_OF_CELL: f32 = 0.5;

/// One brush dab: a soft-falloff disc centered on the surface.
#[derive(Copy, Clone, Debug)]
pub struct BrushStroke {
    /// Mesh-local dab center (a ray/mesh hit point transformed into the layer's
    /// own space).
    pub center: [f32; 3],
    /// Falloff radius in mesh-local mm; zero effect at/beyond this distance.
    pub radius_mm: f32,
    /// Dab strength, 0..1 (0 is a no-op). Add/Remove scale their per-dab
    /// displacement by it; Smooth turns it into a pass count. Cadence (how many
    /// dabs a drag lands) is the caller's job — magnitude here is per-dab so the
    /// brush is framerate-independent when the caller spaces dabs by arc length.
    pub strength: f32,
    /// Unit view direction, pointing FROM the camera INTO the scene. Add/Remove
    /// orient their coherent brush normal toward the camera using this, so
    /// "Add" always builds toward the viewer and "Remove" carves away even
    /// across a scan's inverted-normal patches. Ignored by Smooth; a zero
    /// vector falls back to the averaged surface normal's own sign.
    pub view_dir: [f32; 3],
}

/// Which sculpting operation a dab performs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BrushMode {
    /// Uniform-Laplacian relaxation: irons scanner noise and seams flat,
    /// boundary-pinned, strength = pass count.
    Smooth,
    /// Clay knife building material up toward the camera.
    Add,
    /// Clay knife carving material away from the camera.
    Remove,
}

/// Outcome of one [`BrushSession::apply_stroke`] call: exactly the vertex ids
/// whose position and/or normal changed, so the caller can push a partial GPU
/// update instead of re-uploading the whole mesh. Sorted, deduplicated,
/// indices into the ORIGINAL vertex array `BrushSession::prepare` was built
/// from.
#[derive(Clone, Debug, Default)]
pub struct BrushStrokeOutcome {
    /// Touched vertex ids, ascending.
    pub touched_vertices: Vec<usize>,
}

/// A prepared freeform-sculpting session over one mesh. See the module docs
/// for the amortized-cost shape and soup-correctness contract.
pub struct BrushSession {
    /// Original vertex attributes (color/uv kept verbatim; position/normal
    /// updated in place as dabs apply). Same length and order as the mesh
    /// `BrushSession::prepare` was built from.
    vertices: Vec<EditVertex>,
    /// The ORIGINAL (unwelded) triangle indices — returned verbatim by `finish`,
    /// since brush dabs only move vertices, never retopologize.
    indices: Vec<u32>,
    /// Vertex-vertex adjacency over the WELDED topology (shared corners see
    /// their true neighbors even across soup duplicates). A soup duplicate that
    /// is not the weld representative has an empty ring; it is moved by sibling
    /// propagation from the representative, never in its own right.
    adjacency: Vec<Vec<usize>>,
    /// Per-ORIGINAL-vertex incident triangle indices (into `indices`), used to
    /// recompute normals scoped to the touched region.
    incident_triangles: Vec<Vec<usize>>,
    /// Every other original vertex id that started at the exact same position
    /// as this one (soup duplicates of one physical corner); empty otherwise.
    position_siblings: Vec<Vec<usize>>,
    /// Whether a vertex sits on an open scan boundary (an edge used by only one
    /// triangle). Boundary vertices are pinned by Smooth and by the auto-relax
    /// so the scan's outer edge and any hole rims never erode.
    is_boundary: Vec<bool>,
    /// Shortest welded-neighbor edge length per vertex, captured at prepare
    /// time — the anti-inversion guard's per-vertex step budget.
    max_step: Vec<f32>,
    /// Spatial index over vertex positions, rebuilt from live positions
    /// whenever drift since the last build could otherwise make a query miss a
    /// moved vertex — see [`Self::rebuild_grid_if_stale`].
    grid: VertexGrid,
    /// Vertex positions as of `grid`'s last build/rebuild — the reference
    /// [`Self::rebuild_grid_if_stale`] measures live-position drift against.
    grid_reference_positions: Vec<Vec3>,
    /// Farthest any vertex has drifted from `grid_reference_positions` since
    /// the last grid build.
    max_drift_since_grid_build: f32,
    /// Every vertex id touched by any dab so far this session — reported
    /// honestly as `report.moved_vertices` by `finish`.
    touched_total: HashSet<usize>,
}

impl BrushSession {
    /// Prepare a session over `mesh`: weld soup topology for adjacency, build
    /// the incidence map, the soup position-cluster map, the boundary mask, the
    /// anti-inversion step budget, and the spatial index.
    ///
    /// # Errors
    /// Returns [`MeshEditError::UnsupportedPointCloud`] or
    /// [`MeshEditError::MalformedMesh`] from the shared buffer validation.
    pub fn prepare(mesh: &MeshEditBuffers) -> Result<Self, MeshEditError> {
        validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;

        let welded = weld_soup_topology(mesh)?;
        let adjacency_source = welded.as_ref().unwrap_or(mesh);
        let adjacency = build_vertex_adjacency(adjacency_source);

        let vertex_count = mesh.vertices.len();
        let mut incident_triangles: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
        for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
            for &raw in triangle {
                if let Some(vertex_id) = usize::try_from(raw).ok().filter(|&i| i < vertex_count) {
                    incident_triangles[vertex_id].push(triangle_index);
                }
            }
        }

        let mut clusters: HashMap<[u32; 3], Vec<usize>> = HashMap::with_capacity(vertex_count);
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            clusters
                .entry(canonical_position_key(vertex.position))
                .or_default()
                .push(index);
        }
        let mut position_siblings: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
        for group in clusters.values() {
            if group.len() < 2 {
                continue;
            }
            for &vertex_id in group {
                position_siblings[vertex_id] = group
                    .iter()
                    .copied()
                    .filter(|&id| id != vertex_id)
                    .collect();
            }
        }

        let is_boundary =
            boundary_mask(&adjacency_source.indices, &position_siblings, vertex_count);

        let positions: Vec<Vec3> = mesh
            .vertices
            .iter()
            .map(|v| Vec3::from_array(v.position))
            .collect();
        let max_step: Vec<f32> = (0..vertex_count)
            .map(|index| shortest_incident_edge(&positions, &adjacency[index], positions[index]))
            .collect();
        let grid = VertexGrid::build(&positions);

        Ok(Self {
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            adjacency,
            incident_triangles,
            position_siblings,
            is_boundary,
            max_step,
            grid,
            grid_reference_positions: positions,
            max_drift_since_grid_build: 0.0,
            touched_total: HashSet::new(),
        })
    }

    /// Apply one dab, mutating touched vertex positions and normals in place.
    /// Returns exactly the touched vertex ids for a partial GPU update; empty
    /// when the dab has no effect (zero strength/radius, or no vertex in reach).
    pub fn apply_stroke(&mut self, stroke: BrushStroke, mode: BrushMode) -> BrushStrokeOutcome {
        let Some((weighted, strength)) = self.weighted_candidates(stroke) else {
            return BrushStrokeOutcome::default();
        };
        let mut touched: Vec<usize> = Vec::new();
        match mode {
            BrushMode::Smooth => self.apply_smooth(&weighted, strength, &mut touched),
            BrushMode::Add => self.apply_clay(&weighted, stroke, 1.0, &mut touched),
            BrushMode::Remove => self.apply_clay(&weighted, stroke, -1.0, &mut touched),
        }
        if touched.is_empty() {
            return BrushStrokeOutcome::default();
        }
        touched.sort_unstable();
        touched.dedup();
        self.touched_total.extend(touched.iter().copied());
        self.recompute_normals_near(&touched);
        BrushStrokeOutcome {
            touched_vertices: touched,
        }
    }

    /// Falloff-weighted vertices within the dab's disc (the grid query is a
    /// conservative superset, filtered here to the vertices actually inside the
    /// radius). Weights are the raw spatial falloff (0..1); the clamped dab
    /// strength is returned separately so Smooth can turn it into a pass count
    /// rather than a magnitude. `None` for a no-effect dab. Rebuilds the spatial
    /// index first if drift since its last build could otherwise miss a vertex.
    fn weighted_candidates(&mut self, stroke: BrushStroke) -> Option<(Vec<(usize, f32)>, f32)> {
        let strength = stroke.strength.clamp(0.0, 1.0);
        if strength <= 0.0 || !stroke.radius_mm.is_finite() || stroke.radius_mm <= 0.0 {
            return None;
        }
        self.rebuild_grid_if_stale();
        let center = Vec3::from_array(stroke.center);
        let candidates = self.grid.query_radius(center, stroke.radius_mm);
        if candidates.is_empty() {
            return None;
        }
        let weighted: Vec<(usize, f32)> = candidates
            .into_iter()
            .filter_map(|vertex_id| {
                let distance = self.position(vertex_id).distance(center);
                let weight = falloff(distance, stroke.radius_mm);
                (weight > 0.0).then_some((vertex_id, weight))
            })
            .collect();
        (!weighted.is_empty()).then_some((weighted, strength))
    }

    /// Rebuild the spatial grid from current (live) positions if any indexed
    /// vertex has drifted far enough since the last build that a query could
    /// silently miss it (see [`GRID_REBUILD_DRIFT_FRACTION_OF_CELL`]).
    fn rebuild_grid_if_stale(&mut self) {
        let threshold = self.grid.cell_size() * GRID_REBUILD_DRIFT_FRACTION_OF_CELL;
        if self.max_drift_since_grid_build <= threshold {
            return;
        }
        let positions: Vec<Vec3> = self
            .vertices
            .iter()
            .map(|v| Vec3::from_array(v.position))
            .collect();
        self.grid = VertexGrid::build(&positions);
        self.grid_reference_positions = positions;
        self.max_drift_since_grid_build = 0.0;
    }

    /// Clay Add (`sign = +1`) / Remove (`sign = -1`): displace the whole brushed
    /// region coherently along one camera-oriented brush normal, then relax
    /// tangentially so the triangulation stays even without losing the sculpted
    /// height.
    fn apply_clay(
        &mut self,
        weighted: &[(usize, f32)],
        stroke: BrushStroke,
        sign: f32,
        touched: &mut Vec<usize>,
    ) {
        let strength = stroke.strength.clamp(0.0, 1.0);
        let normal = self.brush_normal(weighted, Vec3::from_array(stroke.view_dir));
        let amplitude = (stroke.radius_mm * ADD_REMOVE_GAIN * strength).max(0.0);
        let displacement: Vec<(usize, Vec3)> = weighted
            .iter()
            .map(|&(vertex_id, weight)| {
                let target = self.position(vertex_id) + normal * (sign * weight * amplitude);
                (vertex_id, target)
            })
            .collect();
        self.commit_moves(displacement.into_iter(), touched);

        // Tangential auto-relax: slide each brushed vertex toward its ring's
        // centroid, but only within the plane perpendicular to the brush normal,
        // so the added height survives while the triangulation evens out (kills
        // the potholes/spikes the old per-vertex-normal push left).
        let relaxed: Vec<(usize, Vec3)> = weighted
            .iter()
            .filter(|&&(vertex_id, _)| self.is_relaxable(vertex_id))
            .filter_map(|&(vertex_id, weight)| {
                let here = self.position(vertex_id);
                let laplacian = self.ring_centroid(vertex_id)? - here;
                let tangential = laplacian - normal * laplacian.dot(normal);
                Some((vertex_id, here + tangential * (AUTOSMOOTH_FACTOR * weight)))
            })
            .collect();
        self.commit_moves(relaxed.into_iter(), touched);
    }

    /// Smooth: several whole uniform-Laplacian passes (count from `strength`, so
    /// a firmer press or the forced Shift mode simply runs more passes), each
    /// blended by the per-vertex falloff, boundary and needle-tip vertices left
    /// alone so scan edges hold.
    fn apply_smooth(&mut self, weighted: &[(usize, f32)], strength: f32, touched: &mut Vec<usize>) {
        for _ in 0..smooth_pass_count(strength) {
            let proposals = self.laplacian_proposals(weighted);
            self.commit_moves(proposals.into_iter(), touched);
        }
    }

    /// One uniform-Laplacian step per candidate: move each vertex a
    /// [`SMOOTH_LAMBDA`]-and-falloff fraction toward its ring centroid. Reads
    /// pre-pass positions so the pass is iteration-order-independent. Skips
    /// boundary and low-valence vertices.
    fn laplacian_proposals(&self, weighted: &[(usize, f32)]) -> Vec<(usize, Vec3)> {
        weighted
            .iter()
            .filter(|&&(vertex_id, _)| self.is_relaxable(vertex_id))
            .filter_map(|&(vertex_id, weight)| {
                let here = self.position(vertex_id);
                let centroid = self.ring_centroid(vertex_id)?;
                Some((
                    vertex_id,
                    here.lerp(centroid, (SMOOTH_LAMBDA * weight).clamp(0.0, 1.0)),
                ))
            })
            .collect()
    }

    /// Whether a vertex may be relaxed/smoothed: interior (not an open-boundary
    /// vertex) and with a real one-ring (a needle tip is left frozen).
    fn is_relaxable(&self, vertex_id: usize) -> bool {
        !self.is_boundary[vertex_id] && self.adjacency[vertex_id].len() >= MIN_RING_FOR_RELAX
    }

    /// The camera-oriented brush normal: bucket the region's vertex normals by
    /// whether they face the camera (Blender's `calc_area_normal`), average only
    /// the toward-viewer bucket, and fall back to the pure camera direction when
    /// that bucket is too weak to trust. Robust to inverted-normal scan patches,
    /// where a naive signed average would cancel to garbage.
    fn brush_normal(&self, weighted: &[(usize, f32)], view_dir: Vec3) -> Vec3 {
        let view = view_dir.normalize_or_zero();
        let has_view = view.length_squared() > f32::EPSILON;
        let (mut toward, mut toward_weight) = (Vec3::ZERO, 0.0_f32);
        let mut total_weight = 0.0_f32;
        for &(vertex_id, weight) in weighted {
            let normal = Vec3::from_array(self.vertices[vertex_id].normal).normalize_or_zero();
            if normal.length_squared() <= f32::EPSILON {
                continue;
            }
            total_weight += weight;
            if !has_view || normal.dot(view) <= 0.0 {
                toward += normal * weight;
                toward_weight += weight;
            }
        }
        if has_view {
            if total_weight > 0.0 && toward_weight >= FRONT_BUCKET_TRUST_FRACTION * total_weight {
                let normal = toward.normalize_or_zero();
                if normal.length_squared() > f32::EPSILON {
                    return normal;
                }
            }
            return -view;
        }
        let normal = toward.normalize_or_zero();
        if normal.length_squared() > f32::EPSILON {
            normal
        } else {
            Vec3::Z
        }
    }

    /// Mean position of `vertex_id`'s welded one-ring, or `None` for a vertex
    /// with no ring (a bare soup duplicate; it is moved by propagation).
    fn ring_centroid(&self, vertex_id: usize) -> Option<Vec3> {
        let ring = &self.adjacency[vertex_id];
        if ring.is_empty() {
            return None;
        }
        let mut mean = Vec3::ZERO;
        for &neighbor in ring {
            mean += self.position(neighbor);
        }
        #[allow(clippy::cast_precision_loss)]
        Some(mean / ring.len() as f32)
    }

    /// Apply a set of proposed target positions: clamp each against the
    /// anti-inversion budget, move the vertex and its soup siblings, and record
    /// every slot touched. Skips no-op moves so a content no-op never dirties.
    fn commit_moves(
        &mut self,
        moves: impl Iterator<Item = (usize, Vec3)>,
        touched: &mut Vec<usize>,
    ) {
        for (vertex_id, target) in moves {
            let clamped = self.clamp_step(vertex_id, target);
            if clamped == self.position(vertex_id) {
                continue;
            }
            self.set_position(vertex_id, clamped);
            touched.push(vertex_id);
            let sibling_count = self.position_siblings[vertex_id].len();
            for sibling_index in 0..sibling_count {
                let sibling = self.position_siblings[vertex_id][sibling_index];
                self.set_position(sibling, clamped);
                touched.push(sibling);
            }
        }
    }

    /// Clamp a proposed position so the step does not exceed
    /// [`MAX_STEP_FRACTION_OF_EDGE`] of the vertex's shortest incident edge —
    /// the anti-inversion guard.
    fn clamp_step(&self, vertex_id: usize, proposed: Vec3) -> Vec3 {
        let here = self.position(vertex_id);
        let step = proposed - here;
        let budget = self.max_step[vertex_id] * MAX_STEP_FRACTION_OF_EDGE;
        if !budget.is_finite() || budget <= 0.0 {
            return here;
        }
        let length = step.length();
        if length <= budget || length <= f32::EPSILON {
            proposed
        } else {
            here + step * (budget / length)
        }
    }

    /// Current (live) vertex attributes mid-session — same length and order as
    /// the mesh the session was prepared from. Interactive callers read the
    /// touched ids from a dab's outcome and copy these into their own display
    /// buffer for a partial GPU update, without ending the session.
    #[must_use]
    pub fn vertices(&self) -> &[EditVertex] {
        &self.vertices
    }

    /// Current (live) position of a vertex mid-session.
    pub(crate) fn position(&self, vertex_id: usize) -> Vec3 {
        Vec3::from_array(self.vertices[vertex_id].position)
    }

    fn set_position(&mut self, vertex_id: usize, position: Vec3) {
        self.vertices[vertex_id].position = position.to_array();
        let drift = position.distance(self.grid_reference_positions[vertex_id]);
        if drift > self.max_drift_since_grid_build {
            self.max_drift_since_grid_build = drift;
        }
    }

    /// Recompute normals for exactly the touched vertices and their one-ring (a
    /// moved vertex changes its neighbors' face-weighted normals too), using
    /// the ORIGINAL (unwelded) incident-triangle map so every soup duplicate's
    /// own triangle is included.
    fn recompute_normals_near(&mut self, touched: &[usize]) {
        let mut scope: Vec<usize> = touched.to_vec();
        for &vertex_id in touched {
            scope.extend(self.adjacency[vertex_id].iter().copied());
            scope.extend(self.position_siblings[vertex_id].iter().copied());
        }
        scope.sort_unstable();
        scope.dedup();

        let mut triangles: Vec<usize> = scope
            .iter()
            .flat_map(|&vertex_id| self.incident_triangles[vertex_id].iter().copied())
            .collect();
        triangles.sort_unstable();
        triangles.dedup();

        let mut accumulated: HashMap<usize, Vec3> = HashMap::with_capacity(scope.len());
        for &triangle_index in &triangles {
            let base = triangle_index * 3;
            let Some(corners) = self.indices.get(base..base + 3) else {
                continue;
            };
            let ids: Vec<usize> = corners
                .iter()
                .filter_map(|&raw| usize::try_from(raw).ok())
                .collect();
            let [a, b, c] = match ids.as_slice() {
                [a, b, c] => [*a, *b, *c],
                _ => continue,
            };
            let (pa, pb, pc) = (self.position(a), self.position(b), self.position(c));
            let face_normal = (pb - pa).cross(pc - pa);
            if !face_normal.is_finite() || face_normal.length_squared() <= f32::EPSILON {
                continue;
            }
            for corner in [a, b, c] {
                *accumulated.entry(corner).or_insert(Vec3::ZERO) += face_normal;
            }
        }
        for &vertex_id in &scope {
            if let Some(&sum) = accumulated.get(&vertex_id) {
                if sum.length_squared() > f32::EPSILON {
                    self.vertices[vertex_id].normal = sum.normalize().to_array();
                }
            }
        }
    }

    /// Bake the session into a [`MeshEditResult`]: same topology, updated vertex
    /// positions/normals, `report.moved_vertices` set to the true count of
    /// vertices touched across every dab this session.
    #[must_use]
    pub fn finish(self) -> MeshEditResult {
        let input_vertices = self.vertices.len();
        let input_triangles = self.indices.len() / 3;
        let moved_vertices = self.touched_total.len();
        MeshEditResult {
            mesh: MeshEditBuffers {
                vertices: self.vertices,
                indices: self.indices,
                topology: MeshTopology::TriangleMesh,
            },
            report: MeshEditReport {
                input_vertices,
                input_triangles,
                output_vertices: input_vertices,
                output_triangles: input_triangles,
                moved_vertices,
                ..MeshEditReport::default()
            },
        }
    }
}

/// Number of whole Laplacian passes one Smooth dab runs, from its clamped
/// strength: at least one, up to [`MAX_SMOOTH_PASSES`] at full strength (the
/// forced Shift mode passes ~1.0). Expressing strength as pass count — not a
/// smaller per-pass factor — is what makes Smooth visibly strong.
fn smooth_pass_count(strength: f32) -> usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let extra = (strength.clamp(0.0, 1.0) * (MAX_SMOOTH_PASSES - 1) as f32).round() as usize;
    1 + extra
}

/// Mark every vertex that sits on an open boundary — an undirected edge used by
/// exactly one triangle — from the welded indices, then propagate the mark to
/// each boundary vertex's soup duplicates so a pinned corner stays pinned in
/// every slot. Scan borders and hole rims are boundaries; pinning them stops
/// Smooth and the auto-relax from eroding the scan's edge.
fn boundary_mask(
    indices: &[u32],
    position_siblings: &[Vec<usize>],
    vertex_count: usize,
) -> Vec<bool> {
    let mut edge_uses: HashMap<(u32, u32), u32> = HashMap::with_capacity(indices.len());
    for triangle in indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = if a <= b { (a, b) } else { (b, a) };
            *edge_uses.entry(key).or_insert(0) += 1;
        }
    }
    let mut is_boundary = vec![false; vertex_count];
    for ((a, b), uses) in edge_uses {
        // Open boundary (1) OR non-manifold flap (>=3): both lack a well-defined
        // pair of sides, so pin them rather than average across an undefined gap.
        if uses != 2 {
            for raw in [a, b] {
                if let Some(id) = usize::try_from(raw).ok().filter(|&i| i < vertex_count) {
                    is_boundary[id] = true;
                }
            }
        }
    }
    for vertex_id in 0..vertex_count {
        if is_boundary[vertex_id] {
            for &sibling in &position_siblings[vertex_id] {
                if sibling < vertex_count {
                    is_boundary[sibling] = true;
                }
            }
        }
    }
    is_boundary
}

/// Shortest edge from `here` to any of `neighbors`' positions, capped (not
/// floored) at 1mm so a single dab cannot take an oversized jump on a
/// sparse/low-poly mesh. A genuinely SMALL edge is returned unfloored: a fine
/// occlusal groove or margin line can have real neighbor spacing well under a
/// coarse floor, and flooring it would inflate `clamp_step`'s budget past what
/// that local topology can tolerate, defeating the anti-inversion guard. An
/// isolated vertex (no finite neighbor distance) falls back to a generous
/// budget so its step is never zero-clamped by a topology fluke.
pub(crate) fn shortest_incident_edge(positions: &[Vec3], neighbors: &[usize], here: Vec3) -> f32 {
    let shortest = neighbors
        .iter()
        .filter_map(|&neighbor| positions.get(neighbor))
        .map(|&position| position.distance(here))
        .filter(|length| length.is_finite() && *length > 0.0)
        .fold(f32::MAX, f32::min);
    if shortest == f32::MAX {
        return 1.0;
    }
    shortest.min(1.0)
}

/// Smooth radial falloff: 1 at the center, 0 at/beyond `radius`, `C1`-smooth at
/// the boundary (squared sculpting falloff).
fn falloff(distance: f32, radius: f32) -> f32 {
    if !(distance.is_finite() && radius.is_finite()) || radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    let t = (1.0 - distance / radius).clamp(0.0, 1.0);
    t * t
}
