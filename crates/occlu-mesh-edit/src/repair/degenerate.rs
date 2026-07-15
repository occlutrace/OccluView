//! Passes 2+3: degenerate triangles and duplicate faces.

use glam::DVec3;

use super::RepairReport;
use crate::{EditVertex, MeshEditBuffers};

/// A triangle counts as zero-area when the sine of its sharpest wedge falls
/// below this — RELATIVE to its own edge lengths, so tiny-but-healthy
/// triangles on fine scans survive while true slivers of any scale die.
const DEGENERATE_SIN: f64 = 1e-5;

/// Pass 2: drop triangles with repeated indices (post-weld) or relatively
/// zero area.
pub(super) fn remove_degenerate_triangles(mesh: &mut MeshEditBuffers, report: &mut RepairReport) {
    let mut kept = Vec::with_capacity(mesh.indices.len());
    let mut removed = 0_usize;
    for tri in mesh.indices.chunks_exact(3) {
        if is_degenerate_triangle(&mesh.vertices, tri) {
            removed += 1;
        } else {
            kept.extend_from_slice(tri);
        }
    }
    if removed > 0 {
        mesh.indices = kept;
        report.removed_degenerate_triangles += removed;
    }
}

fn is_degenerate_triangle(vertices: &[EditVertex], tri: &[u32]) -> bool {
    if tri[0] == tri[1] || tri[1] == tri[2] || tri[2] == tri[0] {
        return true;
    }
    let a = position(vertices, tri[0]);
    let b = position(vertices, tri[1]);
    let c = position(vertices, tri[2]);
    let longest_sq = (b - a)
        .length_squared()
        .max((c - a).length_squared())
        .max((c - b).length_squared());
    if longest_sq <= 0.0 {
        // All three positions coincide (distinct only in attributes).
        return true;
    }
    let cross_sq = (b - a).cross(c - a).length_squared();
    cross_sq <= (DEGENERATE_SIN * longest_sq).powi(2)
}

fn position(vertices: &[EditVertex], index: u32) -> DVec3 {
    let p = vertices[index as usize].position;
    DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
}

/// Pass 3: drop duplicate faces. Triangles are canonicalized to their sorted
/// index triple, so same- and opposite-winding duplicates both collapse onto
/// the first (lowest-index) occurrence.
pub(super) fn remove_duplicate_triangles(mesh: &mut MeshEditBuffers, report: &mut RepairReport) {
    let triangle_count = mesh.triangle_count();
    if triangle_count < 2 {
        return;
    }

    let mut keyed: Vec<([u32; 3], usize)> = mesh
        .indices
        .chunks_exact(3)
        .enumerate()
        .map(|(triangle, tri)| {
            let mut key = [tri[0], tri[1], tri[2]];
            key.sort_unstable();
            (key, triangle)
        })
        .collect();
    keyed.sort_unstable();

    let mut remove = vec![false; triangle_count];
    let mut removed = 0_usize;
    for pair in keyed.windows(2) {
        if pair[0].0 == pair[1].0 {
            remove[pair[1].1] = true;
            removed += 1;
        }
    }
    if removed == 0 {
        return;
    }

    let mut kept = Vec::with_capacity((triangle_count - removed) * 3);
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if !remove[triangle] {
            kept.extend_from_slice(tri);
        }
    }
    mesh.indices = kept;
    report.removed_duplicate_triangles += removed;
}
