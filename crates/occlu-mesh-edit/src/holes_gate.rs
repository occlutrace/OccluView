//! Fill gating: loop collection (with pinch-merge splitting), the scan-border
//! guard, size caps, and selection-majority qualification. Split out of
//! `holes.rs` (file-size budget).

use std::collections::HashSet;

use glam::Vec3;

use super::holes_walk::{
    split_loop_at_coincident_positions, vertex_position, walk_boundary_loop, BoundaryNextMap,
    BoundaryOwnerMap,
};
use super::{FaceSelection, MeshEditBuffers, MeshEditError, MeshEditOptions};
use crate::holes::{FillLoopStats, SELECTION_MAX_BOUNDARY_LOOP};

/// Border guard: a rim must reach this fraction of the LARGEST rim's
/// perimeter to count as scan border.
const BORDER_RIM_RATIO: f64 = 0.5;

/// Border guard: a rim must additionally reach this fraction of the mesh
/// bounding-box diagonal — a small absolute rim is a hole, never a border,
/// even when it happens to be the largest one (closed mesh with pinholes).
const BORDER_BBOX_FRACTION: f64 = 0.5;

/// Walk every boundary chain into loops (skipping already-visited starts),
/// splitting merged pinch loops at coincident-position revisits, and pairing
/// each loop with its mm perimeter. Failed / too-short chains are tallied as
/// degenerate in `stats`.
pub(super) fn collect_boundary_loops(
    mesh: &MeshEditBuffers,
    next_boundary_vertex: &BoundaryNextMap,
    boundary_starts: &[usize],
    stats: &mut FillLoopStats,
) -> Result<Vec<(Vec<usize>, f64)>, MeshEditError> {
    let mut visited = HashSet::new();
    let mut loops: Vec<(Vec<usize>, f64)> = Vec::new();
    for &start in boundary_starts {
        if visited.contains(&start) {
            continue;
        }
        let Some(boundary_loop) = walk_boundary_loop(
            start,
            next_boundary_vertex,
            mesh.vertices.len(),
            &mut visited,
        ) else {
            // Non-simple / numerically stalled chain: not a fillable loop.
            stats.skipped_degenerate += 1;
            continue;
        };
        if boundary_loop.len() < 3 {
            stats.skipped_degenerate += 1;
            continue;
        }
        // A hole pinched onto the border (or onto another hole) walks as one
        // merged loop through duplicated junction copies; split it back into
        // the operator-visible sub-loops so a small hole at the scan edge is
        // never mistaken for the border itself.
        for part in split_loop_at_coincident_positions(mesh, boundary_loop) {
            let perimeter = rim_perimeter_mm(mesh, &part)?;
            loops.push((part, perimeter));
        }
    }
    Ok(loops)
}

/// The perimeter at or above which a rim counts as the scan's natural outer
/// boundary: at least [`BORDER_RIM_RATIO`] of the largest rim's perimeter AND
/// at least [`BORDER_BBOX_FRACTION`] of the referenced bounding-box diagonal.
/// The absolute anchor keeps a closed-but-pinholed mesh fillable: without it,
/// the largest PINHOLE would masquerade as "the border" and stay open.
pub(super) fn border_perimeter_threshold(
    mesh: &MeshEditBuffers,
    loops: &[(Vec<usize>, f64)],
) -> f64 {
    let largest = loops
        .iter()
        .map(|(_, perimeter)| *perimeter)
        .fold(0.0_f64, f64::max);
    let absolute_floor = f64::from(referenced_bbox_diagonal(mesh)) * BORDER_BBOX_FRACTION;
    (largest * BORDER_RIM_RATIO).max(absolute_floor)
}

/// Diagonal of the bounding box of REFERENCED vertices (unreferenced debris
/// must not inflate the border guard's absolute anchor).
fn referenced_bbox_diagonal(mesh: &MeshEditBuffers) -> f32 {
    let mut referenced = vec![false; mesh.vertices.len()];
    for &index in &mesh.indices {
        if let Some(slot) = referenced.get_mut(index as usize) {
            *slot = true;
        }
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut any = false;
    for (vertex, &used) in mesh.vertices.iter().zip(&referenced) {
        if used {
            let p = Vec3::from_array(vertex.position);
            lo = lo.min(p);
            hi = hi.max(p);
            any = true;
        }
    }
    if !any {
        return 0.0;
    }
    let diagonal = (hi - lo).length();
    if diagonal.is_finite() {
        diagonal
    } else {
        0.0
    }
}

/// Whether a rim is too large to cap under the effective size policy: the edge
/// ceiling (lifted for selection-scoped intent) always applies; the optional
/// mm perimeter restraint applies only to the whole-mesh (unselected) path.
pub(super) fn rim_exceeds_size_cap(
    boundary_loop: &[usize],
    perimeter_mm: f64,
    has_selection: bool,
    options: MeshEditOptions,
) -> bool {
    let edge_cap = if has_selection {
        options.max_boundary_loop.max(SELECTION_MAX_BOUNDARY_LOOP)
    } else {
        options.max_boundary_loop
    };
    if boundary_loop.len() > edge_cap {
        return true;
    }
    if !has_selection {
        if let Some(limit_mm) = options.max_rim_perimeter_mm {
            if perimeter_mm > f64::from(limit_mm) {
                return true;
            }
        }
    }
    false
}

/// A rim qualifies for selection-scoped filling when at least half of its
/// owning faces are selected. Half of the rim being explicitly marked is
/// unambiguous intent, yet a stray selection that only clips a small share of a
/// large unrelated rim stays well under the threshold and is refused.
pub(super) fn rim_selection_qualifies(
    boundary_loop: &[usize],
    owner_by_edge: &BoundaryOwnerMap,
    selection: &FaceSelection,
) -> bool {
    let loop_len = boundary_loop.len();
    if loop_len == 0 {
        return false;
    }
    let selected = (0..loop_len)
        .filter(|&index| {
            let a = boundary_loop[index];
            let b = boundary_loop[(index + 1) % loop_len];
            owner_by_edge
                .get(&(a, b))
                .is_some_and(|owner| selection.as_slice()[*owner])
        })
        .count();
    // `selected / loop_len >= 0.5`, done in integers to stay exact.
    2 * selected >= loop_len
}

/// Sum of a rim's edge lengths, widened to `f64` for a stable mm comparison.
fn rim_perimeter_mm(mesh: &MeshEditBuffers, boundary_loop: &[usize]) -> Result<f64, MeshEditError> {
    let loop_len = boundary_loop.len();
    let mut perimeter = 0.0_f64;
    for index in 0..loop_len {
        let current = vertex_position(mesh, boundary_loop[index])?;
        let next = vertex_position(mesh, boundary_loop[(index + 1) % loop_len])?;
        perimeter += f64::from((current - next).length());
    }
    Ok(perimeter)
}
