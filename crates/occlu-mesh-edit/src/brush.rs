//! Freeform sculpting brushes (issue #11): an interactive Add/Remove clay knife
//! and a Smooth relaxer, applied over a soft-falloff disc on the surface — the
//! geometry half of exocad-style freeforming applied to intraoral SCAN meshes.
//!
//! # Session shape
//!
//! A [`BrushSession`] is prepared ONCE per layer (welds STL soup, builds
//! adjacency/incidence/boundary/spatial-grid — amortized over many dabs) and
//! reused across every stroke. Each dab is one [`BrushStroke`] via
//! [`BrushSession::apply_stroke`], returning only the touched vertex ids for a
//! PARTIAL GPU update. [`BrushSession::finish`] bakes the session into a
//! [`MeshEditResult`] for the batch commit path.
//!
//! # Why it stays clean (no potholes / no spikes)
//!
//! Moving each vertex along its OWN normal carves potholes (per-vertex normals
//! diverge) and spikes (a lone vertex outruns its ring). Add/Remove instead
//! moves the whole region COHERENTLY along one averaged brush normal, computed
//! Blender-sculpt style: bucket sampled normals by camera-facing, average only
//! the front bucket, fall back to the camera direction when untrustworthy — so
//! inverted-normal scan patches never flip the push direction. Each dab runs an
//! auto-smooth (uniform-Laplacian) pass so material builds/carves clean instead
//! of leaving the raw push's ripple. Smooth is the same relaxer run as several
//! whole passes (a fractional pass per frame was imperceptible). Both pin open
//! scan boundaries and restrict a dab to the connected component under the
//! cursor, never dragging in a disconnected surface.
//!
//! # Soup correctness
//!
//! STL gives each triangle corner its own vertex, so the vertex ARRAY keeps
//! orphaned duplicates at a moved corner even after welding INDEX topology for
//! adjacency (`weld_soup_topology` rewrites indices, not the vertex array).
//! Every touched vertex's new position/normal propagates to every other slot
//! that started at the same position (`position_siblings`), or a soup scan
//! would crack at each touched corner.

use glam::Vec3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

use super::brush_csr::Csr;
use super::brush_index::VertexGrid;
use super::brush_math::{
    boundary_mask, compute_step_budget, falloff, is_single_component, on_flipped_triangle,
    refresh_step_budget, scope_area_normals, smooth_pass_count, smoothstep,
};
use super::cap_support::build_vertex_adjacency;
use super::topology::{canonical_position_key, weld_soup_topology};
use super::{
    validate_face_edit_buffers, EditVertex, MeshEditBuffers, MeshEditError, MeshEditReport,
    MeshEditResult, MeshTopology,
};

/// Uniform-Laplacian factor for the Smooth tool: aggressive by design (the
/// operator asked for cardinal flattening), strength = pass count.
const SMOOTH_LAMBDA: f32 = 0.6;
/// Taubin λ/μ for the clay auto-smooth: a shrink pass then an inflate pass
/// removes grain WITHOUT the volume loss of a plain Laplacian, so a built dome
/// stays full while the scan's surface noise is ironed out.
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;
/// Add/Remove displacement per fully-weighted dab, as a fraction of brush
/// radius. Radius-relative (not fixed mm) keeps feel consistent across scan
/// scale and zoom; per-dab stays small since a drag accumulates many dabs.
const ADD_REMOVE_GAIN: f32 = 0.08;
/// Auto-smooth rim-taper width as a fraction of the radius: the grain-cleaning
/// relax is near-uniform across the interior and ramps to zero over this outer
/// band, so the built area blends in with no hard edge.
const AUTOSMOOTH_RIM_TAPER: f32 = 0.35;
/// Taubin auto-smooth pairs per Add/Remove dab.
const CLAY_AUTOSMOOTH_PASSES: usize = 2;
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
/// Passes of the post-dab inversion guard (each reverts flipped-triangle
/// vertices; one usually suffices, more resolve cascades).
const MAX_ROLLBACK_ITERS: usize = 4;
/// Grid cells spanned by one brush radius: cell size is `radius / this`, so a
/// radius query scans a small bounded cube of cells regardless of brush size
/// vs. mesh scale — fixes a big brush stuttering over millions of empty cells.
const GRID_CELLS_ACROSS_RADIUS: f32 = 4.0;

/// One brush dab: a soft-falloff disc centered on the surface.
#[derive(Copy, Clone, Debug)]
pub struct BrushStroke {
    /// Mesh-local dab center (a ray/mesh hit point transformed into the layer's
    /// own space).
    pub center: [f32; 3],
    /// Falloff radius in mesh-local mm; zero effect at/beyond this distance.
    pub radius_mm: f32,
    /// Dab strength, 0..1 (0 is a no-op). Add/Remove scale displacement by it;
    /// Smooth turns it into a pass count. Cadence is the caller's job — this is
    /// per-dab magnitude, framerate-independent when dabs are arc-length-spaced.
    pub strength: f32,
    /// Unit view direction, FROM the camera INTO the scene. Add/Remove orient
    /// the coherent brush normal toward the camera, so Add always builds
    /// toward the viewer and Remove carves away, even across a scan's
    /// inverted-normal patches. Ignored by Smooth; a zero vector falls back to
    /// the averaged surface normal's own sign.
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

/// Outcome of one [`BrushSession::apply_stroke`] call: vertex ids whose
/// position and/or normal changed, for a partial GPU update. Deduplicated but
/// NOT sorted (the caller sorts the frame's union once); indices into the
/// ORIGINAL vertex array `BrushSession::prepare` was built from.
#[derive(Clone, Debug, Default)]
pub struct BrushStrokeOutcome {
    /// Touched vertex ids, unique, in first-touched order.
    pub touched_vertices: Vec<usize>,
}

/// A prepared freeform-sculpting session over one mesh. See the module docs
/// for the amortized-cost shape and soup-correctness contract.
pub struct BrushSession {
    /// Original vertex attributes; position/normal updated in place as dabs
    /// apply. Same length/order as the prepared mesh.
    vertices: Vec<EditVertex>,
    /// Dense `SoA` mirror of `vertices`' positions — the source every hot pass
    /// reads, at 12 bytes/vertex vs. the ~40-byte interleaved `EditVertex`.
    positions: Vec<Vec3>,
    /// Original (unwelded) triangle indices, returned verbatim by `finish`.
    indices: Vec<u32>,
    /// Vertex-vertex adjacency over WELDED topology as CSR (a non-representative
    /// soup duplicate has an empty row; it moves via sibling propagation).
    adjacency: Csr,
    /// Per-vertex incident triangle indices (into `indices`) as CSR.
    incident_triangles: Csr,
    /// Other original vertex ids that started at this one's exact position (soup
    /// duplicates), as CSR; empty for a vertex with no duplicate.
    position_siblings: Csr,
    /// Whether a vertex sits on an open boundary — pinned by Smooth and the
    /// auto-relax so scan edges and hole rims never erode.
    is_boundary: Vec<bool>,
    /// Whether the mesh is one connected surface; if so a dab skips the per-dab
    /// component flood fill (the common single-scan case).
    single_component: bool,
    /// Anti-inversion step budget (shortest incident edge) per vertex, refreshed
    /// for the touched region after each dab so it tracks the moved geometry.
    max_step: Vec<f32>,
    /// Pre-dab position per vertex (movable region, stamped this generation), so
    /// the inversion guard can revert a flipped-triangle vertex.
    pre_position: Vec<Vec3>,
    /// Spatial index over positions, cell size matched to the brush radius (see
    /// [`Self::sync_grid`]); relocated incrementally, no O(n) drift rebuild.
    grid: VertexGrid,
    /// Brush radius the grid's cell size is tuned for; a big change rebuilds.
    grid_radius: f32,
    /// Reusable `(id, pre-dab position)` list for one-shot per-dab grid upkeep
    /// (the grid is only read at the next dab's start).
    grid_dirty: Vec<(usize, Vec3)>,
    /// Reusable visited-stamp buffer for the flood fill, touched dedup, and
    /// scope build — avoids a fresh `HashSet` per dab. A slot is touched when
    /// its stamp equals `stamp_generation`.
    component_stamp: Vec<u32>,
    /// Current generation for the stamp buffer.
    stamp_generation: u32,
    /// Every vertex id touched by any dab so far this session — reported as
    /// `report.moved_vertices` by `finish`.
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
        let adjacency = Csr::from_rows(&build_vertex_adjacency(adjacency_source));

        let vertex_count = mesh.vertices.len();
        // Incident triangles, built straight into CSR via a counting sort over
        // the triangle corners — no per-vertex `Vec` allocation.
        let incident_triangles = Csr::from_pairs(
            vertex_count,
            mesh.indices
                .chunks_exact(3)
                .enumerate()
                .flat_map(move |(triangle_index, triangle)| {
                    triangle.iter().filter_map(move |&raw| {
                        usize::try_from(raw)
                            .ok()
                            .filter(|&i| i < vertex_count)
                            .map(|i| (i, triangle_index))
                    })
                }),
        );

        // Soup position clusters → sibling rows → CSR. Bare (unwelded) duplicates
        // of one physical corner share a position; each lists the others.
        let mut clusters: HashMap<[u32; 3], Vec<usize>> = HashMap::with_capacity(vertex_count);
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            clusters
                .entry(canonical_position_key(vertex.position))
                .or_default()
                .push(index);
        }
        let mut sibling_rows: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
        for group in clusters.values() {
            if group.len() < 2 {
                continue;
            }
            for &vertex_id in group {
                sibling_rows[vertex_id] = group
                    .iter()
                    .copied()
                    .filter(|&id| id != vertex_id)
                    .collect();
            }
        }
        let position_siblings = Csr::from_rows(&sibling_rows);

        let is_boundary =
            boundary_mask(&adjacency_source.indices, &position_siblings, vertex_count);
        let single_component = is_single_component(&adjacency, &position_siblings, vertex_count);

        let positions: Vec<Vec3> = mesh
            .vertices
            .iter()
            .map(|v| Vec3::from_array(v.position))
            .collect();
        let max_step = compute_step_budget(&positions, &adjacency, &position_siblings);
        let grid = VertexGrid::build(&positions);

        Ok(Self {
            vertices: mesh.vertices.clone(),
            positions,
            indices: mesh.indices.clone(),
            adjacency,
            incident_triangles,
            position_siblings,
            is_boundary,
            single_component,
            max_step,
            pre_position: vec![Vec3::ZERO; vertex_count],
            grid,
            // 0 forces the first dab to size the grid to its actual radius.
            grid_radius: 0.0,
            grid_dirty: Vec::new(),
            component_stamp: vec![0; vertex_count],
            stamp_generation: 0,
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
        // Snapshot the movable region's pre-dab positions so the grid updates in
        // a SINGLE pass at dab end, not per-vertex on every relax pass.
        self.snapshot_grid_region(&weighted);
        let mut touched: Vec<usize> = Vec::new();
        match mode {
            BrushMode::Smooth => self.apply_smooth(&weighted, strength, &mut touched),
            BrushMode::Add => self.apply_clay(&weighted, stroke, 1.0, &mut touched),
            BrushMode::Remove => self.apply_clay(&weighted, stroke, -1.0, &mut touched),
        }
        // Guarantee no triangle ends this dab flipped vs its pre-dab
        // orientation (the edge clamp is only a heuristic). Runs while the
        // snapshot generation still marks the moved region.
        self.rollback_inversions();
        // Fold every vertex's net motion back into the grid once (a no-op for a
        // vertex that never left its cell), keeping the next query exact.
        self.apply_grid_maintenance();
        if touched.is_empty() {
            return BrushStrokeOutcome::default();
        }
        // Dedup via a stamp (no sort): `touched` has duplicates (displacement +
        // auto-smooth + soup siblings), and sorting tens of thousands of ids per
        // dab was a real cost. The caller sorts the frame's union once instead.
        let unique_generation = self.next_stamp();
        let mut unique = Vec::with_capacity(touched.len());
        for &vertex_id in &touched {
            if self.component_stamp[vertex_id] != unique_generation {
                self.component_stamp[vertex_id] = unique_generation;
                unique.push(vertex_id);
            }
        }
        // Sync interleaved vertex positions from the SoA mirror once, for
        // exactly the touched slots — the hot passes only wrote the mirror.
        // Both `vertices()` and `finish` read from here afterward.
        for &vertex_id in &unique {
            self.vertices[vertex_id].position = self.positions[vertex_id].to_array();
        }
        self.touched_total.extend(unique.iter().copied());
        refresh_step_budget(
            &unique,
            &self.positions,
            &self.adjacency,
            &self.position_siblings,
            &mut self.max_step,
        );
        self.recompute_normals_near(&unique);
        BrushStrokeOutcome {
            touched_vertices: unique,
        }
    }

    /// Falloff-weighted vertices within the dab's disc (the grid query is a
    /// conservative superset, filtered here to those actually inside radius).
    /// Weights are raw spatial falloff (0..1); clamped strength is returned
    /// separately so Smooth turns it into a pass count, not a magnitude.
    /// `None` for a no-effect dab.
    fn weighted_candidates(&mut self, stroke: BrushStroke) -> Option<(Vec<(usize, f32)>, f32)> {
        let strength = stroke.strength.clamp(0.0, 1.0);
        if strength <= 0.0 || !stroke.radius_mm.is_finite() || stroke.radius_mm <= 0.0 {
            return None;
        }
        self.sync_grid(stroke.radius_mm);
        let center = Vec3::from_array(stroke.center);
        let candidates = self.grid.query_radius(center, stroke.radius_mm);
        if candidates.is_empty() {
            return None;
        }
        // Parallel across candidates — a big brush has tens of thousands, and
        // this dominates a dab's per-vertex work. `par_iter().collect()` keeps
        // order, so the result stays deterministic.
        let weighted: Vec<(usize, f32)> = candidates
            .into_par_iter()
            .filter_map(|vertex_id| {
                let distance = self.position(vertex_id).distance(center);
                let weight = falloff(distance, stroke.radius_mm);
                (weight > 0.0).then_some((vertex_id, weight))
            })
            .collect();
        // A single-surface scan (the common case) has no other component to
        // drag along, so skip the per-dab flood fill entirely.
        let weighted = if self.single_component {
            weighted
        } else {
            self.restrict_to_component(weighted, center)
        };
        (!weighted.is_empty()).then_some((weighted, strength))
    }

    /// Keep only candidates in the same connected component as the vertex
    /// nearest the dab center, by flooding welded rings (and soup siblings)
    /// through the in-disc set. A Euclidean radius query can pull in a
    /// spatially-close but topologically SEPARATE surface (a dropout island,
    /// the opposing arch behind the cursor); this stops a dab from dragging
    /// two disjoint sheets together, for Add/Remove as well as Smooth.
    fn restrict_to_component(
        &mut self,
        weighted: Vec<(usize, f32)>,
        center: Vec3,
    ) -> Vec<(usize, f32)> {
        if weighted.len() <= 1 {
            return weighted;
        }
        let Some(seed) = weighted
            .iter()
            .min_by(|a, b| {
                let da = self.position(a.0).distance(center);
                let db = self.position(b.0).distance(center);
                da.total_cmp(&db)
            })
            .map(|&(id, _)| id)
        else {
            return weighted;
        };
        // Two generations off one reusable stamp buffer, no per-dab allocation:
        // `in_disc` marks the candidate set, `reached` marks the flood fill.
        let in_disc = self.next_stamp();
        for &(id, _) in &weighted {
            self.component_stamp[id] = in_disc;
        }
        let reached = self.next_stamp();
        let mut stack = vec![seed];
        self.component_stamp[seed] = reached;
        // The CSR rows borrow `self.adjacency`/`self.position_siblings`, disjoint
        // from the `self.component_stamp` we stamp, so a plain iterator is fine.
        while let Some(vertex_id) = stack.pop() {
            for &neighbor in self.adjacency.row(vertex_id) {
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] == in_disc {
                    self.component_stamp[neighbor] = reached;
                    stack.push(neighbor);
                }
            }
            for &neighbor in self.position_siblings.row(vertex_id) {
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] == in_disc {
                    self.component_stamp[neighbor] = reached;
                    stack.push(neighbor);
                }
            }
        }
        weighted
            .into_iter()
            .filter(|&(id, _)| self.component_stamp[id] == reached)
            .collect()
    }

    /// Hand out the next stamp generation, resetting the stamp buffer on the
    /// rare `u32` wrap so a stale stamp can never masquerade as the current one.
    fn next_stamp(&mut self) -> u32 {
        self.stamp_generation = self.stamp_generation.wrapping_add(1);
        if self.stamp_generation == 0 {
            self.component_stamp.iter_mut().for_each(|s| *s = 0);
            self.stamp_generation = 1;
        }
        self.stamp_generation
    }

    /// Keep the spatial grid usable for a dab of `radius`: rebuild it (from
    /// live positions, cell size matched to radius) only when the brush radius
    /// changed enough to make the old cell size too coarse or fine — sized to
    /// radius so a big brush never scans millions of empty cells.
    fn sync_grid(&mut self, radius: f32) {
        // ONLY a brush-radius change (which changes cell size) forces a
        // rebuild — a rare, deliberate size-slider move. Motion during a
        // stroke is tracked incrementally, not by a per-dab O(n) rebuild
        // (the stall a big scan showed).
        if self.grid_radius > 0.0 && (0.6..=1.7).contains(&(radius / self.grid_radius)) {
            return;
        }
        let desired_cell = (radius / GRID_CELLS_ACROSS_RADIUS).max(f32::MIN_POSITIVE);
        let positions: Vec<Vec3> = self
            .vertices
            .iter()
            .map(|v| Vec3::from_array(v.position))
            .collect();
        self.grid = VertexGrid::build_with_cell_size(&positions, desired_cell);
        self.grid_radius = radius;
    }

    /// Clay Add (`sign = +1`) / Remove (`sign = -1`): displace the brushed
    /// region coherently along one camera-oriented brush normal, then
    /// auto-smooth so material builds/carves CLEAN instead of lifting the
    /// scan's own surface noise (the "ripple" left by a pure push).
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
        // Only weld representatives (a real ring) displace independently; their
        // soup duplicates follow via sibling propagation in `commit_moves`. A
        // duplicate displacing on its own would re-apply the dab against its
        // own fallback budget and overrun the representative's clamp.
        let displacement: Vec<(usize, Vec3)> = weighted
            .par_iter()
            .filter(|&&(vertex_id, _)| !self.adjacency.is_empty_row(vertex_id))
            .filter_map(|&(vertex_id, weight)| {
                let here = self.position(vertex_id);
                let clamped =
                    self.clamp_step(vertex_id, here + normal * (sign * weight * amplitude));
                (clamped != here).then_some((vertex_id, clamped))
            })
            .collect();
        self.commit_moves(displacement.into_iter(), touched);

        // De-noise the whole dab, not just its peak: the displacement's t²
        // falloff is too concentrated to clean the mid-radius (where the uneven
        // anti-inversion clamp leaves grain), so auto-smooth uses a plateau
        // weight — near-uniform across the interior, tapered to zero at the rim.
        // Taubin (shrink+inflate) cleans the grain without collapsing the dome.
        let smooth_weights: Vec<(usize, f32)> = weighted
            .iter()
            .map(|&(vertex_id, weight)| {
                let t = weight.sqrt(); // t = 1 - distance/radius
                (vertex_id, smoothstep(AUTOSMOOTH_RIM_TAPER, t))
            })
            .collect();
        self.taubin_smooth(&smooth_weights, CLAY_AUTOSMOOTH_PASSES, touched);
    }

    /// Smooth: aggressive uniform-Laplacian relaxation, pass count from
    /// `strength` (Shift forces the max) — cardinal flattening. Boundary and
    /// needle-tip vertices are left alone.
    fn apply_smooth(&mut self, weighted: &[(usize, f32)], strength: f32, touched: &mut Vec<usize>) {
        for _ in 0..smooth_pass_count(strength) {
            self.relax_pass(weighted, SMOOTH_LAMBDA, touched);
        }
    }

    /// One Laplacian pass: move each relaxable candidate a `factor`-and-falloff
    /// fraction toward its ring centroid, then commit. A NEGATIVE `factor` moves
    /// AWAY from the centroid — the inflate half of a Taubin pair. Reads pre-pass
    /// positions (computed in parallel) so the pass is order-independent; skips
    /// boundary and low-valence vertices.
    fn relax_pass(&mut self, weighted: &[(usize, f32)], factor: f32, touched: &mut Vec<usize>) {
        let proposals: Vec<(usize, Vec3)> = weighted
            .par_iter()
            .filter(|&&(vertex_id, _)| self.is_relaxable(vertex_id))
            .filter_map(|&(vertex_id, weight)| {
                let here = self.position(vertex_id);
                let centroid = self.ring_centroid(vertex_id)?;
                let target = here.lerp(centroid, (factor * weight).clamp(-1.0, 1.0));
                let clamped = self.clamp_step(vertex_id, target);
                (clamped != here).then_some((vertex_id, clamped))
            })
            .collect();
        self.commit_moves(proposals.into_iter(), touched);
    }

    /// `pairs` Taubin iterations: each a shrink pass (λ) then an inflate pass
    /// (μ), removing surface noise while preserving volume and features.
    fn taubin_smooth(&mut self, weighted: &[(usize, f32)], pairs: usize, touched: &mut Vec<usize>) {
        for _ in 0..pairs {
            self.relax_pass(weighted, TAUBIN_LAMBDA, touched);
            self.relax_pass(weighted, TAUBIN_MU, touched);
        }
    }

    /// Whether a vertex may be relaxed/smoothed: interior (not an open-boundary
    /// vertex) and with a real one-ring (a needle tip is left frozen).
    fn is_relaxable(&self, vertex_id: usize) -> bool {
        !self.is_boundary[vertex_id] && self.adjacency.row_len(vertex_id) >= MIN_RING_FOR_RELAX
    }

    /// Camera-oriented brush normal: bucket region vertex normals by camera-
    /// facing (Blender's `calc_area_normal`), average only the toward-viewer
    /// bucket, falling back to the camera direction when too weak to trust —
    /// robust to inverted-normal patches where a naive signed average cancels.
    fn brush_normal(&self, weighted: &[(usize, f32)], view_dir: Vec3) -> Vec3 {
        let view = view_dir.normalize_or_zero();
        let has_view = view.length_squared() > f32::EPSILON;
        // Parallel reduction: a big brush buckets tens of thousands of normals,
        // folding the toward-camera sum/weight/total across threads.
        let (toward, toward_weight, total_weight) = weighted
            .par_iter()
            .map(|&(vertex_id, weight)| {
                let normal = Vec3::from_array(self.vertices[vertex_id].normal).normalize_or_zero();
                if normal.length_squared() <= f32::EPSILON {
                    return (Vec3::ZERO, 0.0_f32, 0.0_f32);
                }
                if !has_view || normal.dot(view) <= 0.0 {
                    (normal * weight, weight, weight)
                } else {
                    (Vec3::ZERO, 0.0, weight)
                }
            })
            .reduce(
                || (Vec3::ZERO, 0.0_f32, 0.0_f32),
                |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2),
            );
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
        let ring = self.adjacency.row(vertex_id);
        if ring.is_empty() {
            return None;
        }
        let mut mean = Vec3::ZERO;
        for &neighbor in ring {
            mean += self.position(neighbor as usize);
        }
        #[allow(clippy::cast_precision_loss)]
        Some(mean / ring.len() as f32)
    }

    /// Write already-clamped, already-non-no-op target positions: move each
    /// vertex and its soup siblings, recording every touched slot. Clamping
    /// and the no-op filter happen upstream in the parallel proposal maps, so
    /// this serial commit is a minimal scatter.
    fn commit_moves(
        &mut self,
        moves: impl Iterator<Item = (usize, Vec3)>,
        touched: &mut Vec<usize>,
    ) {
        for (vertex_id, target) in moves {
            self.set_position(vertex_id, target);
            touched.push(vertex_id);
            let sibling_count = self.position_siblings.row_len(vertex_id);
            for sibling_index in 0..sibling_count {
                let sibling = self.position_siblings.row(vertex_id)[sibling_index] as usize;
                self.set_position(sibling, target);
                touched.push(sibling);
            }
        }
    }

    /// Clamp a step to [`MAX_STEP_FRACTION_OF_EDGE`] of the shortest incident
    /// edge — the anti-inversion guard.
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

    /// Current (live) vertex attributes mid-session — callers copy the touched
    /// ids into their display buffer for a partial GPU update.
    #[must_use]
    pub fn vertices(&self) -> &[EditVertex] {
        &self.vertices
    }

    /// Current (live) position of a vertex, from the dense `positions` mirror.
    pub(crate) fn position(&self, vertex_id: usize) -> Vec3 {
        self.positions[vertex_id]
    }

    /// Write a live position into the `positions` mirror only — the source every
    /// hot pass reads. `vertices[].position` and the grid are synced once per dab.
    fn set_position(&mut self, vertex_id: usize, position: Vec3) {
        self.positions[vertex_id] = position;
    }

    /// Record each movable vertex's pre-dab position (weighted candidates + soup
    /// siblings) into `grid_dirty` and `pre_position`, deduped via a stamp.
    fn snapshot_grid_region(&mut self, weighted: &[(usize, f32)]) {
        self.grid_dirty.clear();
        let generation = self.next_stamp();
        for &(vertex_id, _) in weighted {
            if self.component_stamp[vertex_id] != generation {
                self.component_stamp[vertex_id] = generation;
                self.pre_position[vertex_id] = self.positions[vertex_id];
                self.grid_dirty.push((vertex_id, self.positions[vertex_id]));
            }
            for i in 0..self.position_siblings.row_len(vertex_id) {
                let sibling = self.position_siblings.row(vertex_id)[i] as usize;
                if self.component_stamp[sibling] != generation {
                    self.component_stamp[sibling] = generation;
                    self.pre_position[sibling] = self.positions[sibling];
                    self.grid_dirty.push((sibling, self.positions[sibling]));
                }
            }
        }
    }

    /// Relocate every snapshotted vertex from its pre-dab cell to its final one
    /// in a single pass (a within-cell move is a cheap no-op), so the grid is
    /// exact for the next dab's query without a per-pass or O(n) rebuild.
    fn apply_grid_maintenance(&mut self) {
        let dirty = std::mem::take(&mut self.grid_dirty);
        for &(vertex_id, previous) in &dirty {
            self.grid
                .relocate(vertex_id, previous, self.positions[vertex_id]);
        }
        self.grid_dirty = dirty;
    }

    /// Revert every moved vertex that sits on a triangle flipped vs its pre-dab
    /// orientation, iterating until none remain. Reverting a vertex to its
    /// pre-dab position restores all its incident triangles to known-good
    /// geometry, so the mesh never accumulates an inversion the clamp missed.
    fn rollback_inversions(&mut self) {
        let generation = self.stamp_generation;
        let dirty = std::mem::take(&mut self.grid_dirty);
        for _ in 0..MAX_ROLLBACK_ITERS {
            let to_revert: Vec<usize> = dirty
                .par_iter()
                .filter(|&&(vertex_id, _)| {
                    on_flipped_triangle(
                        vertex_id,
                        generation,
                        &self.incident_triangles,
                        &self.indices,
                        &self.positions,
                        &self.pre_position,
                        &self.component_stamp,
                    )
                })
                .map(|&(vertex_id, _)| vertex_id)
                .collect();
            if to_revert.is_empty() {
                break;
            }
            for vertex_id in to_revert {
                self.positions[vertex_id] = self.pre_position[vertex_id];
            }
        }
        self.grid_dirty = dirty;
    }

    /// Recompute normals for the touched vertices and their one-ring, each
    /// affected vertex reading its own incident faces in parallel (Blender-
    /// sculpt PR #116209 — no single-threaded face dedup).
    fn recompute_normals_near(&mut self, touched: &[usize]) {
        // Build the scope (touched + welded rings + soup siblings) deduped via a
        // stamp — index loops, no sort, no allocation churn on a big brush.
        let scope_generation = self.next_stamp();
        let mut scope: Vec<usize> = Vec::with_capacity(touched.len() * 4);
        for &vertex_id in touched {
            if self.component_stamp[vertex_id] != scope_generation {
                self.component_stamp[vertex_id] = scope_generation;
                scope.push(vertex_id);
            }
            for &neighbor in self.adjacency.row(vertex_id) {
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] != scope_generation {
                    self.component_stamp[neighbor] = scope_generation;
                    scope.push(neighbor);
                }
            }
            for &sibling in self.position_siblings.row(vertex_id) {
                let sibling = sibling as usize;
                if self.component_stamp[sibling] != scope_generation {
                    self.component_stamp[sibling] = scope_generation;
                    scope.push(sibling);
                }
            }
        }

        // Conflict-free parallel recompute (see `scope_area_normals`), then the
        // trivial serial normalize + write-back.
        let new_normals = scope_area_normals(
            &scope,
            &self.incident_triangles,
            &self.indices,
            &self.positions,
        );
        for (offset, &vertex_id) in scope.iter().enumerate() {
            let sum = new_normals[offset];
            if sum.length_squared() > f32::EPSILON {
                self.vertices[vertex_id].normal = sum.normalize().to_array();
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
