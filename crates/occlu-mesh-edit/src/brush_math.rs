//! Pure, stateless helpers for the sculpt brush kernel, split out of `brush.rs`
//! to hold the workspace's 800-line file budget: the Smooth pass count, the
//! open-boundary mask, the per-vertex anti-inversion step budget, the
//! shortest-incident-edge probe, and the radial falloff.

use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;

use super::brush_csr::Csr;

const COLLAPSE_FRACTION_SQUARED: f32 = 1e-4;

/// Area-weighted vertex normal for every id in `scope`, computed in PARALLEL
/// and conflict-free — each entry reads only its own incident faces, so no
/// per-face dedup is needed. Returns one un-normalized normal per `scope`
/// entry; the caller normalizes and writes back. Blender-sculpt strategy
/// (PR #116209): recompute from geometry directly instead of scattering face
/// normals through a single-threaded `VectorSet`.
pub(crate) fn scope_area_normals(
    scope: &[usize],
    incident_triangles: &Csr,
    indices: &[u32],
    positions: &[Vec3],
) -> Vec<Vec3> {
    let vertex_count = positions.len();
    scope
        .par_iter()
        .map(|&vertex_id| {
            let mut sum = Vec3::ZERO;
            for &triangle_index in incident_triangles.row(vertex_id) {
                let base = triangle_index as usize * 3;
                let Some(slice) = indices.get(base..base + 3) else {
                    continue;
                };
                let (a, b, c) = (slice[0] as usize, slice[1] as usize, slice[2] as usize);
                if a >= vertex_count || b >= vertex_count || c >= vertex_count {
                    continue;
                }
                let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
                if face.is_finite() {
                    sum += face;
                }
            }
            sum
        })
        .collect()
}

/// Most Laplacian passes a single forced (Shift) dab runs. High so Shift
/// smooths CARDINALLY — enough to iron a rough scan patch nearly flat in one
/// held stroke.
pub(crate) const MAX_SMOOTH_PASSES: usize = 16;

/// Number of whole Laplacian passes one Smooth dab runs, from its clamped
/// strength: at least one, up to [`MAX_SMOOTH_PASSES`] at full strength (the
/// forced Shift mode passes ~1.0). Expressing strength as pass count — not a
/// smaller per-pass factor — is what makes Smooth visibly strong.
pub(crate) fn smooth_pass_count(strength: f32) -> usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let extra = (strength.clamp(0.0, 1.0) * (MAX_SMOOTH_PASSES - 1) as f32).round() as usize;
    1 + extra
}

/// Mark every vertex on an open boundary — an undirected edge used by exactly
/// one triangle — from the welded indices, then propagate to each boundary
/// vertex's soup duplicates so a pinned corner stays pinned everywhere. Scan
/// borders and hole rims are boundaries; pinning stops Smooth/auto-smooth from
/// eroding the scan's edge.
pub(crate) fn boundary_mask(
    indices: &[u32],
    position_siblings: &Csr,
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
            for &sibling in position_siblings.row(vertex_id) {
                let sibling = sibling as usize;
                if sibling < vertex_count {
                    is_boundary[sibling] = true;
                }
            }
        }
    }
    is_boundary
}

/// Per-vertex anti-inversion step budget: shortest incident welded edge,
/// reduced to the minimum across each soup cluster. A non-representative
/// duplicate has an EMPTY welded ring, so [`shortest_incident_edge`] would
/// give it the generous isolated-vertex fallback instead of the
/// representative's tight budget. Without this propagation, a duplicate —
/// captured by the same spatial query since all copies share a position —
/// re-applies a dab at the loose fallback and overwrites the representative's
/// clamped move, silently defeating the anti-inversion guard on STL soup.
pub(crate) fn compute_step_budget(
    positions: &[Vec3],
    adjacency: &Csr,
    position_siblings: &Csr,
) -> Vec<f32> {
    let raw: Vec<f32> = (0..positions.len())
        .map(|index| shortest_incident_edge(positions, adjacency.row(index), positions[index]))
        .collect();
    (0..positions.len())
        .map(|index| {
            let mut budget = raw[index];
            for &sibling in position_siblings.row(index) {
                if let Some(&value) = raw.get(sibling as usize) {
                    budget = budget.min(value);
                }
            }
            budget
        })
        .collect()
}

/// Shortest edge from `here` to any of `neighbors`' positions, capped (not
/// floored) at 1mm so a single dab cannot jump too far on a sparse/low-poly
/// mesh. A genuinely SMALL edge is returned unfloored: a fine occlusal groove
/// or margin line can have real spacing well under a coarse floor, and
/// flooring would inflate `clamp_step`'s budget past what the local topology
/// tolerates. An isolated vertex (no finite neighbor distance) falls back to
/// a generous budget so its step is never zero-clamped by a topology fluke.
pub(crate) fn shortest_incident_edge(positions: &[Vec3], neighbors: &[u32], here: Vec3) -> f32 {
    let shortest = neighbors
        .iter()
        .filter_map(|&neighbor| positions.get(neighbor as usize))
        .map(|&position| position.distance(here))
        .filter(|length| length.is_finite() && *length > 0.0)
        .fold(f32::MAX, f32::min);
    if shortest == f32::MAX {
        return 1.0;
    }
    shortest.min(1.0)
}

/// Whether the mesh is a SINGLE connected surface (welded rings + soup sibling
/// links). A dab on a single-component scan — the common case — can skip the
/// per-dab flood fill entirely, since there is nothing else to drag along.
/// Computed once at prepare.
pub(crate) fn is_single_component(
    adjacency: &Csr,
    position_siblings: &Csr,
    vertex_count: usize,
) -> bool {
    if vertex_count == 0 {
        return true;
    }
    let mut visited = vec![false; vertex_count];
    let mut stack = vec![0usize];
    visited[0] = true;
    let mut reached = 1usize;
    while let Some(vertex_id) = stack.pop() {
        for &neighbor in adjacency
            .row(vertex_id)
            .iter()
            .chain(position_siblings.row(vertex_id).iter())
        {
            let neighbor = neighbor as usize;
            if neighbor < vertex_count && !visited[neighbor] {
                visited[neighbor] = true;
                reached += 1;
                stack.push(neighbor);
            }
        }
    }
    reached == vertex_count
}

/// Whether any triangle incident to `vertex_id` flipped orientation vs its
/// pre-dab geometry: pre positions for the moved region (stamped `generation`),
/// current positions elsewhere. The post-dab inversion guard's per-vertex test.
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_flipped_triangle(
    vertex_id: usize,
    generation: u32,
    incident: &Csr,
    indices: &[u32],
    positions: &[Vec3],
    pre_position: &[Vec3],
    stamp: &[u32],
) -> bool {
    let vertex_count = positions.len();
    let pre = |v: usize| {
        if stamp[v] == generation {
            pre_position[v]
        } else {
            positions[v]
        }
    };
    for &triangle in incident.row(vertex_id) {
        let base = triangle as usize * 3;
        let Some(slice) = indices.get(base..base + 3) else {
            continue;
        };
        let (a, b, c) = (slice[0] as usize, slice[1] as usize, slice[2] as usize);
        if a >= vertex_count || b >= vertex_count || c >= vertex_count {
            continue;
        }
        let normal_pre = (pre(b) - pre(a)).cross(pre(c) - pre(a));
        let normal_now = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        let pre_area_squared = normal_pre.length_squared();
        // Scan meshes contain legitimate very small facets. An absolute
        // epsilon treated those as collapsed after an otherwise valid dab and
        // caused the caller to discard an entire brush stroke. Use the facet's
        // own pre-dab area for the collapse test; true winding reversals still
        // fail regardless of scale.
        let collapsed = pre_area_squared > f32::EPSILON
            && normal_now.length_squared() <= pre_area_squared * COLLAPSE_FRACTION_SQUARED;
        let reversed = pre_area_squared > f32::EPSILON && normal_now.dot(normal_pre) < 0.0;
        if collapsed || reversed {
            return true;
        }
    }
    false
}

/// Recompute `max_step` for `touched` from current positions (shortest incident
/// edge, then the soup-cluster minimum), keeping the anti-inversion budget in
/// step with the moved geometry instead of stale prepare-time edges.
pub(crate) fn refresh_step_budget(
    touched: &[usize],
    positions: &[Vec3],
    adjacency: &Csr,
    siblings: &Csr,
    max_step: &mut [f32],
) {
    for &vertex_id in touched {
        max_step[vertex_id] =
            shortest_incident_edge(positions, adjacency.row(vertex_id), positions[vertex_id]);
    }
    for &vertex_id in touched {
        let mut budget = max_step[vertex_id];
        for &sibling in siblings.row(vertex_id) {
            budget = budget.min(max_step[sibling as usize]);
        }
        max_step[vertex_id] = budget;
    }
}

/// Hermite smoothstep ramping 0→1 as `t` goes 0→`edge`, then held at 1.
pub(crate) fn smoothstep(edge: f32, t: f32) -> f32 {
    if edge <= 0.0 {
        return if t > 0.0 { 1.0 } else { 0.0 };
    }
    let s = (t / edge).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

/// Smooth radial falloff: 1 at the center, 0 at/beyond `radius`, `C1`-smooth at
/// the boundary (squared sculpting falloff).
pub(crate) fn falloff(distance: f32, radius: f32) -> f32 {
    if !(distance.is_finite() && radius.is_finite()) || radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    let t = (1.0 - distance / radius).clamp(0.0, 1.0);
    t * t
}
