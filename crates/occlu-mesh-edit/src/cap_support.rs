//! Rim-neighborhood sampling for interpolated caps: vertex adjacency, the
//! outside support band that pins the quadric fit's curvature, and the fixed
//! outside umbrellas the fairing step continues across the seam.

use std::collections::HashSet;

use glam::Vec3;

use super::cap_fair::RimSupport;
use super::MeshEditBuffers;

/// Number of surface rings sampled outside the rim to pin the cap curvature.
const SUPPORT_RING_DEPTH: usize = 2;

/// Build vertex-vertex adjacency from triangle connectivity. Out-of-range
/// indices are skipped here; they are reported by the earlier buffer
/// validation, so this stays infallible.
pub(super) fn build_vertex_adjacency(mesh: &MeshEditBuffers) -> Vec<Vec<usize>> {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); mesh.vertices.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if u < adjacency.len() && v < adjacency.len() {
                if !adjacency[u].contains(&v) {
                    adjacency[u].push(v);
                }
                if !adjacency[v].contains(&u) {
                    adjacency[v].push(u);
                }
            }
        }
    }
    adjacency
}

/// The rim plus everything within [`SUPPORT_RING_DEPTH`] rings outside it.
/// `positions` carries only the OUTSIDE samples (curvature support for the
/// quadric fit) with their geodesic-ish distance to the rim; `vertex_set`
/// additionally contains the rim itself and backs the self-intersection
/// guard's neighborhood query.
pub(super) struct SupportBand {
    /// Positions of surface vertices outside the rim, ring by ring.
    pub(super) positions: Vec<[f32; 3]>,
    /// Per-sample distance to the rim, accumulated along the adjacency walk
    /// that discovered the sample (a cheap geodesic proxy). The quadric fit
    /// downweights far samples: a topological neighbor that is METRICALLY far
    /// (a deep socket wall, a cone apex) is distant geometry, not the local
    /// curvature the band exists to capture.
    pub(super) distances: Vec<f32>,
    /// Rim vertices plus every vertex the band visited.
    pub(super) vertex_set: HashSet<usize>,
}

/// Collect positions of surface vertices within [`SUPPORT_RING_DEPTH`] rings
/// OUTSIDE the rim. These samples carry the local curvature the (often planar)
/// rim ring alone cannot, letting the fitted cap surface bulge to follow the
/// surrounding shape instead of collapsing to a flat disk.
pub(super) fn gather_support_band(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
    adjacency: &[Vec<usize>],
) -> SupportBand {
    let rim: HashSet<usize> = boundary_loop.iter().copied().collect();
    let mut frontier: Vec<(usize, f32)> = boundary_loop.iter().map(|&v| (v, 0.0_f32)).collect();
    let mut seen: HashSet<usize> = rim.clone();
    let mut support: Vec<[f32; 3]> = Vec::new();
    let mut distances: Vec<f32> = Vec::new();
    for _ in 0..SUPPORT_RING_DEPTH {
        let mut next: Vec<(usize, f32)> = Vec::new();
        for &(vertex, walked) in &frontier {
            let Some(neighbors) = adjacency.get(vertex) else {
                continue;
            };
            let Some(from) = mesh.vertices.get(vertex) else {
                continue;
            };
            for &neighbor in neighbors {
                if !seen.insert(neighbor) {
                    continue;
                }
                let Some(position) = mesh.vertices.get(neighbor) else {
                    next.push((neighbor, walked));
                    continue;
                };
                let step =
                    Vec3::from_array(position.position).distance(Vec3::from_array(from.position));
                support.push(position.position);
                distances.push(walked + step);
                next.push((neighbor, walked + step));
            }
        }
        frontier = next;
    }
    SupportBand {
        positions: support,
        distances,
        vertex_set: seen,
    }
}

/// For each rim vertex (ring order), the positions of its mesh neighbors that
/// are NOT on the rim: the fixed outside umbrella completing the rim vertex's
/// full one-ring for seam-continuous fairing.
pub(super) fn rim_outside_support(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
    adjacency: &[Vec<usize>],
) -> Vec<RimSupport> {
    let rim: HashSet<usize> = boundary_loop.iter().copied().collect();
    boundary_loop
        .iter()
        .map(|&vertex| {
            let mut outside = Vec::new();
            if let Some(neighbors) = adjacency.get(vertex) {
                for &neighbor in neighbors {
                    if rim.contains(&neighbor) {
                        continue;
                    }
                    if let Some(position) = mesh.vertices.get(neighbor) {
                        outside.push(Vec3::from_array(position.position));
                    }
                }
            }
            RimSupport { outside }
        })
        .collect()
}
