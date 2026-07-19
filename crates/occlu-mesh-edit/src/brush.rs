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
//! flip the push direction. Each dab is followed by an auto-smooth
//! (uniform-Laplacian) pass so material builds and carves CLEAN — ironing out
//! the scan's own surface noise and evening the triangulation — instead of just
//! lifting the raw noisy surface (the "ripple" a pure push leaves). Smooth is
//! the same relaxer run harder, as several whole passes (fractional Taubin per
//! frame is imperceptible — the reason the old smooth did nothing). Both pin
//! open scan boundaries so the scan's outer edge never erodes, and a dab only
//! ever affects the connected component under the cursor, never dragging a
//! spatially-close but disconnected surface along with it.
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
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

use super::brush_csr::Csr;
use super::brush_index::VertexGrid;
use super::brush_math::{
    boundary_mask, compute_step_budget, falloff, is_single_component, scope_area_normals,
    smooth_pass_count,
};
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
/// Add/Remove displacement per fully-weighted dab, as a fraction of the brush
/// radius. Scaling by radius (not a fixed mm) keeps the brush feeling the same
/// on a coarse or a fine scan and at any zoom; buildup accumulates over the many
/// arc-length-spaced dabs of a drag, so this stays small per dab.
const ADD_REMOVE_GAIN: f32 = 0.08;
/// Strength of the auto-smooth Laplacian pass that follows every Add/Remove
/// dab: how far each brushed vertex moves toward its ring centroid, ironing out
/// the scan's surface noise so material builds and carves clean instead of
/// rippled. High enough to visibly de-noise, below the level that would erase
/// the coherent sculpted dome.
const AUTOSMOOTH_FACTOR: f32 = 0.6;
/// Auto-smooth passes per Add/Remove dab. Two passes leave the built/carved
/// area genuinely CLEAN (no residual scan ripple) rather than merely softened,
/// while still letting material build; the coherent push runs first, so this
/// de-noises without flattening the dome.
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
/// Grid cells spanned by one brush radius. The grid's cell size is kept at
/// `radius / this` so a radius query only ever scans a small, bounded cube of
/// cells regardless of how the brush size compares to the mesh scale — the fix
/// for a big brush stuttering as it scanned millions of empty cells.
const GRID_CELLS_ACROSS_RADIUS: f32 = 4.0;

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
/// update instead of re-uploading the whole mesh. Deduplicated but NOT sorted
/// (an interactive caller sorts the whole frame's union once); indices into the
/// ORIGINAL vertex array `BrushSession::prepare` was built from.
#[derive(Clone, Debug, Default)]
pub struct BrushStrokeOutcome {
    /// Touched vertex ids, unique, in first-touched order.
    pub touched_vertices: Vec<usize>,
}

/// A prepared freeform-sculpting session over one mesh. See the module docs
/// for the amortized-cost shape and soup-correctness contract.
pub struct BrushSession {
    /// Original vertex attributes (color/uv kept verbatim; position/normal
    /// updated in place as dabs apply). Same length and order as the mesh
    /// `BrushSession::prepare` was built from.
    vertices: Vec<EditVertex>,
    /// Dense struct-of-arrays mirror of `vertices`' positions, kept in sync by
    /// [`Self::set_position`]. Every hot pass gathers positions by scattered
    /// vertex id; reading them from this 12-byte-per-vertex array instead of the
    /// ~40-byte interleaved `EditVertex` cuts the cache traffic that dominates a
    /// big-brush dab roughly threefold.
    positions: Vec<Vec3>,
    /// The ORIGINAL (unwelded) triangle indices — returned verbatim by `finish`,
    /// since brush dabs only move vertices, never retopologize.
    indices: Vec<u32>,
    /// Vertex-vertex adjacency over the WELDED topology (shared corners see
    /// their true neighbors even across soup duplicates), as CSR so a one-ring
    /// gather in the hot passes is a contiguous slice, not a per-vertex heap
    /// chase. A soup duplicate that is not the weld representative has an empty
    /// ring; it is moved by sibling propagation from the representative.
    adjacency: Csr,
    /// Per-ORIGINAL-vertex incident triangle indices (into `indices`), as CSR;
    /// used to recompute normals scoped to the touched region.
    incident_triangles: Csr,
    /// Every other original vertex id that started at the exact same position
    /// as this one (soup duplicates of one physical corner), as CSR; empty row
    /// for a vertex with no duplicate.
    position_siblings: Csr,
    /// Whether a vertex sits on an open scan boundary (an edge used by only one
    /// triangle). Boundary vertices are pinned by Smooth and by the auto-relax
    /// so the scan's outer edge and any hole rims never erode.
    is_boundary: Vec<bool>,
    /// Whether the whole mesh is one connected surface. When true, a dab can
    /// skip the per-dab connected-component flood fill (there is nothing else
    /// to avoid dragging along) — the common single-scan case.
    single_component: bool,
    /// Shortest welded-neighbor edge length per vertex, captured at prepare
    /// time — the anti-inversion guard's per-vertex step budget.
    max_step: Vec<f32>,
    /// Spatial index over vertex positions. Its cell size is matched to the
    /// current brush radius (see [`Self::sync_grid`]) so a query's cell-scan
    /// stays cheap for any brush size; moved vertices are relocated in it
    /// incrementally by [`Self::set_position`], so it never needs an O(n)
    /// drift rebuild mid-stroke.
    grid: VertexGrid,
    /// The brush radius the grid's cell size is currently tuned for; a big
    /// change (a size-slider adjustment) triggers a rebuild.
    grid_radius: f32,
    /// Reusable `(vertex id, pre-dab position)` list for one-shot grid
    /// maintenance. The spatial grid is only READ at the start of the next dab,
    /// never during this dab's sub-passes, so instead of relocating every vertex
    /// in the grid on every one of a Smooth dab's many passes, the movable region
    /// is snapshotted once at dab start and relocated once at dab end.
    grid_dirty: Vec<(usize, Vec3)>,
    /// Reusable visited-stamp buffer for the per-dab component flood fill, the
    /// touched-set dedup, and the normal-recompute scope build, so none of them
    /// allocates a fresh `HashSet` per dab (the cost that made a big brush
    /// stutter on a million-triangle scan). A slot is "touched this pass" when
    /// its stamp equals `stamp_generation`; clearing is a generation bump.
    component_stamp: Vec<u32>,
    /// Current generation for the stamp buffer.
    stamp_generation: u32,
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
        // Snapshot the movable region's pre-dab positions so the spatial grid can
        // be brought up to date in a SINGLE pass at the end of the dab, rather
        // than per-vertex on every one of a Smooth dab's many relax passes.
        self.snapshot_grid_region(&weighted);
        let mut touched: Vec<usize> = Vec::new();
        match mode {
            BrushMode::Smooth => self.apply_smooth(&weighted, strength, &mut touched),
            BrushMode::Add => self.apply_clay(&weighted, stroke, 1.0, &mut touched),
            BrushMode::Remove => self.apply_clay(&weighted, stroke, -1.0, &mut touched),
        }
        // Fold every vertex's net motion back into the grid once (no-op for the
        // coherent centre that never left its cell), keeping the next dab's
        // radius query exact.
        self.apply_grid_maintenance();
        if touched.is_empty() {
            return BrushStrokeOutcome::default();
        }
        // Dedup via a stamp (no sort): `touched` carries duplicates (a vertex
        // moved by both the displacement and the auto-smooth pass, plus soup
        // siblings), and a per-dab sort of tens of thousands of ids was a real
        // cost on a big brush. The interactive caller sorts the whole frame's
        // union once before the GPU write; order here is irrelevant.
        let unique_generation = self.next_stamp();
        let mut unique = Vec::with_capacity(touched.len());
        for &vertex_id in &touched {
            if self.component_stamp[vertex_id] != unique_generation {
                self.component_stamp[vertex_id] = unique_generation;
                unique.push(vertex_id);
            }
        }
        // Sync the interleaved vertex positions from the SoA mirror once, for
        // exactly the touched slots — the hot passes only wrote the mirror. The
        // caller reads `vertices()` for these ids to patch its GPU shadow, and
        // `finish` bakes them, so both see the final positions.
        for &vertex_id in &unique {
            self.vertices[vertex_id].position = self.positions[vertex_id].to_array();
        }
        self.touched_total.extend(unique.iter().copied());
        self.recompute_normals_near(&unique);
        BrushStrokeOutcome {
            touched_vertices: unique,
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
        self.sync_grid(stroke.radius_mm);
        let center = Vec3::from_array(stroke.center);
        let candidates = self.grid.query_radius(center, stroke.radius_mm);
        if candidates.is_empty() {
            return None;
        }
        // Parallel across the candidate set — a big brush has tens of thousands
        // of candidates and this (plus the displacement/relax maps below) is the
        // per-vertex work that dominates a dab. `par_iter().collect()` preserves
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

    /// Keep only the candidates in the same connected component as the vertex
    /// nearest the dab center, by flooding the welded rings (and soup siblings)
    /// through the in-disc set. A purely Euclidean radius query can pull in a
    /// spatially-close but topologically SEPARATE surface (a dropout island, the
    /// opposing arch sitting just behind the cursor); restricting to one
    /// component stops a dab from dragging two disjoint sheets together — the
    /// robustness a messy multi-surface scan needs, and what makes the "never
    /// reaches across a gap" contract true for the clay push, not only Smooth.
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

    /// Keep the spatial grid usable for a dab of the given radius: rebuild it
    /// (from live positions, with a cell size matched to the radius) when the
    /// brush radius has changed enough that the old cell size would make the
    /// query's cell scan too coarse or too fine, OR when vertices have drifted
    /// far enough that a query could miss a moved one. Sizing the cell to the
    /// radius is what keeps a big brush from scanning millions of empty cells.
    fn sync_grid(&mut self, radius: f32) {
        // ONLY a brush-radius change (which changes the cell size) forces a full
        // rebuild — a rare, deliberate size-slider move. Vertex motion during a
        // stroke is tracked incrementally in `set_position`, so there is no
        // per-dab O(n) drift rebuild (the stall a big scan showed).
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

    /// Clay Add (`sign = +1`) / Remove (`sign = -1`): displace the whole brushed
    /// region coherently along one camera-oriented brush normal, then run an
    /// auto-smooth pass so material is built or carved CLEAN instead of just
    /// lifting the scan's own surface noise (the "ripple" left by a pure push).
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
        // Only weld representatives (a real ring) are displaced independently;
        // their soup duplicates follow via sibling propagation in `commit_moves`.
        // Letting a duplicate also displace on its own would let it re-apply the
        // dab from the already-moved position against its own (fallback) budget
        // and overrun the representative's correct clamp.
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

        // Auto-smooth: several uniform-Laplacian relax passes over the brushed
        // region, so the built/carved surface comes out genuinely clean rather
        // than the scan's raw noise pushed up (or down) wholesale. The coherent
        // push already made a smooth dome, so these irons out the high-frequency
        // scan grain and even the triangulation without collapsing the sculpted
        // volume; boundary and needle-tip vertices are left alone.
        for _ in 0..CLAY_AUTOSMOOTH_PASSES {
            self.relax_pass(weighted, AUTOSMOOTH_FACTOR, touched);
        }
    }

    /// Smooth: several whole uniform-Laplacian passes (count from `strength`, so
    /// a firmer press or the forced Shift mode simply runs more passes), each
    /// blended by the per-vertex falloff, boundary and needle-tip vertices left
    /// alone so scan edges hold.
    fn apply_smooth(&mut self, weighted: &[(usize, f32)], strength: f32, touched: &mut Vec<usize>) {
        for _ in 0..smooth_pass_count(strength) {
            self.relax_pass(weighted, SMOOTH_LAMBDA, touched);
        }
    }

    /// One uniform-Laplacian pass: move each relaxable candidate a
    /// `factor`-and-falloff fraction toward its ring centroid, then commit.
    /// Reads pre-pass positions (computed in parallel) so the pass is
    /// iteration-order-independent. Skips boundary and low-valence vertices.
    fn relax_pass(&mut self, weighted: &[(usize, f32)], factor: f32, touched: &mut Vec<usize>) {
        let proposals: Vec<(usize, Vec3)> = weighted
            .par_iter()
            .filter(|&&(vertex_id, _)| self.is_relaxable(vertex_id))
            .filter_map(|&(vertex_id, weight)| {
                let here = self.position(vertex_id);
                let centroid = self.ring_centroid(vertex_id)?;
                let target = here.lerp(centroid, (factor * weight).clamp(0.0, 1.0));
                let clamped = self.clamp_step(vertex_id, target);
                (clamped != here).then_some((vertex_id, clamped))
            })
            .collect();
        self.commit_moves(proposals.into_iter(), touched);
    }

    /// Whether a vertex may be relaxed/smoothed: interior (not an open-boundary
    /// vertex) and with a real one-ring (a needle tip is left frozen).
    fn is_relaxable(&self, vertex_id: usize) -> bool {
        !self.is_boundary[vertex_id] && self.adjacency.row_len(vertex_id) >= MIN_RING_FOR_RELAX
    }

    /// The camera-oriented brush normal: bucket the region's vertex normals by
    /// whether they face the camera (Blender's `calc_area_normal`), average only
    /// the toward-viewer bucket, and fall back to the pure camera direction when
    /// that bucket is too weak to trust. Robust to inverted-normal scan patches,
    /// where a naive signed average would cancel to garbage.
    fn brush_normal(&self, weighted: &[(usize, f32)], view_dir: Vec3) -> Vec3 {
        let view = view_dir.normalize_or_zero();
        let has_view = view.length_squared() > f32::EPSILON;
        // Parallel reduction: a big brush buckets tens of thousands of normals,
        // so fold the toward-camera sum, its weight, and the total weight across
        // threads rather than a serial gather over the interleaved vertices.
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

    /// Write a set of already-clamped, already-non-no-op target positions:
    /// move each vertex and its soup siblings, recording every slot touched.
    /// The anti-inversion clamp and the no-op filter are done in the parallel
    /// proposal maps that feed this, so the serial commit is a minimal scatter.
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

    /// Current (live) position of a vertex mid-session — read from the dense
    /// `positions` mirror.
    pub(crate) fn position(&self, vertex_id: usize) -> Vec3 {
        self.positions[vertex_id]
    }

    /// Write a vertex's live position into the dense `positions` mirror only —
    /// the sole source of truth every hot pass reads back. The interleaved
    /// `vertices[].position` (40-byte stride) and the spatial grid are both
    /// synced ONCE per dab afterwards, not on every one of a many-pass Smooth's
    /// scattered writes, so the hot commit touches only the compact 12-byte
    /// array.
    fn set_position(&mut self, vertex_id: usize, position: Vec3) {
        self.positions[vertex_id] = position;
    }

    /// Record the pre-dab position of every vertex the dab could move — the
    /// weighted candidates and their soup siblings — into `grid_dirty`, deduped
    /// via a stamp so each id appears once with its TRUE pre-dab position.
    fn snapshot_grid_region(&mut self, weighted: &[(usize, f32)]) {
        self.grid_dirty.clear();
        let generation = self.next_stamp();
        for &(vertex_id, _) in weighted {
            if self.component_stamp[vertex_id] != generation {
                self.component_stamp[vertex_id] = generation;
                self.grid_dirty.push((vertex_id, self.positions[vertex_id]));
            }
            for i in 0..self.position_siblings.row_len(vertex_id) {
                let sibling = self.position_siblings.row(vertex_id)[i] as usize;
                if self.component_stamp[sibling] != generation {
                    self.component_stamp[sibling] = generation;
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

    /// Recompute normals for exactly the touched vertices and their one-ring (a
    /// moved vertex changes its neighbors' face-weighted normals too), using
    /// the ORIGINAL (unwelded) incident-triangle map so every soup duplicate's
    /// own triangle is included.
    ///
    /// This is the Blender-sculpt strategy (PR #116209): recompute every
    /// affected vertex's normal DIRECTLY from its incident faces in parallel,
    /// each vertex writing only its own slot — deliberately recomputing a shared
    /// face normal once per corner rather than paying for the single-threaded
    /// `VectorSet` dedup of faces that was the real bottleneck on a big dab.
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
        let new_normals =
            scope_area_normals(&scope, &self.incident_triangles, &self.indices, &self.positions);
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
