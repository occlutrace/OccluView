//! Freeform sculpting brushes (issue #11): Smooth, Add/Remove (virtual wax
//! knife), and Drag (elastic pull), applied over a soft-falloff disc on the
//! surface — the geometry half of exocad-style freeforming applied to scan
//! meshes, not restorations.
//!
//! # Session shape
//!
//! A [`BrushSession`] is prepared ONCE when the operator arms the brush tool
//! (welds STL soup, builds adjacency/incidence/spatial grid — the one-time
//! O(n) cost an interactive drag amortizes over many strokes) and then takes
//! one [`BrushStroke`] per input frame via [`BrushSession::apply_stroke`],
//! returning only the touched vertex ids so the caller can push a PARTIAL
//! GPU buffer update instead of re-uploading the whole mesh.
//! [`BrushSession::finish`] bakes the accumulated edits into a
//! [`MeshEditResult`] for the normal undo/redo commit path (mirroring every
//! other kernel entry point).
//!
//! # Soup correctness
//!
//! STL stores each triangle's corners as independent vertices, so the vertex
//! ARRAY still has orphaned duplicates at a moved corner even after the
//! session welds INDEX topology for adjacency purposes (`weld_soup_topology`
//! rewrites triangle indices, never the vertex array). Every touched vertex's
//! new position and normal are propagated to every other vertex slot that
//! started at the exact same position (`position_siblings`), or a smoothed
//! soup scan would show a hairline crack at every touched triangle corner.
//!
//! # Guard
//!
//! v1 is displacement-only: every proposed step is clamped to a fraction of
//! the vertex's shortest incident (welded) edge, so a brush cannot invert or
//! degenerate a triangle in one stroke. Local remeshing under heavy stretch
//! is a follow-up.

use glam::Vec3;
use std::collections::{HashMap, HashSet};

use super::brush_index::VertexGrid;
use super::cap_support::build_vertex_adjacency;
use super::topology::{canonical_position_key, weld_soup_topology};
use super::{
    validate_face_edit_buffers, validate_selection_against_triangle_count, EditVertex,
    FaceSelection, MeshEditBuffers, MeshEditError, MeshEditReport, MeshEditResult, MeshTopology,
};

/// [`smooth_selected_faces`] iteration count: enough Taubin passes to visibly
/// flatten a marked seam/patch from one button click (unlike an interactive
/// brush stroke, which is deliberately gentle per frame).
const SELECTION_SMOOTH_ITERATIONS: usize = 12;
/// Extra radius beyond the selection's own bounding sphere, as a fraction of
/// it, so the falloff decays to zero PAST the marked boundary instead of at
/// it — the soft blend into untouched surrounding surface the forum request
/// ("smooth the transition area") actually asked for.
const SELECTION_SMOOTH_MARGIN: f32 = 0.35;

/// Taubin's shrink pass factor (positive: pulls each vertex toward its
/// umbrella mean).
const TAUBIN_LAMBDA: f32 = 0.5;
/// Taubin's compensating inflate pass factor (negative, `|MU| > LAMBDA`): the
/// second pass cancels the first pass's volume loss instead of the naive
/// single-pass Laplacian's steady shrink toward a point.
const TAUBIN_MU: f32 = -0.53;
/// Add/Remove step at full strength and falloff, in mm, before the per-vertex
/// edge-length guard clamps it further.
const ADD_REMOVE_STEP_MM: f32 = 0.6;
/// Largest displacement step as a fraction of a vertex's shortest incident
/// (welded) edge — the anti-inversion guard.
const MAX_STEP_FRACTION_OF_EDGE: f32 = 0.4;

/// One brush stroke: a soft-falloff disc centered on the surface.
#[derive(Copy, Clone, Debug)]
pub struct BrushStroke {
    /// World-space stroke center (typically a ray/mesh hit point).
    pub center: [f32; 3],
    /// Falloff radius in mm; zero effect at/beyond this distance.
    pub radius_mm: f32,
    /// Overall stroke strength, 0..1 (0 is a no-op; callers typically drive
    /// this from a UI slider and/or per-frame time delta).
    pub strength: f32,
}

/// Which sculpting operation a stroke performs.
#[derive(Copy, Clone, Debug)]
pub enum BrushMode {
    /// Volume-preserving (Taubin lambda/mu) relaxation: irons out noise and
    /// the "spike" seams of issue #9 without the naive-Laplacian shrink.
    Smooth,
    /// Push the surface outward along each vertex's normal (virtual wax
    /// knife, additive).
    Add,
    /// Pull the surface inward along each vertex's normal (virtual wax
    /// knife, subtractive).
    Remove,
    /// Drag every touched vertex by a world-space delta, falloff-weighted —
    /// the "pull like cloth" freeform gesture.
    Drag {
        /// World-space displacement for this stroke (frame-to-frame drag
        /// delta, not a cumulative total).
        delta: [f32; 3],
    },
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
    /// updated in place as strokes apply). Same length and order as the mesh
    /// `BrushSession::prepare` was built from.
    vertices: Vec<EditVertex>,
    /// The ORIGINAL (unwelded) triangle indices — returned verbatim by
    /// `finish`, since brush strokes only move vertices, never retopologize.
    indices: Vec<u32>,
    /// Vertex-vertex adjacency over the WELDED topology (shared corners see
    /// their true neighbors even across soup duplicates).
    adjacency: Vec<Vec<usize>>,
    /// Per-ORIGINAL-vertex incident triangle indices (into `indices`), used to
    /// recompute normals scoped to the touched region.
    incident_triangles: Vec<Vec<usize>>,
    /// Every other original vertex id that started at the exact same
    /// position as this one (soup duplicates of one physical corner); empty
    /// for a vertex with no duplicates.
    position_siblings: Vec<Vec<usize>>,
    /// Shortest welded-neighbor edge length per vertex, captured at prepare
    /// time — the anti-inversion guard's per-vertex step budget.
    max_step: Vec<f32>,
    /// Spatial index over the (fixed at prepare time) original positions.
    grid: VertexGrid,
    /// Every vertex id touched by any stroke so far this session — reported
    /// honestly as `report.moved_vertices` by `finish`.
    touched_total: HashSet<usize>,
}

impl BrushSession {
    /// Prepare a session over `mesh`: weld soup topology for adjacency,
    /// build the incidence map, the soup position-cluster map, the
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
            max_step,
            grid,
            touched_total: HashSet::new(),
        })
    }

    /// Apply one stroke, mutating touched vertex positions and normals in
    /// place. Returns exactly the touched vertex ids for a partial GPU
    /// update; empty when the stroke has no effect (zero strength/radius, or
    /// no vertex within reach).
    pub fn apply_stroke(&mut self, stroke: BrushStroke, mode: BrushMode) -> BrushStrokeOutcome {
        let strength = stroke.strength.clamp(0.0, 1.0);
        if strength <= 0.0 || !stroke.radius_mm.is_finite() || stroke.radius_mm <= 0.0 {
            return BrushStrokeOutcome::default();
        }
        let center = Vec3::from_array(stroke.center);
        let candidates = self.grid.query_radius(center, stroke.radius_mm);
        if candidates.is_empty() {
            return BrushStrokeOutcome::default();
        }

        // Falloff-weighted candidates actually inside the disc (the grid
        // query is a conservative superset).
        let weighted: Vec<(usize, f32)> = candidates
            .into_iter()
            .filter_map(|vertex_id| {
                let distance = self.position(vertex_id).distance(center);
                let weight = falloff(distance, stroke.radius_mm);
                (weight > 0.0).then_some((vertex_id, weight * strength))
            })
            .collect();
        if weighted.is_empty() {
            return BrushStrokeOutcome::default();
        }

        let proposals = match mode {
            BrushMode::Smooth => self.smooth_proposals(&weighted),
            BrushMode::Add => self.wax_knife_proposals(&weighted, 1.0),
            BrushMode::Remove => self.wax_knife_proposals(&weighted, -1.0),
            BrushMode::Drag { delta } => self.drag_proposals(&weighted, Vec3::from_array(delta)),
        };

        let mut touched: Vec<usize> = Vec::with_capacity(proposals.len() * 2);
        for (vertex_id, new_position) in proposals {
            let clamped = self.clamp_step(vertex_id, new_position);
            if clamped == self.position(vertex_id) {
                continue;
            }
            self.set_position(vertex_id, clamped);
            touched.push(vertex_id);
            let siblings = self.position_siblings[vertex_id].clone();
            for sibling in siblings {
                self.set_position(sibling, clamped);
                touched.push(sibling);
            }
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

    /// Two-pass Taubin lambda/mu relaxation, synchronous within each pass
    /// (every touched vertex reads pre-pass positions, so the result does not
    /// depend on iteration order), blended toward the original position by
    /// each vertex's own falloff weight.
    fn smooth_proposals(&self, weighted: &[(usize, f32)]) -> Vec<(usize, Vec3)> {
        let pass1: HashMap<usize, Vec3> = weighted
            .iter()
            .map(|&(vertex_id, _)| {
                let original = self.position(vertex_id);
                (
                    vertex_id,
                    self.umbrella_step(vertex_id, original, TAUBIN_LAMBDA, None),
                )
            })
            .collect();
        weighted
            .iter()
            .map(|&(vertex_id, weight)| {
                let original = self.position(vertex_id);
                // Pass 2's "here" is THIS vertex's pass-1 result, not its
                // original position — Taubin's second pass relaxes the
                // ALREADY-SHRUNK surface, which is what cancels the shrink.
                let pass1_here = pass1[&vertex_id];
                let after_pass1 =
                    self.umbrella_step(vertex_id, pass1_here, TAUBIN_MU, Some(&pass1));
                (vertex_id, original.lerp(after_pass1, weight))
            })
            .collect()
    }

    /// One umbrella-relaxation step for `vertex_id` starting from `here`:
    /// `here + factor * (mean(neighbors) - here)`. Neighbor positions come
    /// from `overrides` when present (this vertex was also moved in an
    /// earlier pass this stroke), else the session's current (pre-stroke)
    /// position.
    fn umbrella_step(
        &self,
        vertex_id: usize,
        here: Vec3,
        factor: f32,
        overrides: Option<&HashMap<usize, Vec3>>,
    ) -> Vec3 {
        let ring = &self.adjacency[vertex_id];
        if ring.is_empty() {
            return here;
        }
        let mut mean = Vec3::ZERO;
        for &neighbor in ring {
            let position = overrides
                .and_then(|map| map.get(&neighbor))
                .copied()
                .unwrap_or_else(|| self.position(neighbor));
            mean += position;
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = mean / (ring.len() as f32);
        let laplacian = mean - here;
        here + factor * laplacian
    }

    /// Add/Remove: displace along the vertex's OWN (pre-stroke) normal.
    fn wax_knife_proposals(&self, weighted: &[(usize, f32)], sign: f32) -> Vec<(usize, Vec3)> {
        weighted
            .iter()
            .map(|&(vertex_id, weight)| {
                let normal = Vec3::from_array(self.vertices[vertex_id].normal).normalize_or_zero();
                let step = sign * weight * ADD_REMOVE_STEP_MM;
                (vertex_id, self.position(vertex_id) + normal * step)
            })
            .collect()
    }

    /// Drag: displace by the stroke's world-space delta, falloff-weighted.
    fn drag_proposals(&self, weighted: &[(usize, f32)], delta: Vec3) -> Vec<(usize, Vec3)> {
        weighted
            .iter()
            .map(|&(vertex_id, weight)| (vertex_id, self.position(vertex_id) + delta * weight))
            .collect()
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

    fn position(&self, vertex_id: usize) -> Vec3 {
        Vec3::from_array(self.vertices[vertex_id].position)
    }

    fn set_position(&mut self, vertex_id: usize, position: Vec3) {
        self.vertices[vertex_id].position = position.to_array();
    }

    /// Recompute normals for exactly the touched vertices and their one-ring
    /// (a moved vertex changes its neighbors' face-weighted normals too),
    /// using the ORIGINAL (unwelded) incident-triangle map so every soup
    /// duplicate's own triangle is included.
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

    /// Bake the session into a [`MeshEditResult`]: same topology, updated
    /// vertex positions/normals, `report.moved_vertices` set to the true
    /// count of vertices touched across every stroke this session (never
    /// dirties the caller's undo history on a content no-op).
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

/// One-click Smooth over an explicit face selection (the Mesh Editor's
/// button, as opposed to an interactive brush drag): runs
/// [`SELECTION_SMOOTH_ITERATIONS`] Taubin passes over the marked region using
/// one enclosing stroke (selection centroid, radius = bounding sphere +
/// [`SELECTION_SMOOTH_MARGIN`]), so the result blends into the untouched
/// surrounding surface rather than stopping abruptly at the marked boundary —
/// directly what the "spike" seams of issue #9 need after a jagged Close
/// Holes cap, and what the forum's original smoothing request asked for.
///
/// An empty selection is a valid no-op (mirrors [`super::holes::fill_selected_holes`]):
/// it never widens into a whole-mesh smooth.
///
/// # Errors
/// Returns [`MeshEditError::UnsupportedPointCloud`], malformed-mesh, or
/// selection-length errors from the shared buffer/selection validation.
pub fn smooth_selected_faces(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
) -> Result<MeshEditResult, MeshEditError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    validate_selection_against_triangle_count(mesh.triangle_count(), selection)?;

    let Some((center, radius_mm)) = selection_enclosing_sphere(mesh, selection) else {
        return Ok(MeshEditResult {
            mesh: mesh.clone(),
            report: MeshEditReport {
                input_vertices: mesh.vertices.len(),
                input_triangles: mesh.triangle_count(),
                output_vertices: mesh.vertices.len(),
                output_triangles: mesh.triangle_count(),
                ..MeshEditReport::default()
            },
        });
    };

    let mut session = BrushSession::prepare(mesh)?;
    let stroke = BrushStroke {
        center: center.to_array(),
        radius_mm,
        strength: 1.0,
    };
    for _ in 0..SELECTION_SMOOTH_ITERATIONS {
        session.apply_stroke(stroke, BrushMode::Smooth);
    }
    Ok(session.finish())
}

/// Centroid and margin-padded bounding radius of every vertex referenced by a
/// selected triangle. `None` for an empty selection. Callers have already run
/// [`validate_selection_against_triangle_count`]/[`validate_face_edit_buffers`],
/// so an out-of-range lookup here cannot happen for a well-formed input; a
/// corner is simply skipped rather than aborting the whole computation if one
/// somehow did.
fn selection_enclosing_sphere(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
) -> Option<(Vec3, f32)> {
    let mut seen = HashSet::new();
    let mut positions = Vec::new();
    for (triangle_index, selected) in selection.as_slice().iter().enumerate() {
        if !*selected {
            continue;
        }
        let base = triangle_index * 3;
        let Some(corners) = mesh.indices.get(base..base + 3) else {
            continue;
        };
        for &raw in corners {
            let Some(vertex_id) = usize::try_from(raw).ok() else {
                continue;
            };
            if seen.insert(vertex_id) {
                if let Some(vertex) = mesh.vertices.get(vertex_id) {
                    positions.push(Vec3::from_array(vertex.position));
                }
            }
        }
    }
    if positions.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let centroid = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
    let max_radius = positions
        .iter()
        .map(|&p| p.distance(centroid))
        .fold(0.0_f32, f32::max);
    let radius = (max_radius * (1.0 + SELECTION_SMOOTH_MARGIN)).max(max_radius + 0.5);
    Some((centroid, radius))
}

/// Shortest edge from `here` to any of `neighbors`' positions; a generous
/// fallback for an isolated vertex (no neighbors) so its step budget is never
/// zero-clamped by a topology fluke.
fn shortest_incident_edge(positions: &[Vec3], neighbors: &[usize], here: Vec3) -> f32 {
    neighbors
        .iter()
        .filter_map(|&neighbor| positions.get(neighbor))
        .map(|&position| position.distance(here))
        .filter(|length| length.is_finite() && *length > 0.0)
        .fold(f32::MAX, f32::min)
        .clamp(0.05, 1.0)
}

/// Smooth radial falloff: 1 at the center, 0 at/beyond `radius`, `C1`-smooth
/// at the boundary (squared-cosine-style sculpting falloff).
fn falloff(distance: f32, radius: f32) -> f32 {
    if !(distance.is_finite() && radius.is_finite()) || radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    let t = (1.0 - distance / radius).clamp(0.0, 1.0);
    t * t
}
