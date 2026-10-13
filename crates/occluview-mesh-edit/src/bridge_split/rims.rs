use std::collections::{BTreeMap, BTreeSet};

use super::clip::{ClipProvenance, LineageKey};
use crate::topology::canonical_position_key;
use crate::{BridgeSplitError, MeshEditBuffers, MeshEditError};

type PositionKey = [u32; 3];
type CutEdge = (PositionKey, PositionKey);
type CutAdjacency = BTreeMap<PositionKey, BTreeSet<PositionKey>>;

struct CutGraph {
    representative: BTreeMap<PositionKey, usize>,
    adjacency: CutAdjacency,
    edges: BTreeSet<CutEdge>,
    lineage_representatives: BTreeMap<PositionKey, BTreeMap<LineageKey, usize>>,
    ambiguous_positions: BTreeSet<PositionKey>,
    ambiguous_edges: BTreeSet<CutEdge>,
    has_lineage: bool,
}

/// Which reading of a cut rim the caller wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CutPolicy {
    /// Refuse a segment that collapsed to a geometric point, an opposing
    /// duplicate, a branch or an open rim. Used for closed-solid splitting,
    /// where dropping a generated edge would hide a topology defect.
    Strict,
    /// Skip a segment that collapsed to a point. Open and branching components
    /// are the caller's business, not a defect.
    Lenient,
}

/// The single reading of `cut_edges` both walkers start from.
struct CutScan {
    /// The lowest vertex index seen at each geometric position, so a walk can
    /// name a position by a vertex that exists in the mesh.
    representative: BTreeMap<PositionKey, usize>,
    /// Segments as generated, in the direction the caller listed them. The
    /// recovery path canonicalises this set afterwards.
    directed: BTreeSet<CutEdge>,
    lineage_representatives: BTreeMap<PositionKey, BTreeMap<LineageKey, usize>>,
    ambiguous_positions: BTreeSet<PositionKey>,
    ambiguous_edges: BTreeSet<CutEdge>,
    has_lineage: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CutLoop {
    pub(crate) vertices: Vec<usize>,
    pub(crate) lineages: Vec<Option<LineageKey>>,
}

/// Read the cut edges against the mesh once.
///
/// Both walkers need the same three things — the vertex range check, the
/// canonical position key and the lowest-index representative — and they were
/// built twice. Only the treatment of a collapsed segment differs, which is what
/// [`CutPolicy`] selects.
fn collect_cut_scan(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: Option<&ClipProvenance>,
    policy: CutPolicy,
) -> Result<CutScan, BridgeSplitError> {
    if cut_edges.is_empty() {
        return Err(damaged("no cut edges were generated"));
    }

    let mut representative: BTreeMap<PositionKey, usize> = BTreeMap::new();
    let mut directed = BTreeSet::new();
    let mut lineage_representatives = BTreeMap::new();
    let mut ambiguous_positions = BTreeSet::new();
    let mut ambiguous_edges = BTreeSet::new();
    let mut edge_occurrences = BTreeMap::<CutEdge, usize>::new();
    if provenance.is_some_and(|provenance| provenance.cut_edge_lineages.len() != cut_edges.len()) {
        return Err(MeshEditError::MalformedMesh {
            reason: "cut edge lineage count does not match generated cut edges".to_string(),
        }
        .into());
    }
    for (edge_index, &[from, to]) in cut_edges.iter().enumerate() {
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
            if policy == CutPolicy::Strict {
                return Err(damaged("cut edge collapsed to one geometric point"));
            }
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
        directed.insert((from_key, to_key));
        if let Some(provenance) = provenance {
            let [from_lineage, to_lineage] = provenance.cut_edge_lineages[edge_index];
            for (position, vertex_index, lineage) in [
                (from_key, from as usize, from_lineage),
                (to_key, to as usize, to_lineage),
            ] {
                if lineage.ambiguous {
                    ambiguous_positions.insert(position);
                }
                lineage_representatives
                    .entry(position)
                    .or_insert_with(BTreeMap::new)
                    .entry(lineage.key)
                    .and_modify(|representative| {
                        *representative = (*representative).min(vertex_index)
                    })
                    .or_insert(vertex_index);
            }
            let edge = ordered_edge(from_key, to_key);
            let occurrences = edge_occurrences.entry(edge).or_default();
            *occurrences += 1;
            if *occurrences > 1 {
                ambiguous_edges.insert(edge);
            }
        }
    }

    Ok(CutScan {
        representative,
        directed,
        lineage_representatives,
        ambiguous_positions,
        ambiguous_edges,
        has_lineage: provenance.is_some(),
    })
}

#[cfg(test)]
pub(crate) fn build_cut_loops(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
) -> Result<Vec<Vec<usize>>, BridgeSplitError> {
    build_cut_loops_internal(mesh, cut_edges, None)
        .map(|loops| loops.into_iter().map(|ring| ring.vertices).collect())
}

pub(crate) fn build_cut_loops_with_provenance(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: &ClipProvenance,
) -> Result<Vec<CutLoop>, BridgeSplitError> {
    build_cut_loops_internal(mesh, cut_edges, Some(provenance))
}

fn build_cut_loops_internal(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: Option<&ClipProvenance>,
) -> Result<Vec<CutLoop>, BridgeSplitError> {
    let CutScan {
        representative,
        directed,
        lineage_representatives,
        ambiguous_positions,
        ambiguous_edges,
        has_lineage,
    } = collect_cut_scan(mesh, cut_edges, provenance, CutPolicy::Strict)?;

    if has_lineage && !ambiguous_edges.is_empty() {
        return Err(damaged("cut rim contains an ambiguous source edge"));
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
        // `start` came from these keys and `outgoing` is never mutated below.
        let first_next = outgoing[&start];
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
        let ring = finish_lineage_loop(
            mesh,
            ring,
            has_lineage,
            &lineage_representatives,
            &ambiguous_positions,
            true,
        )?
        .ok_or_else(|| damaged("cut rim has ambiguous source lineage"))?;
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
#[cfg(test)]
pub(crate) fn build_closed_cut_loops(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
) -> Result<Vec<Vec<usize>>, BridgeSplitError> {
    build_closed_cut_loops_internal(mesh, cut_edges, None)
        .map(|loops| loops.into_iter().map(|ring| ring.vertices).collect())
}

pub(crate) fn build_closed_cut_loops_with_provenance(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: &ClipProvenance,
) -> Result<Vec<CutLoop>, BridgeSplitError> {
    build_closed_cut_loops_internal(mesh, cut_edges, Some(provenance))
}

fn build_closed_cut_loops_internal(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: Option<&ClipProvenance>,
) -> Result<Vec<CutLoop>, BridgeSplitError> {
    let graph = build_surface_cut_graph(mesh, cut_edges, provenance)?;
    let CutGraph {
        representative,
        adjacency,
        edges,
        lineage_representatives,
        ambiguous_positions,
        ambiguous_edges,
        has_lineage,
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
        if component_edges
            .iter()
            .any(|edge| ambiguous_edges.contains(edge))
        {
            continue;
        }
        let vertices = walk_closed_component(
            &component_vertices,
            &adjacency,
            &representative,
        )?;
        if let Some(ring) = finish_lineage_loop(
            mesh,
            vertices,
            has_lineage,
            &lineage_representatives,
            &ambiguous_positions,
            false,
        )? {
            loops.push(ring);
        }
    }

    if loops.is_empty() {
        return Err(damaged("no complete closed cut rim could be recovered"));
    }
    Ok(loops)
}

fn build_surface_cut_graph(
    mesh: &MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: Option<&ClipProvenance>,
) -> Result<CutGraph, BridgeSplitError> {
    let CutScan {
        representative,
        directed,
        lineage_representatives,
        ambiguous_positions,
        ambiguous_edges,
        has_lineage,
    } = collect_cut_scan(mesh, cut_edges, provenance, CutPolicy::Lenient)?;
    let edges: BTreeSet<CutEdge> = directed
        .into_iter()
        .map(|(from, to)| ordered_edge(from, to))
        .collect();
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
        lineage_representatives,
        ambiguous_positions,
        ambiguous_edges,
        has_lineage,
    })
}

fn finish_lineage_loop(
    mesh: &MeshEditBuffers,
    mut vertices: Vec<usize>,
    has_lineage: bool,
    lineage_representatives: &BTreeMap<PositionKey, BTreeMap<LineageKey, usize>>,
    ambiguous_positions: &BTreeSet<PositionKey>,
    strict: bool,
) -> Result<Option<CutLoop>, BridgeSplitError> {
    let mut lineages = Vec::with_capacity(vertices.len());
    if !has_lineage {
        lineages.resize(vertices.len(), None);
        return Ok(Some(CutLoop { vertices, lineages }));
    }

    for vertex in &mut vertices {
        let point = mesh
            .vertices
            .get(*vertex)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: "cut rim vertex is out of range while reading lineage".to_string(),
            })?;
        let position = canonical_position_key(point.position);
        let Some(candidates) = lineage_representatives.get(&position) else {
            if strict {
                return Err(damaged("cut rim lost source lineage at a vertex"));
            }
            return Ok(None);
        };
        if ambiguous_positions.contains(&position) || candidates.len() != 1 {
            if strict {
                return Err(damaged("cut rim has ambiguous source lineage at a vertex"));
            }
            return Ok(None);
        }
        let Some((&lineage, &representative)) = candidates.iter().next() else {
            if strict {
                return Err(damaged("cut rim has no source lineage at a vertex"));
            }
            return Ok(None);
        };
        *vertex = representative;
        lineages.push(Some(lineage));
    }
    Ok(Some(CutLoop { vertices, lineages }))
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
