//! Connected components over 3D points: patches, marks, and contacts.
//!
//! Two questions in this crate are the same question. "Which penetration depth
//! belongs to which mark" and "how many contacts does this bite have" are both
//! "chain the points that are close enough to belong together, and count the
//! chains". One implementation answers both, so a mark can never be one thing
//! for the colour and another for the count.
//!
//! The chain is distance-based rather than mesh-topological on purpose. Two
//! vertices join when a chain of hops each no longer than the radius links
//! them, which means a coarse scan — where neighbouring vertices can be a
//! millimetre apart — still forms one patch, while two cusps that merely touch
//! at a single vertex stay separate marks.
//!
//! # Why the integer grid
//!
//! A uniform integer grid with a cell side of `radius / √3` guarantees that
//! every point inside one cell is within the radius of every other point in it,
//! so only neighbouring cells ever need a pairwise test. Cell keys are exact
//! integers rather than hashed floats because a hash collision would silently
//! fuse two unrelated patches into one mark, and a fused mark reports a peak
//! depth that belongs to a different part of the tooth.

use std::collections::HashMap;

use glam::DVec3;

/// Chaining radius for per-patch penetration flattening, in millimetres.
///
/// Big enough to bridge the speckle holes inside one spot, small enough not to
/// fuse neighbouring cusps. Deliberately not the same number as the contact
/// cluster radius (3.0 mm) or the hover region radius (1.6 mm): the three
/// answer different questions, and unifying them would change one reading to
/// fix another.
pub(crate) const FLATTEN_RADIUS_MM: f64 = 0.75;

/// Collapse every connected penetration patch to its peak depth.
///
/// A multi-hue ramp needs this: the rim of an interference passes through every
/// intermediate depth and paints a rainbow bullseye around the saturated
/// centre. A single-hue ramp does not — there the flattening only destroys the
/// force distribution INSIDE a mark, which is the reading the operator wants.
/// So the caller that owns the colour law decides, and this function does what
/// it is told rather than guessing.
pub(crate) fn flatten_penetration_patches(positions: &[f32], signed_mm: &mut [f32]) {
    let mut members: Vec<usize> = Vec::new();
    for (index, value) in signed_mm.iter().enumerate() {
        if value.is_finite() && *value < -super::PENETRATION_EPS_MM {
            members.push(index);
        }
    }
    if members.len() < 2 {
        return;
    }

    let Some(points) = points_of(positions, &members) else {
        return;
    };
    let (labels, patch_count) = connected_components(&points, FLATTEN_RADIUS_MM);

    let mut peaks = vec![f32::INFINITY; patch_count];
    for (slot, vertex) in members.iter().enumerate() {
        let Some(label) = labels.get(slot) else {
            continue;
        };
        let Some(value) = signed_mm.get(*vertex) else {
            continue;
        };
        if let Some(peak) = peaks.get_mut(*label) {
            if *value < *peak {
                *peak = *value;
            }
        }
    }
    for (slot, vertex) in members.iter().enumerate() {
        let Some(label) = labels.get(slot) else {
            continue;
        };
        let Some(peak) = peaks.get(*label) else {
            continue;
        };
        if let Some(value) = signed_mm.get_mut(*vertex) {
            *value = *peak;
        }
    }
}

/// The vertex positions named by `members`, or `None` when one lies outside the
/// position buffer or is not finite.
///
/// A non-finite position aborts the whole pass rather than being skipped: the
/// grid would put it in cell zero, next to whatever real patch happens to be
/// there, and fusing the two would move a mark's peak depth onto a different
/// part of the tooth. Leaving the field untouched is the honest failure.
fn points_of(positions: &[f32], members: &[usize]) -> Option<Vec<DVec3>> {
    let mut points = Vec::with_capacity(members.len());
    for vertex in members {
        let offset = vertex.checked_mul(3)?;
        let point = DVec3::new(
            f64::from(*positions.get(offset)?),
            f64::from(*positions.get(offset + 1)?),
            f64::from(*positions.get(offset + 2)?),
        );
        if !point.is_finite() {
            return None;
        }
        points.push(point);
    }
    Some(points)
}

/// Connected components of `points` under `radius_mm`, one label per point.
///
/// Two points belong to the same component when a chain of hops no longer than
/// the radius links them. The cell side is `radius / √3`, so every point in one
/// cell is linked and only neighbouring cell pairs need explicit tests.
///
/// Deterministic: labels are handed out in point order, and the union pass
/// walks cells in insertion order, so the same points always produce the same
/// labels and the same count regardless of how the caller parallelised the work
/// that produced them.
pub(crate) fn connected_components(points: &[DVec3], radius_mm: f64) -> (Vec<usize>, usize) {
    let count = points.len();
    if count == 0 {
        return (Vec::new(), 0);
    }
    let side = radius_mm / 3.0_f64.sqrt();
    // A radius that is not a positive, finite number has no cells to chain
    // through: every point stands alone rather than being merged into one mark
    // by a degenerate grid.
    if !side.is_finite() || side <= 0.0 {
        return ((0..count).collect(), count);
    }
    let inverse = 1.0 / side;
    let radius_sq = radius_mm * radius_mm;

    // Exact integer cell keys, not a hash of the coordinate: a hash collision
    // would fuse two unrelated patches into one mark.
    let mut cell_index: HashMap<(i32, i32, i32), usize> = HashMap::new();
    let mut cell_coords: Vec<(i32, i32, i32)> = Vec::new();
    let mut cell_members: Vec<Vec<usize>> = Vec::new();
    let mut cell_of_point: Vec<usize> = Vec::with_capacity(count);
    for point in points {
        let key = cell_key(*point, inverse);
        let cell = *cell_index.entry(key).or_insert_with(|| {
            cell_coords.push(key);
            cell_members.push(Vec::new());
            cell_coords.len() - 1
        });
        if let Some(bucket) = cell_members.get_mut(cell) {
            bucket.push(cell_of_point.len());
        }
        cell_of_point.push(cell);
    }

    let mut parent: Vec<usize> = (0..cell_coords.len()).collect();
    // Two points within the radius sit at most ceil(radius/side) = 2 cells
    // apart on each axis.
    for cell in 0..cell_coords.len() {
        let Some((cell_x, cell_y, cell_z)) = cell_coords.get(cell).copied() else {
            continue;
        };
        for dx in -2..=2 {
            for dy in -2..=2 {
                for dz in -2..=2 {
                    if (dx, dy, dz) <= (0, 0, 0) {
                        continue; // each unordered pair once
                    }
                    let Some(&other) = cell_index.get(&(cell_x + dx, cell_y + dy, cell_z + dz))
                    else {
                        continue;
                    };
                    if find(&mut parent, cell) == find(&mut parent, other) {
                        continue;
                    }
                    let (Some(left), Some(right)) =
                        (cell_members.get(cell), cell_members.get(other))
                    else {
                        continue;
                    };
                    if cells_touch(points, left, right, radius_sq) {
                        union(&mut parent, cell, other);
                    }
                }
            }
        }
    }

    let mut label_of_root: HashMap<usize, usize> = HashMap::new();
    let mut labels = vec![0_usize; count];
    for (point, cell) in cell_of_point.iter().enumerate() {
        let root = find(&mut parent, *cell);
        let next = label_of_root.len();
        let label = *label_of_root.entry(root).or_insert(next);
        if let Some(slot) = labels.get_mut(point) {
            *slot = label;
        }
    }
    let total = label_of_root.len();
    (labels, total)
}

/// The integer cell a point falls in.
///
/// The cast saturates rather than wrapping, which is what keeps a scan placed
/// at a wild coordinate from aliasing onto another cell.
#[allow(clippy::cast_possible_truncation)]
fn cell_key(point: DVec3, inverse_cell: f64) -> (i32, i32, i32) {
    (
        (point.x * inverse_cell).floor() as i32,
        (point.y * inverse_cell).floor() as i32,
        (point.z * inverse_cell).floor() as i32,
    )
}

/// Whether any pair across two cells falls inside the radius.
fn cells_touch(points: &[DVec3], left: &[usize], right: &[usize], radius_sq: f64) -> bool {
    for first in left {
        let Some(point) = points.get(*first) else {
            continue;
        };
        for second in right {
            let Some(other) = points.get(*second) else {
                continue;
            };
            if (*point - *other).length_squared() <= radius_sq {
                return true;
            }
        }
    }
    false
}

/// The root of `node`, halving the path on the way up.
fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent.get(node).copied().unwrap_or(node) != node {
        let Some(grandparent) = parent
            .get(node)
            .and_then(|parent_of| parent.get(*parent_of))
            .copied()
        else {
            return node;
        };
        let Some(slot) = parent.get_mut(node) else {
            return node;
        };
        *slot = grandparent;
        node = grandparent;
    }
    node
}

/// Join two cells' components, keeping the lower root.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let root_a = find(parent, a);
    let root_b = find(parent, b);
    if root_a != root_b {
        let (low, high) = if root_a < root_b {
            (root_a, root_b)
        } else {
            (root_b, root_a)
        };
        if let Some(slot) = parent.get_mut(high) {
            *slot = low;
        }
    }
}
