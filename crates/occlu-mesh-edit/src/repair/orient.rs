//! Pass 6: coherent triangle orientation.
//!
//! Per component: BFS over shared undirected edges enforcing OPPOSING
//! directed edges (seed = lowest triangle index), then an area-weighted
//! majority rule so the component keeps its original dominant orientation,
//! then — for CLOSED components only — an outward signed-volume check. Open
//! components never get the volume flip.

use std::collections::VecDeque;

use super::{
    dvec3_position, edge_faces, triangle_area_f64, Components, EdgeIncidence, RepairReport,
};
use crate::MeshEditBuffers;

pub(super) fn orient_components(
    mesh: &mut MeshEditBuffers,
    components: &Components,
    incidence: &[EdgeIncidence],
    report: &mut RepairReport,
) {
    let triangle_count = mesh.triangle_count();
    if triangle_count == 0 {
        return;
    }
    let mut flip = vec![false; triangle_count];
    let mut visited = vec![false; triangle_count];

    for members in &components.members {
        propagate_orientation(mesh, incidence, members[0], &mut visited, &mut flip);
        normalize_majority(mesh, members, &mut flip);
        report.reoriented_triangles += members.iter().filter(|&&t| flip[t]).count();
    }

    let closed = closed_components(components, incidence);
    for (component, members) in components.members.iter().enumerate() {
        if !closed[component] {
            continue;
        }
        if signed_volume_six(mesh, members, &flip) < 0.0 {
            for &triangle in members {
                flip[triangle] = !flip[triangle];
            }
            report.flipped_components += 1;
        }
    }

    for (triangle, &flagged) in flip.iter().enumerate() {
        if flagged {
            mesh.indices.swap(triangle * 3 + 1, triangle * 3 + 2);
        }
    }
}

/// BFS from `seed` (which keeps its winding): a neighbor must traverse the
/// shared edge in the direction OPPOSITE to the current face's effective
/// direction, or it gets a flip flag. First assignment wins on conflicts.
fn propagate_orientation(
    mesh: &MeshEditBuffers,
    incidence: &[EdgeIncidence],
    seed: usize,
    visited: &mut [bool],
    flip: &mut [bool],
) {
    let mut queue = VecDeque::from([seed]);
    visited[seed] = true;
    while let Some(triangle) = queue.pop_front() {
        for (edge, forward) in directed_edges(&mesh.indices, triangle) {
            let effective = forward != flip[triangle];
            for &(_, neighbor) in edge_faces(incidence, edge) {
                if visited[neighbor] {
                    continue;
                }
                visited[neighbor] = true;
                // Same effective direction on a shared edge = inconsistent.
                flip[neighbor] = traverses_forward(&mesh.indices, neighbor, edge) == effective;
                queue.push_back(neighbor);
            }
        }
    }
}

/// If the BFS flagged more area than it kept, the seed side was the minority:
/// invert the flags so the component preserves its dominant orientation.
fn normalize_majority(mesh: &MeshEditBuffers, members: &[usize], flip: &mut [bool]) {
    let mut kept_area = 0.0_f64;
    let mut flipped_area = 0.0_f64;
    for &triangle in members {
        let area = triangle_area_f64(mesh, triangle);
        if flip[triangle] {
            flipped_area += area;
        } else {
            kept_area += area;
        }
    }
    if flipped_area > kept_area {
        for &triangle in members {
            flip[triangle] = !flip[triangle];
        }
    }
}

/// A component is closed iff every one of its undirected edges carries
/// exactly two faces (no boundary, and pass 4 already capped fan counts).
fn closed_components(components: &Components, incidence: &[EdgeIncidence]) -> Vec<bool> {
    let mut closed = vec![true; components.members.len()];
    let mut run_start = 0;
    while run_start < incidence.len() {
        let mut run_end = run_start + 1;
        while run_end < incidence.len() && incidence[run_end].0 == incidence[run_start].0 {
            run_end += 1;
        }
        if run_end - run_start != 2 {
            for &(_, triangle) in &incidence[run_start..run_end] {
                closed[components.component_of[triangle]] = false;
            }
        }
        run_start = run_end;
    }
    closed
}

/// Six times the signed volume of a component, accumulated in `f64`, with
/// pending winding flips applied virtually.
fn signed_volume_six(mesh: &MeshEditBuffers, members: &[usize], flip: &[bool]) -> f64 {
    let mut six_volume = 0.0_f64;
    for &triangle in members {
        let base = triangle * 3;
        let mut second = mesh.indices[base + 1];
        let mut third = mesh.indices[base + 2];
        if flip[triangle] {
            std::mem::swap(&mut second, &mut third);
        }
        let a = dvec3_position(mesh, mesh.indices[base]);
        let b = dvec3_position(mesh, second);
        let c = dvec3_position(mesh, third);
        six_volume += a.dot(b.cross(c));
    }
    six_volume
}

/// The three canonical edges of a triangle with their traversal direction:
/// `true` when the winding walks the edge from its lower to its higher index.
fn directed_edges(indices: &[u32], triangle: usize) -> [((u32, u32), bool); 3] {
    let base = triangle * 3;
    let (a, b, c) = (indices[base], indices[base + 1], indices[base + 2]);
    [
        directed_edge(a, b),
        directed_edge(b, c),
        directed_edge(c, a),
    ]
}

fn directed_edge(from: u32, to: u32) -> ((u32, u32), bool) {
    ((from.min(to), from.max(to)), from < to)
}

/// Whether `triangle` traverses `edge` from its lower to its higher index.
fn traverses_forward(indices: &[u32], triangle: usize, edge: (u32, u32)) -> bool {
    directed_edges(indices, triangle)
        .into_iter()
        .find(|&(candidate, _)| candidate == edge)
        .is_some_and(|(_, forward)| forward)
}
