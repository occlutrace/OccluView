use std::collections::{BTreeMap, BTreeSet};

use crate::topology::canonical_position_key;
use crate::{BridgeSplitError, MeshEditBuffers, MeshEditError};

type PositionKey = [u32; 3];

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

fn damaged(reason: &str) -> BridgeSplitError {
    BridgeSplitError::DamagedCutRim {
        reason: reason.to_string(),
    }
}
