//! Shared vertex-fan splitting (bowtie / boundary-pinch resolution).
//!
//! A single vertex can be shared by two otherwise-disjoint face fans (a
//! "bowtie" vertex) — the repair pipeline splits these so the mesh becomes
//! edge-manifold, and hole filling splits the BOUNDARY-junction subset so two
//! rims that meet at one vertex become independent simple loops that both fill.
//!
//! Both callers only DUPLICATE vertices (exact payload copies, zero
//! displacement) and re-point indices — no position ever moves and no triangle
//! is dropped.

use std::collections::{HashMap, HashSet};

use crate::topology_analysis::vertex_fan_clusters;
use crate::{MeshEditBuffers, MeshEditError};

/// Append an exact copy of `vertex` and return its index.
pub(crate) fn push_duplicate_vertex(
    mesh: &mut MeshEditBuffers,
    vertex: u32,
) -> Result<u32, MeshEditError> {
    let duplicate =
        u32::try_from(mesh.vertices.len()).map_err(|_| MeshEditError::MalformedMesh {
            reason: "split vertex count exceeds u32::MAX".to_string(),
        })?;
    let copy = mesh.vertices[vertex as usize];
    mesh.vertices.push(copy);
    Ok(duplicate)
}

/// Cluster `faces` (all incident to `vertex`) by shared edges THROUGH that
/// vertex; the cluster containing the lowest triangle index keeps the original
/// vertex, every other cluster gets a duplicated copy. Returns whether the
/// vertex was actually split (more than one cluster).
pub(crate) fn split_vertex_fans(
    mesh: &mut MeshEditBuffers,
    vertex: u32,
    faces: &[usize],
) -> Result<bool, MeshEditError> {
    let (cluster_of, cluster_count) = vertex_fan_clusters(&mesh.indices, vertex, faces);
    if cluster_count <= 1 {
        return Ok(false);
    }

    let mut replacement = vec![vertex; cluster_count];
    for slot in replacement.iter_mut().skip(1) {
        *slot = push_duplicate_vertex(mesh, vertex)?;
    }
    for (local, &triangle) in faces.iter().enumerate() {
        let target = replacement[cluster_of[local]];
        if target == vertex {
            continue;
        }
        for slot in triangle * 3..triangle * 3 + 3 {
            if mesh.indices[slot] == vertex {
                mesh.indices[slot] = target;
            }
        }
    }
    Ok(true)
}

/// Split every BOUNDARY-junction vertex per incident fan so adjacent rims that
/// meet at a single vertex become independent simple loops. A boundary
/// junction is a vertex whose boundary in- or out-degree exceeds one (two rims,
/// or a pinch, pass through it).
///
/// Returns the rewritten mesh plus the number of vertices split, or `None` when
/// nothing pinches: a clean/closed mesh — and any input already bowtie-split by
/// the repair pipeline — passes through untouched, so the caller's downstream
/// path stays byte-for-byte identical.
///
/// # Errors
/// Returns [`MeshEditError`] only on index overflow while duplicating a vertex.
pub(crate) fn split_boundary_pinch_vertices(
    mesh: &MeshEditBuffers,
) -> Result<Option<(MeshEditBuffers, usize)>, MeshEditError> {
    let junctions = boundary_junction_vertices(mesh);
    if junctions.is_empty() {
        return Ok(None);
    }

    let mut work = mesh.clone();
    // Sorted (vertex, triangle) incidence; runs of equal vertices are that
    // vertex's incident faces in ascending triangle order (mirrors repair's
    // bowtie sweep so the split is deterministic).
    let mut incidence: Vec<(u32, usize)> = Vec::with_capacity(work.indices.len());
    for (triangle, tri) in work.indices.chunks_exact(3).enumerate() {
        for &vertex in tri {
            incidence.push((vertex, triangle));
        }
    }
    incidence.sort_unstable();

    let mut split_count = 0_usize;
    let mut run_start = 0;
    while run_start < incidence.len() {
        let vertex = incidence[run_start].0;
        let mut run_end = run_start + 1;
        while run_end < incidence.len() && incidence[run_end].0 == vertex {
            run_end += 1;
        }
        if junctions.contains(&vertex) {
            let faces: Vec<usize> = incidence[run_start..run_end]
                .iter()
                .map(|&(_, triangle)| triangle)
                .collect();
            if split_vertex_fans(&mut work, vertex, &faces)? {
                split_count += 1;
            }
        }
        run_start = run_end;
    }

    if split_count == 0 {
        return Ok(None);
    }
    Ok(Some((work, split_count)))
}

/// Vertices whose boundary in- or out-degree exceeds one: two rims meet there,
/// so the boundary walk cannot pick a unique successor and both rims stall.
fn boundary_junction_vertices(mesh: &MeshEditBuffers) -> HashSet<u32> {
    let mut directed: HashSet<(u32, u32)> = HashSet::with_capacity(mesh.indices.len());
    for tri in mesh.indices.chunks_exact(3) {
        for edge in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            directed.insert(edge);
        }
    }

    let mut out_degree: HashMap<u32, u32> = HashMap::new();
    let mut in_degree: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) {
            *out_degree.entry(a).or_default() += 1;
            *in_degree.entry(b).or_default() += 1;
        }
    }

    let mut junctions = HashSet::new();
    for (&vertex, &degree) in out_degree.iter().chain(in_degree.iter()) {
        if degree > 1 {
            junctions.insert(vertex);
        }
    }
    junctions
}
