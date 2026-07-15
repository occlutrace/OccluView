//! Shared deterministic graph analysis over already-canonical triangle indices.

use std::collections::VecDeque;

use super::adjacency::triangle_edge_keys;

/// One `(undirected edge, triangle)` incidence entry.
pub(crate) type EdgeIncidence = ((u32, u32), usize);

/// Sorted incidence list. Equal-edge runs contain incident faces in ascending
/// triangle order.
pub(crate) fn edge_incidence(indices: &[u32]) -> Vec<EdgeIncidence> {
    let mut incidence = Vec::with_capacity(indices.len());
    for (triangle, face) in indices.chunks_exact(3).enumerate() {
        for edge in triangle_edge_keys(face) {
            incidence.push((edge, triangle));
        }
    }
    incidence.sort_unstable();
    incidence
}

/// Incident faces of one undirected edge, found by binary search.
pub(crate) fn edge_faces(incidence: &[EdgeIncidence], edge: (u32, u32)) -> &[EdgeIncidence] {
    let start = incidence.partition_point(|entry| entry.0 < edge);
    let end = incidence.partition_point(|entry| entry.0 <= edge);
    &incidence[start..end]
}

/// Edge-connected triangle components in deterministic seed/member order.
pub(crate) struct Components {
    pub(crate) component_of: Vec<usize>,
    pub(crate) members: Vec<Vec<usize>>,
}

pub(crate) fn connected_components(indices: &[u32], incidence: &[EdgeIncidence]) -> Components {
    let triangle_count = indices.len() / 3;
    let mut component_of = vec![usize::MAX; triangle_count];
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut queue = VecDeque::new();

    for seed in 0..triangle_count {
        if component_of[seed] != usize::MAX {
            continue;
        }
        let component = members.len();
        component_of[seed] = component;
        queue.push_back(seed);
        let mut list = Vec::new();
        while let Some(triangle) = queue.pop_front() {
            list.push(triangle);
            for edge in triangle_edge_keys(&indices[triangle * 3..triangle * 3 + 3]) {
                for &(_, neighbor) in edge_faces(incidence, edge) {
                    if component_of[neighbor] == usize::MAX {
                        component_of[neighbor] = component;
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        list.sort_unstable();
        members.push(list);
    }

    Components {
        component_of,
        members,
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TopologyDefects {
    pub(crate) boundary_edges: usize,
    pub(crate) non_manifold_edges: usize,
    pub(crate) inconsistent_winding_edges: usize,
    pub(crate) non_manifold_vertices: usize,
}

pub(crate) fn topology_defects(indices: &[u32], incidence: &[EdgeIncidence]) -> TopologyDefects {
    let mut defects = TopologyDefects::default();
    let mut run_start = 0;
    while run_start < incidence.len() {
        let edge = incidence[run_start].0;
        let mut run_end = run_start + 1;
        while run_end < incidence.len() && incidence[run_end].0 == edge {
            run_end += 1;
        }
        let face_count = run_end - run_start;
        if face_count == 1 {
            defects.boundary_edges += 1;
        } else if face_count > 2 {
            defects.non_manifold_edges += 1;
        } else if face_count == 2 {
            let forward_uses = incidence[run_start..run_end]
                .iter()
                .filter(|&&(_, triangle)| triangle_uses_edge_forward(indices, triangle, edge))
                .count();
            defects.inconsistent_winding_edges += usize::from(forward_uses != 1);
        }
        run_start = run_end;
    }

    if defects.boundary_edges == 0 && defects.non_manifold_edges == 0 {
        defects.non_manifold_vertices = count_non_manifold_vertices(indices);
    }
    defects
}

fn triangle_uses_edge_forward(indices: &[u32], triangle: usize, edge: (u32, u32)) -> bool {
    let face = &indices[triangle * 3..triangle * 3 + 3];
    [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])]
        .into_iter()
        .any(|directed| directed == edge)
}

fn count_non_manifold_vertices(indices: &[u32]) -> usize {
    let mut vertex_faces: Vec<(u32, usize)> = Vec::with_capacity(indices.len());
    for (triangle, face) in indices.chunks_exact(3).enumerate() {
        for &vertex in face {
            vertex_faces.push((vertex, triangle));
        }
    }
    vertex_faces.sort_unstable();

    let mut count = 0;
    let mut run_start = 0;
    while run_start < vertex_faces.len() {
        let vertex = vertex_faces[run_start].0;
        let mut run_end = run_start + 1;
        while run_end < vertex_faces.len() && vertex_faces[run_end].0 == vertex {
            run_end += 1;
        }
        let faces: Vec<usize> = vertex_faces[run_start..run_end]
            .iter()
            .map(|&(_, triangle)| triangle)
            .collect();
        count += usize::from(vertex_fan_clusters(indices, vertex, &faces).1 > 1);
        run_start = run_end;
    }
    count
}

/// Per-face fan cluster ids plus the number of clusters around one vertex.
pub(crate) fn vertex_fan_clusters(
    indices: &[u32],
    vertex: u32,
    faces: &[usize],
) -> (Vec<usize>, usize) {
    let mut pairs: Vec<(u32, usize)> = Vec::with_capacity(faces.len() * 2);
    for (local, &triangle) in faces.iter().enumerate() {
        for &other in &indices[triangle * 3..triangle * 3 + 3] {
            if other != vertex {
                pairs.push((other, local));
            }
        }
    }
    pairs.sort_unstable();

    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut run_start = 0;
    while run_start < pairs.len() {
        let mut scan = run_start + 1;
        while scan < pairs.len() && pairs[scan].0 == pairs[run_start].0 {
            union(&mut parent, pairs[run_start].1, pairs[scan].1);
            scan += 1;
        }
        run_start = scan;
    }

    let mut roots = Vec::new();
    let mut cluster_of = vec![0; faces.len()];
    for (local, cluster) in cluster_of.iter_mut().enumerate() {
        let root = find(&mut parent, local);
        *cluster = if let Some(existing) = roots.iter().position(|&seen| seen == root) {
            existing
        } else {
            roots.push(root);
            roots.len() - 1
        };
    }
    (cluster_of, roots.len())
}

fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn union(parent: &mut [usize], first: usize, second: usize) {
    let first_root = find(parent, first);
    let second_root = find(parent, second);
    if first_root != second_root {
        let low = first_root.min(second_root);
        let high = first_root.max(second_root);
        parent[high] = low;
    }
}
