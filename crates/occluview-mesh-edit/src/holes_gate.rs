//! Fill gating: loop collection (with pinch-merge splitting), the scan-border
//! guard, size caps, and whether a mark holds a rim.

use std::collections::{HashMap, HashSet};

use glam::Vec3;

use super::holes_region::MarkRings;
use super::holes_walk::{
    split_loop_at_coincident_positions, vertex_position, walk_boundary_loop, BoundaryNextMap,
    BoundaryOwners, BoundaryWalk,
};
use super::{MeshEditBuffers, MeshEditError, MeshEditOptions};
use crate::holes::{FillLoopStats, CLOSE_HOLES_EDGE_CEILING};

/// Border guard: a rim must reach this fraction of the largest rim's
/// perimeter to count as scan border.
const BORDER_RIM_RATIO: f64 = 0.5;

/// Border guard: a rim must additionally reach this fraction of the mesh
/// bounding-box diagonal — a small absolute rim is a hole, never a border,
/// even when it happens to be the largest one (closed mesh with pinholes).
const BORDER_BBOX_FRACTION: f64 = 0.5;

/// Border guard, sole-rim case: with only one rim the ratio test carries no
/// information (a rim is always at least half of itself), so the decision rests
/// entirely on the absolute anchor — and half the diagonal is far too eager
/// there. A scan border is the outline of an open sheet and runs close to the
/// model's full extent; a molar socket on a closed model is 25-35 mm perimeter
/// on a 65-75 mm arch, which a half-diagonal anchor declares "border" and
/// refuses to fill. Requiring the full diagonal keeps a genuine open-sheet border out
/// while letting a socket close.
const SOLE_RIM_BBOX_FRACTION: f64 = 1.0;

/// What a mark says about the vertices of the region cut out around it.
pub(super) struct MarkedVertices {
    /// Corners of marked faces: damage is reported where the operator pointed.
    pub(super) marked_corners: Vec<bool>,
    /// Corners of the region's outermost faces. The surface goes on past
    /// them, so a chain of boundary edges cannot be followed any further there.
    pub(super) outermost_corners: Vec<bool>,
}

/// Walk every boundary chain into loops (skipping already-visited starts),
/// splitting merged pinch loops at coincident-position revisits, and pairing
/// each loop with its mm perimeter. Failed / too-short chains are tallied as
/// degenerate in `stats`.
///
/// With `marked` the mesh is the region around a mark, and a chain that does
/// not close is damage only when it starts on a corner of a marked face and
/// stays inside the region: a chain that runs to the region's edge is a rim
/// that leaves the marked area, which the mark touches at most.
pub(super) fn collect_boundary_loops(
    mesh: &MeshEditBuffers,
    next_boundary_vertex: &BoundaryNextMap,
    boundary_starts: &[usize],
    marked: Option<&MarkedVertices>,
    stats: &mut FillLoopStats,
) -> Result<Vec<(Vec<usize>, f64)>, MeshEditError> {
    let flagged = |flags: &[bool], vertex: usize| flags.get(vertex).copied().unwrap_or(false);
    let mut visited = HashSet::new();
    // Rims that run to the region's edge: for each vertex of one, which rim,
    // and per rim whether it has been counted as partly marked. A rim is
    // walked in stretches, the one that reaches the edge first and the ones
    // behind it as they run into it.
    let mut leaving: HashMap<usize, usize> = HashMap::new();
    let mut counted: Vec<bool> = Vec::new();
    let mut loops: Vec<(Vec<usize>, f64)> = Vec::new();
    for &start in boundary_starts {
        if visited.contains(&start) {
            continue;
        }
        let pointed_at = marked.is_none_or(|marked| flagged(&marked.marked_corners, start));
        let boundary_loop = match walk_boundary_loop(
            start,
            next_boundary_vertex,
            mesh.vertices.len(),
            &mut visited,
        ) {
            BoundaryWalk::Rim(boundary_loop) => boundary_loop,
            BoundaryWalk::Open { path, stopped_at } => {
                let at_edge =
                    marked.is_some_and(|marked| flagged(&marked.outermost_corners, stopped_at));
                let rim = match leaving.get(&stopped_at) {
                    Some(&rim) => Some(rim),
                    None if at_edge => {
                        counted.push(false);
                        Some(counted.len() - 1)
                    }
                    None => None,
                };
                if let Some(rim) = rim {
                    let touched = marked.is_some_and(|marked| {
                        path.iter()
                            .any(|&vertex| flagged(&marked.marked_corners, vertex))
                    });
                    if touched && !std::mem::replace(&mut counted[rim], true) {
                        stats.skipped_partial += 1;
                    }
                    leaving.extend(path.into_iter().map(|vertex| (vertex, rim)));
                } else if pointed_at {
                    // Non-simple / numerically stalled chain: not a fillable
                    // loop.
                    stats.skipped_degenerate += 1;
                }
                continue;
            }
        };
        if boundary_loop.len() < 3 {
            stats.skipped_degenerate += usize::from(pointed_at);
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
/// boundary: at least [`BORDER_RIM_RATIO`] of the largest rim's perimeter and
/// at least [`BORDER_BBOX_FRACTION`] of the referenced bounding-box diagonal.
/// The absolute anchor keeps a closed-but-pinholed mesh fillable: without it,
/// the largest pinhole would masquerade as "the border" and stay open.
///
/// With a single rim the ratio half of that rule is vacuous — a rim is always
/// at least half of itself — so the anchor alone decides, and it tightens to
/// [`SOLE_RIM_BBOX_FRACTION`]. See that constant for why.
pub(super) fn border_perimeter_threshold(
    mesh: &MeshEditBuffers,
    loops: &[(Vec<usize>, f64)],
) -> f64 {
    let diagonal = f64::from(referenced_bbox_diagonal(mesh));
    if loops.len() <= 1 {
        return diagonal * SOLE_RIM_BBOX_FRACTION;
    }
    let largest = loops
        .iter()
        .map(|(_, perimeter)| *perimeter)
        .fold(0.0_f64, f64::max);
    (largest * BORDER_RIM_RATIO).max(diagonal * BORDER_BBOX_FRACTION)
}

/// Diagonal of the bounding box of referenced vertices (unreferenced debris
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
/// ceiling (lifted for selection-scoped intent) always applies, as does an
/// explicitly supplied mm perimeter restraint.
pub(super) fn rim_exceeds_size_cap(
    boundary_loop: &[usize],
    perimeter_mm: f64,
    has_selection: bool,
    options: MeshEditOptions,
) -> bool {
    let edge_cap = if has_selection {
        options.max_boundary_loop.max(CLOSE_HOLES_EDGE_CEILING)
    } else {
        options.max_boundary_loop
    };
    if boundary_loop.len() > edge_cap {
        return true;
    }
    options
        .max_rim_perimeter_mm
        .is_some_and(|limit_mm| perimeter_mm > f64::from(limit_mm))
}

/// What a mark has of a rim.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum RimHold {
    /// The mark holds the rim: it closes.
    Held,
    /// The mark touches the rim without holding it.
    Partial,
    /// No face of the rim is marked.
    Untouched,
}

/// Whether the operator's mark holds a rim: at least half of the rim's faces
/// are marked, and none of them lies beyond reach of the mark.
///
/// Half of the rim being explicitly marked is unambiguous intent, and with the
/// reach it leaves room for the faces a lasso misses on a rim it encloses: the
/// ones that look away from the camera in surface mode. A rim that runs out of
/// the marked area is not held however much of it is marked: the hole is then
/// only partly inside the mark, and closing it would close what the operator
/// did not select.
pub(super) fn rim_hold(
    boundary_loop: &[usize],
    owner_by_edge: &BoundaryOwners,
    rings: &MarkRings,
) -> RimHold {
    let loop_len = boundary_loop.len();
    let (mut marked, mut out_of_reach) = (0_usize, false);
    for index in 0..loop_len {
        let a = boundary_loop[index];
        let b = boundary_loop[(index + 1) % loop_len];
        // The zero-length edge between the two copies of a split junction has
        // no face; it counts toward the rim's length and is never marked.
        let Some(owner) = owner_by_edge.owner(a, b) else {
            continue;
        };
        marked += usize::from(rings.is_marked(owner));
        out_of_reach |= !rings.is_near(owner);
    }
    if marked == 0 {
        RimHold::Untouched
    // `marked / loop_len >= 0.5`, done in integers to stay exact.
    } else if out_of_reach || 2 * marked < loop_len {
        RimHold::Partial
    } else {
        RimHold::Held
    }
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

/// Refuse a triangle soup whose corners are still not shared.
///
/// In index space every edge of every triangle reads as a boundary: each
/// triangle becomes its own three-edge rim, and the duplicate check each rim
/// runs scans all triangles. That is quadratic, and it does not finish -- a
/// 500k-triangle soup was still running after ten minutes.
///
/// The test is whether the indices share vertices, and it is asked of the
/// post-weld mesh. Two simpler keys give the wrong answer:
///
/// * The `heal_boundary_rims` flag: healing welds by full payload (position
///   and colour/UV bits), so a soup whose coincident corners carry different
///   payloads merges nothing, and the healing pass would classify all three
///   edges of every triangle as an isolated nick and delete the whole mesh.
///   Every OBJ is such a soup: the reader pushes one vertex per face corner and
///   never dedups.
/// * The vertex/index lengths cannot see the weld at all:
///   `weld_soup_topology` clones the vertex array and remaps only the indices, so
///   a soup has `vertices.len() == indices.len()` before and after a successful
///   weld. Keying on lengths refuses every large soup, including the weldable
///   binary-STL arch Close Holes exists to heal.
///
/// A welded surface points many corners at the same vertex; a soup gives every
/// corner its own. Counting distinct referenced vertices separates the two at
/// any size, and it costs one linear pass.
pub(super) fn refuse_unweldable_soup(
    mesh: &MeshEditBuffers,
    triangles: usize,
) -> Result<(), MeshEditError> {
    /// Below this a quadratic pass is merely slow, and some fixtures rely on
    /// filling small soups directly.
    const SOUP_REFUSAL_TRIANGLES: usize = 20_000;

    if triangles < SOUP_REFUSAL_TRIANGLES {
        return Ok(());
    }
    // Distinct referenced vertices. A closed triangle surface references about
    // half as many vertices as it has triangles; a tangled patch references
    // about one per triangle; a soup references one per corner, i.e. three per
    // triangle. Comparing against the corner count is what tells them apart.
    let mut seen = vec![false; mesh.vertices.len()];
    let mut distinct = 0usize;
    for &index in &mesh.indices {
        let Some(flag) = seen.get_mut(index as usize) else {
            continue;
        };
        if !*flag {
            *flag = true;
            distinct += 1;
        }
    }
    if distinct.saturating_mul(2) < mesh.indices.len() {
        return Ok(());
    }
    Err(MeshEditError::InvalidOptions {
        reason: format!(
            "hole filling on {triangles} triangles whose corners are not shared is quadratic; \
             weld the soup first (coincident corners that differ in colour or UV cannot be \
             welded by position)"
        ),
    })
}
