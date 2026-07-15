use std::collections::{BTreeMap, BTreeSet};

use crate::topology::canonical_position_key;
use crate::{BridgeSplitError, MeshEditBuffers, MeshEditError};

type PositionKey = [u32; 3];
type CutEdge = (PositionKey, PositionKey);
type CutAdjacency = BTreeMap<PositionKey, BTreeSet<PositionKey>>;

struct CutGraph {
    representative: BTreeMap<PositionKey, usize>,
    adjacency: CutAdjacency,
    edges: BTreeSet<CutEdge>,
}

pub(crate) fn build_cut_loops(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
) -> Result<Vec<Vec<usize>>, BridgeSplitError> {
    if cut_edges.is_empty() {
        return Err(damaged("no cut edges were generated"));
    }

    let mut representative: BTreeMap<PositionKey, usize> = BTreeMap::new();
    let mut directed = BTreeSet::new();
    for &[from, to] in cut_edges {
        let from_vertex =
            mesh.vertices
                .get(from as usize)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut edge start is out of range".to_string(),
                })?;
        let to_vertex =
            mesh.vertices
                .get(to as usize)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut edge end is out of range".to_string(),
                })?;
        let from_key = canonical_position_key(from_vertex.position);
        let to_key = canonical_position_key(to_vertex.position);
        if from_key == to_key {
            return Err(damaged("cut edge collapsed to one geometric point"));
        }
        representative
            .entry(from_key)
            .and_modify(|index| *index = (*index).min(from as usize))
            .or_insert(from as usize);
        representative
            .entry(to_key)
            .and_modify(|index| *index = (*index).min(to as usize))
            .or_insert(to as usize);
        directed.insert((from_key, to_key));
    }

    for &(from, to) in &directed {
        if directed.contains(&(to, from)) {
            return Err(damaged("cut rim contains opposing duplicate segments"));
        }
    }

    let mut outgoing = BTreeMap::new();
    let mut incoming = BTreeMap::new();
    for &(from, to) in &directed {
        if outgoing.insert(from, to).is_some() {
            return Err(damaged("cut rim has a branching outgoing junction"));
        }
        if incoming.insert(to, from).is_some() {
            return Err(damaged("cut rim has a branching incoming junction"));
        }
    }
    if outgoing.len() != representative.len() || incoming.len() != representative.len() {
        return Err(damaged("cut rim is open or has an incomplete junction"));
    }

    let mut visited = BTreeSet::new();
    let mut loops = Vec::new();
    for &start in outgoing.keys() {
        let Some(&first_next) = outgoing.get(&start) else {
            return Err(damaged("cut rim is missing an outgoing segment"));
        };
        if visited.contains(&(start, first_next)) {
            continue;
        }
        let mut ring = Vec::new();
        let mut ring_vertices = BTreeSet::new();
        let mut current = start;
        loop {
            if !ring_vertices.insert(current) {
                if current == start {
                    break;
                }
                return Err(damaged("cut rim revisits a vertex before closing"));
            }
            let &global = representative
                .get(&current)
                .ok_or_else(|| damaged("cut rim lost a vertex representative"))?;
            ring.push(global);
            let &next = outgoing
                .get(&current)
                .ok_or_else(|| damaged("cut rim terminates before closing"))?;
            if !visited.insert((current, next)) {
                return Err(damaged("cut rim reuses a directed segment"));
            }
            current = next;
            if current == start {
                break;
            }
            if ring.len() > representative.len() {
                return Err(damaged("cut rim walk exceeded its vertex budget"));
            }
        }
        if ring.len() < 3 {
            return Err(damaged("cut rim has fewer than three vertices"));
        }
        loops.push(ring);
    }
    if visited.len() != directed.len() {
        return Err(damaged(
            "not every generated cut segment belongs to a closed loop",
        ));
    }
    Ok(loops)
}

/// Recover only complete closed components from a surface cut graph.
///
/// An open scan can legitimately produce a cut path that terminates at its
/// natural border. That path is not safe to cap, but an unrelated closed cut
/// loop must not be discarded because of it. This recovery path therefore
/// splits the graph into connected components, ignores open/branching
/// components, and returns only components whose geometric degree is exactly
/// two at every vertex. The strict [`build_cut_loops`] path remains in use for
/// closed-solid splitting, where silently dropping any generated edge would
/// hide a topology defect.
pub(crate) fn build_closed_cut_loops(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
) -> Result<Vec<Vec<usize>>, BridgeSplitError> {
    if cut_edges.is_empty() {
        return Err(damaged("no cut edges were generated"));
    }

    let graph = build_surface_cut_graph(mesh, cut_edges)?;
    let CutGraph {
        representative,
        adjacency,
        edges,
    } = graph;
    let mut remaining = edges;

    let mut loops = Vec::new();
    while let Some(&(seed_from, seed_to)) = remaining.iter().next() {
        let component_vertices = connected_component(seed_from, seed_to, &adjacency);
        let component_edges = component_edges(&component_vertices, &adjacency);
        for edge in &component_edges {
            remaining.remove(edge);
        }

        if !is_simple_cycle(&component_vertices, &component_edges, &adjacency) {
            continue;
        }
        loops.push(walk_closed_component(
            &component_vertices,
            &adjacency,
            &representative,
        )?);
    }

    if loops.is_empty() {
        return Err(damaged("no complete closed cut rim could be recovered"));
    }
    Ok(loops)
}

fn build_surface_cut_graph(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
) -> Result<CutGraph, BridgeSplitError> {
    let mut representative: BTreeMap<PositionKey, usize> = BTreeMap::new();
    let mut edges = BTreeSet::new();
    for &[from, to] in cut_edges {
        let from_vertex =
            mesh.vertices
                .get(from as usize)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut edge start is out of range".to_string(),
                })?;
        let to_vertex =
            mesh.vertices
                .get(to as usize)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut edge end is out of range".to_string(),
                })?;
        let from_key = canonical_position_key(from_vertex.position);
        let to_key = canonical_position_key(to_vertex.position);
        if from_key == to_key {
            continue;
        }
        representative
            .entry(from_key)
            .and_modify(|index| *index = (*index).min(from as usize))
            .or_insert(from as usize);
        representative
            .entry(to_key)
            .and_modify(|index| *index = (*index).min(to as usize))
            .or_insert(to as usize);
        edges.insert(ordered_edge(from_key, to_key));
    }
    if edges.is_empty() {
        return Err(damaged(
            "every cut segment collapsed to one geometric point",
        ));
    }

    let mut adjacency = CutAdjacency::new();
    for &(from, to) in &edges {
        adjacency.entry(from).or_default().insert(to);
        adjacency.entry(to).or_default().insert(from);
    }
    Ok(CutGraph {
        representative,
        adjacency,
        edges,
    })
}

fn ordered_edge(first: PositionKey, second: PositionKey) -> CutEdge {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

fn connected_component(
    seed_from: PositionKey,
    seed_to: PositionKey,
    adjacency: &CutAdjacency,
) -> BTreeSet<PositionKey> {
    let mut component = BTreeSet::new();
    let mut stack = vec![seed_from, seed_to];
    while let Some(vertex) = stack.pop() {
        if !component.insert(vertex) {
            continue;
        }
        if let Some(neighbors) = adjacency.get(&vertex) {
            stack.extend(neighbors.iter().copied());
        }
    }
    component
}

fn component_edges(
    vertices: &BTreeSet<PositionKey>,
    adjacency: &CutAdjacency,
) -> BTreeSet<CutEdge> {
    vertices
        .iter()
        .filter_map(|vertex| adjacency.get(vertex).map(|neighbors| (*vertex, neighbors)))
        .flat_map(|(vertex, neighbors)| {
            neighbors
                .iter()
                .map(move |neighbor| ordered_edge(vertex, *neighbor))
        })
        .collect()
}

fn is_simple_cycle(
    vertices: &BTreeSet<PositionKey>,
    edges: &BTreeSet<CutEdge>,
    adjacency: &CutAdjacency,
) -> bool {
    vertices.len() >= 3
        && edges.len() == vertices.len()
        && vertices.iter().all(|vertex| {
            adjacency
                .get(vertex)
                .is_some_and(|neighbors| neighbors.len() == 2)
        })
}

fn walk_closed_component(
    vertices: &BTreeSet<PositionKey>,
    adjacency: &CutAdjacency,
    representative: &BTreeMap<PositionKey, usize>,
) -> Result<Vec<usize>, BridgeSplitError> {
    let start = *vertices
        .first()
        .ok_or_else(|| damaged("closed cut component has no start vertex"))?;
    let mut ring_keys = Vec::with_capacity(vertices.len());
    let mut previous = None;
    let mut current = start;
    loop {
        ring_keys.push(current);
        let neighbors = adjacency
            .get(&current)
            .ok_or_else(|| damaged("closed cut component lost adjacency"))?;
        let next = neighbors
            .iter()
            .copied()
            .find(|candidate| Some(*candidate) != previous)
            .ok_or_else(|| damaged("closed cut component has no successor"))?;
        if next == start {
            break;
        }
        previous = Some(current);
        current = next;
        if ring_keys.len() > vertices.len() {
            return Err(damaged(
                "closed cut component walk exceeded its vertex budget",
            ));
        }
    }
    ring_keys
        .into_iter()
        .map(|key| {
            representative
                .get(&key)
                .copied()
                .ok_or_else(|| damaged("closed cut component lost a vertex representative"))
        })
        .collect()
}

fn damaged(reason: &str) -> BridgeSplitError {
    BridgeSplitError::DamagedCutRim {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::build_closed_cut_loops;
    use crate::{EditVertex, MeshEditBuffers, MeshTopology};

    #[test]
    fn surface_recovery_keeps_a_closed_cycle_when_another_path_hits_a_border() {
        let mesh = MeshEditBuffers {
            vertices: vec![
                EditVertex::at([0.0, 0.0, 0.0]),
                EditVertex::at([1.0, 0.0, 0.0]),
                EditVertex::at([1.0, 1.0, 0.0]),
                EditVertex::at([0.0, 1.0, 0.0]),
                EditVertex::at([3.0, 0.0, 0.0]),
                EditVertex::at([4.0, 0.0, 0.0]),
            ],
            indices: Vec::new(),
            topology: MeshTopology::TriangleMesh,
        };
        let loops = build_closed_cut_loops(&mesh, &[[0, 1], [1, 2], [2, 3], [3, 0], [4, 5]])
            .expect("the complete cycle is recoverable");

        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
    }

    #[test]
    fn surface_recovery_does_not_cap_a_branching_component() {
        let mesh = MeshEditBuffers {
            vertices: vec![
                EditVertex::at([0.0, 0.0, 0.0]),
                EditVertex::at([1.0, 0.0, 0.0]),
                EditVertex::at([1.0, 1.0, 0.0]),
                EditVertex::at([0.0, 1.0, 0.0]),
                EditVertex::at([0.5, 0.5, 0.0]),
            ],
            indices: Vec::new(),
            topology: MeshTopology::TriangleMesh,
        };
        assert!(build_closed_cut_loops(&mesh, &[[0, 1], [1, 2], [2, 3], [3, 0], [0, 4],]).is_err());
    }
}
