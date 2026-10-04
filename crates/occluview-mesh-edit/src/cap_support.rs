//! What a cap reads of the surface around its rim: vertex adjacency, the
//! neighbourhood the self-intersection guard checks the cap against, and the
//! fixed outside umbrellas the cap's shape continues across the seam.

use std::collections::{HashMap, HashSet};

use glam::Vec3;

use super::cap_fair::RimSupport;
use super::cap_guard::VertexTriangleIncidence;
use super::cap_minweight::TakenTriangles;
use super::MeshEditBuffers;

/// Number of surface rings outside the rim the guard checks a cap against.
const GUARD_RING_DEPTH: usize = 2;

/// Build vertex-vertex adjacency from triangle connectivity. Out-of-range
/// indices are skipped here; they are reported by the earlier buffer
/// validation, so this stays infallible.
pub(super) fn build_vertex_adjacency(mesh: &MeshEditBuffers) -> Vec<Vec<usize>> {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); mesh.vertices.len()];
    for triangle in mesh.indices.as_chunks::<3>().0 {
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

/// The rim plus every vertex within [`GUARD_RING_DEPTH`] rings outside it: the
/// surface a cap could run into, for the self-intersection guard.
pub(super) fn rim_neighbourhood(
    boundary_loop: &[usize],
    adjacency: &[Vec<usize>],
) -> HashSet<usize> {
    let mut seen: HashSet<usize> = boundary_loop.iter().copied().collect();
    let mut frontier: Vec<usize> = boundary_loop.to_vec();
    for _ in 0..GUARD_RING_DEPTH {
        let mut next: Vec<usize> = Vec::new();
        for &vertex in &frontier {
            let Some(neighbors) = adjacency.get(vertex) else {
                continue;
            };
            for &neighbor in neighbors {
                if seen.insert(neighbor) {
                    next.push(neighbor);
                }
            }
        }
        frontier = next;
    }
    seen
}

/// For each rim edge (from the rim vertex of the same index to the next), what
/// the scan triangle across it adds to the edge's hinge: the fixed side of the
/// hinge, which lets a cap's shape continue the scan across the seam.
pub(super) fn rim_outside_support(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
    incidence: &VertexTriangleIncidence,
) -> Vec<RimSupport> {
    let position = |vertex: usize| {
        mesh.vertices
            .get(vertex)
            .map(|vertex| Vec3::from_array(vertex.position))
    };
    let loop_len = boundary_loop.len();
    (0..loop_len)
        .map(|index| {
            let (from, to) = (boundary_loop[index], boundary_loop[(index + 1) % loop_len]);
            // A rim edge is a directed edge of the one triangle that owns it;
            // the edge between the two copies of a split junction has none.
            let apex = incidence.triangles_of(from).iter().find_map(|&triangle| {
                let corners = mesh.indices.get(triangle * 3..triangle * 3 + 3)?;
                (0..3)
                    .find(|&corner| {
                        corners[corner] as usize == from && corners[(corner + 1) % 3] as usize == to
                    })
                    .map(|corner| corners[(corner + 2) % 3] as usize)
            });
            match (position(from), position(to), apex.and_then(position)) {
                (Some(from), Some(to), Some(apex)) => RimSupport::across(from, to, apex),
                _ => RimSupport::default(),
            }
        })
        .collect()
}

/// The triangles the surface already has on a rim's own vertices, in
/// rim-local indices: the ones a cap of the rim may not use.
pub(super) fn rim_taken_triangles(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
    incidence: &VertexTriangleIncidence,
) -> TakenTriangles {
    let local_of: HashMap<usize, usize> = boundary_loop
        .iter()
        .enumerate()
        .map(|(local, &vertex)| (vertex, local))
        .collect();
    let mut taken = TakenTriangles::new();
    for &rim_vertex in boundary_loop {
        for &triangle in incidence.triangles_of(rim_vertex) {
            let Some(corners) = mesh.indices.get(triangle * 3..triangle * 3 + 3) else {
                continue;
            };
            let on_rim = [corners[0], corners[1], corners[2]]
                .map(|corner| local_of.get(&(corner as usize)).copied());
            if let [Some(a), Some(b), Some(c)] = on_rim {
                let mut triple = [a, b, c];
                triple.sort_unstable();
                taken.insert(triple);
            }
        }
    }
    taken
}
