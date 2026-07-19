//! Pure, stateless helpers for the sculpt brush kernel, split out of `brush.rs`
//! to hold the workspace's 800-line file budget: the Smooth pass count, the
//! open-boundary mask, the per-vertex anti-inversion step budget, the
//! shortest-incident-edge probe, and the radial falloff.

use glam::Vec3;
use std::collections::HashMap;

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

/// Mark every vertex that sits on an open boundary — an undirected edge used by
/// exactly one triangle — from the welded indices, then propagate the mark to
/// each boundary vertex's soup duplicates so a pinned corner stays pinned in
/// every slot. Scan borders and hole rims are boundaries; pinning them stops
/// Smooth and the auto-smooth from eroding the scan's edge.
pub(crate) fn boundary_mask(
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

/// Per-vertex anti-inversion step budget: the shortest incident welded edge,
/// then reduced to the minimum across each soup cluster. A non-representative
/// soup duplicate has an EMPTY welded ring (only the cluster representative
/// carries the real adjacency), so [`shortest_incident_edge`] would hand it the
/// generous isolated-vertex fallback; propagating the cluster minimum gives
/// every duplicate the representative's tight, correct budget. Without this a
/// higher-id duplicate — captured by the same spatial query, since all copies
/// share a position — re-applies a clay dab at the loose fallback budget and
/// overwrites the representative's correctly clamped move, silently defeating
/// the anti-inversion guard on ordinary STL soup.
pub(crate) fn compute_step_budget(
    positions: &[Vec3],
    adjacency: &[Vec<usize>],
    position_siblings: &[Vec<usize>],
) -> Vec<f32> {
    let raw: Vec<f32> = (0..positions.len())
        .map(|index| shortest_incident_edge(positions, &adjacency[index], positions[index]))
        .collect();
    (0..positions.len())
        .map(|index| {
            let mut budget = raw[index];
            for &sibling in &position_siblings[index] {
                if let Some(&value) = raw.get(sibling) {
                    budget = budget.min(value);
                }
            }
            budget
        })
        .collect()
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

/// Whether the mesh is a SINGLE connected surface (over welded rings + soup
/// sibling links). A dab on a single-component scan — the overwhelmingly common
/// case — can skip the per-dab component flood fill entirely, since there is no
/// other surface to accidentally drag along. Computed once at prepare.
pub(crate) fn is_single_component(
    adjacency: &[Vec<usize>],
    position_siblings: &[Vec<usize>],
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
        for &neighbor in adjacency[vertex_id]
            .iter()
            .chain(position_siblings[vertex_id].iter())
        {
            if neighbor < vertex_count && !visited[neighbor] {
                visited[neighbor] = true;
                reached += 1;
                stack.push(neighbor);
            }
        }
    }
    reached == vertex_count
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
