//! Post-cap self-intersection guard.
//!
//! A cap over a strongly curved or badly damaged rim can fold onto itself or
//! poke through nearby surface (`MeshLab`'s close-holes refuses such caps; we
//! previously emitted them silently). This module answers one question: does
//! a candidate cap PIERCE itself or the surface around its rim?
//!
//! Test model: segment-vs-triangle piercing between triangles that share no
//! vertex id, evaluated in `f64`. Pairs sharing a vertex (cap fans around rim
//! vertices, seam adjacency) legitimately touch and are skipped; strict
//! interior epsilons keep exact edge/edge contact between duplicated-position
//! vertices from counting as damage. Coplanar overlap without piercing is out
//! of scope (it is not resolvable at f32 scan precision anyway).
//!
//! Broad phase: sort by AABB min-x, sweep, and reject by full AABB overlap —
//! deterministic and allocation-light.

use std::collections::HashSet;

use glam::{DVec3, Vec3};

use super::MeshEditBuffers;

/// Strict-interior margin for the segment parameter and barycentrics: contact
/// must be measurably inside a face to count as piercing.
const PIERCE_EPSILON: f64 = 1e-9;

/// Vertex-to-incident-triangle map in CSR layout (deterministic, one
/// allocation each), built once per fill run and shared by every loop's guard.
pub(super) struct VertexTriangleIncidence {
    offsets: Vec<usize>,
    data: Vec<usize>,
}

impl VertexTriangleIncidence {
    pub(super) fn build(mesh: &MeshEditBuffers) -> Self {
        let vertex_count = mesh.vertices.len();
        let mut counts = vec![0_usize; vertex_count + 1];
        for &index in &mesh.indices {
            let vertex = index as usize;
            if vertex < vertex_count {
                counts[vertex + 1] += 1;
            }
        }
        for slot in 1..counts.len() {
            counts[slot] += counts[slot - 1];
        }
        let mut data = vec![0_usize; *counts.last().unwrap_or(&0)];
        let mut cursor = counts.clone();
        for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
            for &index in tri {
                let vertex = index as usize;
                if vertex < vertex_count {
                    data[cursor[vertex]] = triangle;
                    cursor[vertex] += 1;
                }
            }
        }
        Self {
            offsets: counts,
            data,
        }
    }

    fn triangles_of(&self, vertex: usize) -> &[usize] {
        if vertex + 1 >= self.offsets.len() {
            return &[];
        }
        &self.data[self.offsets[vertex]..self.offsets[vertex + 1]]
    }
}

/// One candidate cap in local indices: `0..rim.len()` map to the global rim
/// ids in ring order, the rest to generated vertices at
/// `generated_base + (local - rim.len())`.
pub(super) struct CapCandidate<'a> {
    /// Global rim vertex ids in ring order.
    pub(super) rim: &'a [usize],
    /// Rim vertex positions (same order).
    pub(super) rim_positions: &'a [Vec3],
    /// Generated interior positions (empty for plain caps).
    pub(super) generated: &'a [Vec3],
    /// First global id assigned to generated vertices.
    pub(super) generated_base: usize,
    /// Cap triangles in local indices.
    pub(super) triangles: &'a [[usize; 3]],
}

/// Whether the candidate cap pierces itself or the surface within two rings
/// of its rim. `ring_set` is the rim + 2-ring vertex set from the support
/// band; the incidence map turns it into the local triangle neighborhood.
pub(super) fn candidate_pierces(
    mesh: &MeshEditBuffers,
    incidence: &VertexTriangleIncidence,
    ring_set: &HashSet<usize>,
    candidate: &CapCandidate<'_>,
) -> bool {
    let local = |index: usize| -> (usize, Vec3) {
        if index < candidate.rim.len() {
            (candidate.rim[index], candidate.rim_positions[index])
        } else {
            let interior = index - candidate.rim.len();
            (
                candidate.generated_base + interior,
                candidate
                    .generated
                    .get(interior)
                    .copied()
                    .unwrap_or(Vec3::ZERO),
            )
        }
    };
    let cap: Vec<GuardTriangle> = candidate
        .triangles
        .iter()
        .map(|&[a, b, c]| {
            let corners = [local(a), local(b), local(c)];
            GuardTriangle {
                ids: corners.map(|(id, _)| id),
                positions: corners.map(|(_, position)| position),
            }
        })
        .collect();

    // Deterministic neighborhood: triangles touching the ring set, gathered
    // per vertex then sorted + deduped (HashSet iteration order cancels out).
    let mut nearby: Vec<usize> = ring_set
        .iter()
        .flat_map(|&vertex| incidence.triangles_of(vertex).iter().copied())
        .collect();
    nearby.sort_unstable();
    nearby.dedup();
    let surround: Vec<GuardTriangle> = nearby
        .into_iter()
        .map(|triangle| {
            let base = triangle * 3;
            let ids = [
                mesh.indices[base] as usize,
                mesh.indices[base + 1] as usize,
                mesh.indices[base + 2] as usize,
            ];
            GuardTriangle {
                ids,
                positions: ids.map(|id| {
                    mesh.vertices
                        .get(id)
                        .map_or(Vec3::ZERO, |vertex| Vec3::from_array(vertex.position))
                }),
            }
        })
        .collect();

    cap_pierces(&cap, &surround)
}

/// One triangle in the guard's world: global vertex ids (generated cap
/// vertices use ids past the mesh vertex count, so they never collide) plus
/// positions.
pub(super) struct GuardTriangle {
    /// Global vertex ids used only for shared-vertex pair rejection.
    pub(super) ids: [usize; 3],
    /// Vertex positions.
    pub(super) positions: [Vec3; 3],
}

/// Whether any cap triangle pierces another cap triangle or a surrounding
/// surface triangle. Pairs sharing any vertex id are skipped.
pub(super) fn cap_pierces(cap: &[GuardTriangle], surround: &[GuardTriangle]) -> bool {
    // One combined array; entries below `cap.len()` are cap triangles. Only
    // pairs with at least one cap member are tested.
    let cap_count = cap.len();
    let total = cap_count + surround.len();
    let triangle = |index: usize| -> &GuardTriangle {
        if index < cap_count {
            &cap[index]
        } else {
            &surround[index - cap_count]
        }
    };

    let mut bounds: Vec<(Vec3, Vec3)> = Vec::with_capacity(total);
    let mut world_lo = Vec3::splat(f32::MAX);
    let mut world_hi = Vec3::splat(f32::MIN);
    for index in 0..total {
        let [a, b, c] = triangle(index).positions;
        let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
        world_lo = world_lo.min(lo);
        world_hi = world_hi.max(hi);
        bounds.push((lo, hi));
    }

    // Sweep along the WIDEST world axis (a hole on an axis-aligned wall would
    // degenerate a fixed-x sweep into an all-pairs scan). Sort key falls back
    // to the index for full determinism.
    let extent = world_hi - world_lo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let along = |v: Vec3| -> f32 { v.to_array()[axis] };
    let mut order: Vec<usize> = (0..total).collect();
    order.sort_unstable_by(|&lhs, &rhs| {
        along(bounds[lhs].0)
            .partial_cmp(&along(bounds[rhs].0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(lhs.cmp(&rhs))
    });

    for (slot, &first) in order.iter().enumerate() {
        let (lo1, hi1) = bounds[first];
        for &second in order.iter().skip(slot + 1) {
            let (lo2, hi2) = bounds[second];
            if along(lo2) > along(hi1) {
                break;
            }
            if first >= cap_count && second >= cap_count {
                continue;
            }
            if lo1.x > hi2.x
                || lo2.x > hi1.x
                || lo1.y > hi2.y
                || lo2.y > hi1.y
                || lo1.z > hi2.z
                || lo2.z > hi1.z
            {
                continue;
            }
            let t1 = triangle(first);
            let t2 = triangle(second);
            if shares_vertex(t1, t2) {
                continue;
            }
            if triangles_pierce(t1, t2) {
                return true;
            }
        }
    }
    false
}

fn shares_vertex(t1: &GuardTriangle, t2: &GuardTriangle) -> bool {
    t1.ids.iter().any(|id| t2.ids.contains(id))
}

/// Interior dihedral beyond this is a FOLD, not curvature: the cap's
/// parameterization collapsed (e.g. a strongly wrapped rim whose projection
/// folds over, making the quadric height field two-valued).
const FOLD_COSINE_LIMIT: f32 = -0.5; // 120 degrees

/// Whether the candidate cap folds onto itself: any pair of cap triangles
/// sharing an interior edge whose normals point more than 120 degrees apart.
/// Degenerate (near-zero-area) triangles are skipped — they carry no
/// direction, and the piercing guard covers actual overlap.
pub(super) fn candidate_folds(candidate: &CapCandidate<'_>) -> bool {
    let position = |index: usize| -> Vec3 {
        if index < candidate.rim.len() {
            candidate.rim_positions[index]
        } else {
            candidate
                .generated
                .get(index - candidate.rim.len())
                .copied()
                .unwrap_or(Vec3::ZERO)
        }
    };
    let normals: Vec<Vec3> = candidate
        .triangles
        .iter()
        .map(|&[a, b, c]| {
            let (pa, pb, pc) = (position(a), position(b), position(c));
            (pb - pa).cross(pc - pa)
        })
        .collect();

    // Sorted undirected-edge incidence (deterministic, allocation-light).
    let mut incidence: Vec<((usize, usize), usize)> =
        Vec::with_capacity(candidate.triangles.len() * 3);
    for (triangle_index, &[a, b, c]) in candidate.triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            incidence.push(((u.min(v), u.max(v)), triangle_index));
        }
    }
    incidence.sort_unstable();
    for pair in incidence.windows(2) {
        let ((edge_a, t1), (edge_b, t2)) = (pair[0], pair[1]);
        if edge_a != edge_b {
            continue;
        }
        let (n1, n2) = (normals[t1], normals[t2]);
        let scale = n1.length() * n2.length();
        if scale <= f32::EPSILON {
            continue;
        }
        if n1.dot(n2) / scale < FOLD_COSINE_LIMIT {
            return true;
        }
    }
    false
}

/// Symmetric piercing test: any edge of one triangle passing strictly through
/// the interior of the other.
fn triangles_pierce(t1: &GuardTriangle, t2: &GuardTriangle) -> bool {
    let a = t1.positions.map(|p| p.as_dvec3());
    let b = t2.positions.map(|p| p.as_dvec3());
    edge_pierces(a[0], a[1], &b)
        || edge_pierces(a[1], a[2], &b)
        || edge_pierces(a[2], a[0], &b)
        || edge_pierces(b[0], b[1], &a)
        || edge_pierces(b[1], b[2], &a)
        || edge_pierces(b[2], b[0], &a)
}

/// Whether the segment `seg_start..seg_end` crosses the strict interior of
/// triangle `tri`.
fn edge_pierces(seg_start: DVec3, seg_end: DVec3, tri: &[DVec3; 3]) -> bool {
    let [a, b, c] = *tri;
    let normal = (b - a).cross(c - a);
    let direction = seg_end - seg_start;
    let denominator = normal.dot(direction);
    if denominator.abs() <= f64::EPSILON * normal.length() * direction.length() {
        return false; // Parallel or degenerate: no transversal crossing.
    }
    let t = normal.dot(a - seg_start) / denominator;
    if !(PIERCE_EPSILON..=1.0 - PIERCE_EPSILON).contains(&t) {
        return false;
    }
    let hit = seg_start + direction * t;
    // Barycentric containment, strictly interior.
    let v0 = b - a;
    let v1 = c - a;
    let v2 = hit - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() <= f64::EPSILON {
        return false;
    }
    let beta = (d11 * d20 - d01 * d21) / denom;
    let gamma = (d00 * d21 - d01 * d20) / denom;
    let alpha = 1.0 - beta - gamma;
    alpha > PIERCE_EPSILON && beta > PIERCE_EPSILON && gamma > PIERCE_EPSILON
}
