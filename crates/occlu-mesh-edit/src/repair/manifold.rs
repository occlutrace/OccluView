//! Passes 4+5: split non-manifold edges and bowtie vertices.
//!
//! Both passes only DUPLICATE vertices (exact payload copies, zero
//! displacement) and re-point indices — no position ever moves and no
//! triangle is dropped.

use super::{edge_incidence, triangle_area_f64, RepairReport};
use crate::pinch::{push_duplicate_vertex, split_vertex_fans};
use crate::{MeshEditBuffers, MeshEditError};

/// Pass 4: for every undirected edge with more than two incident faces, keep
/// the two largest-area faces on the original edge and re-point each extra
/// face onto duplicated copies of the edge's two vertices (split-to-border).
pub(super) fn split_nonmanifold_edges(
    mesh: &mut MeshEditBuffers,
    report: &mut RepairReport,
) -> Result<(), MeshEditError> {
    let incidence = edge_incidence(&mesh.indices);
    // (triangle, original vertex) pairs to detach; deduplicated so a face
    // that is extra on two edges sharing a vertex duplicates it only once.
    let mut detach: Vec<(usize, u32)> = Vec::new();

    let mut run_start = 0;
    while run_start < incidence.len() {
        let mut run_end = run_start + 1;
        while run_end < incidence.len() && incidence[run_end].0 == incidence[run_start].0 {
            run_end += 1;
        }
        if run_end - run_start > 2 {
            report.split_nonmanifold_edges += 1;
            let (a, b) = incidence[run_start].0;
            let mut faces: Vec<(f64, usize)> = incidence[run_start..run_end]
                .iter()
                .map(|&(_, triangle)| (triangle_area_f64(mesh, triangle), triangle))
                .collect();
            // Largest area first; ties break on the lower triangle index.
            faces.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
            for &(_, triangle) in &faces[2..] {
                detach.push((triangle, a));
                detach.push((triangle, b));
            }
        }
        run_start = run_end;
    }
    if detach.is_empty() {
        return Ok(());
    }
    detach.sort_unstable();
    detach.dedup();

    // Match against a snapshot: once a slot is re-pointed to a duplicate, its
    // original value is gone from the live buffer.
    let snapshot = mesh.indices.clone();
    for &(triangle, vertex) in &detach {
        let duplicate = push_duplicate_vertex(mesh, vertex)?;
        let base = triangle * 3;
        for (slot, &original) in mesh.indices[base..base + 3]
            .iter_mut()
            .zip(&snapshot[base..base + 3])
        {
            if original == vertex {
                *slot = duplicate;
            }
        }
    }
    Ok(())
}

/// Pass 5: cluster each vertex's incident faces by edge-connectivity THROUGH
/// that vertex; the cluster containing the lowest triangle index keeps the
/// original vertex, every other cluster gets a duplicated copy.
pub(super) fn split_bowtie_vertices(
    mesh: &mut MeshEditBuffers,
    report: &mut RepairReport,
) -> Result<(), MeshEditError> {
    let mut incidence: Vec<(u32, usize)> = Vec::with_capacity(mesh.indices.len());
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        for &vertex in tri {
            incidence.push((vertex, triangle));
        }
    }
    incidence.sort_unstable();

    let mut run_start = 0;
    while run_start < incidence.len() {
        let vertex = incidence[run_start].0;
        let mut run_end = run_start + 1;
        while run_end < incidence.len() && incidence[run_end].0 == vertex {
            run_end += 1;
        }
        if run_end - run_start >= 2 {
            let faces: Vec<usize> = incidence[run_start..run_end]
                .iter()
                .map(|&(_, triangle)| triangle)
                .collect();
            // Shared fan-clustering split (see `crate::pinch`). Each vertex
            // that resolves into more than one fan counts once.
            if split_vertex_fans(mesh, vertex, &faces)? {
                report.split_bowtie_vertices += 1;
            }
        }
        run_start = run_end;
    }
    Ok(())
}
