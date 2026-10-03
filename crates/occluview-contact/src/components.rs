//! Connected components over 3D points: patches, marks, and contacts.
//!
//! Two questions in this crate are the same question. "Which penetration depth
//! belongs to which mark" and "how many contacts does this bite have" are both
//! "chain the points that are close enough to belong together, and count the
//! chains". One implementation answers both, so a mark can never be one thing
//! for the colour and another for the count.
//!
//! The chain is distance-based rather than mesh-topological. Two vertices join
//! when a chain of hops each no longer than the radius links them, which means
//! a coarse scan — where neighbouring vertices can be a millimetre apart —
//! still forms one patch, while two cusps that merely touch at a single vertex
//! stay separate marks.
//!
//! Points are partitioned by their spatial bounds. Every partition fits inside
//! the chaining radius, and neighbouring partitions are joined only after an
//! exact distance test. Source coordinates stay floating point throughout;
//! translating a scan cannot saturate an integer grid and fuse distant marks.

use std::collections::HashMap;

use glam::DVec3;

/// Chaining radius for per-patch penetration flattening, in millimetres.
///
/// Big enough to bridge the speckle holes inside one spot, small enough not to
/// fuse neighbouring cusps. Not the same number as the contact cluster radius
/// (3.0 mm, `stats`) or the touch gate (0.02 mm): the three answer different
/// questions, and unifying them would change one reading to fix another.
pub(crate) const FLATTEN_RADIUS_MM: f64 = 0.75;

/// Collapse every connected penetration patch to its peak depth.
///
/// A multi-hue ramp needs this: the rim of an interference passes through every
/// intermediate depth and paints a rainbow bullseye around the saturated
/// centre. A single-hue ramp does not — there the flattening only destroys the
/// force distribution inside a mark, which is the reading the operator wants.
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
/// spatial comparisons would be undefined, and fusing unrelated points would
/// move a mark's peak depth onto a different part of the tooth. Leaving the field untouched is the safe failure.
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
/// the radius links them. Each spatial partition fits inside that radius;
/// partition pairs whose bounds are close enough receive exact distance tests.
///
/// Deterministic: labels are handed out in point order, and the union pass
/// partitions with stable point-index tie breaks, so the same points produce the same
/// labels and the same count regardless of how the caller parallelised the work
/// that produced them.
pub(crate) fn connected_components(points: &[DVec3], radius_mm: f64) -> (Vec<usize>, usize) {
    let count = points.len();
    if count == 0 {
        return (Vec::new(), 0);
    }
    if !radius_mm.is_finite() || radius_mm <= 0.0 || points.iter().any(|point| !point.is_finite()) {
        return ((0..count).collect(), count);
    }
    let mut members: Vec<usize> = (0..count).collect();
    let mut cells = Vec::new();
    let mut cell_of_point = vec![0; count];
    let tree = ComponentTree::build(
        points,
        &mut members,
        radius_mm,
        &mut cells,
        &mut cell_of_point,
    );
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    tree.join_neighbors(&tree, points, &cells, radius_mm, &mut parent);

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

/// Distance comparison that cannot overflow or underflow when squared.
fn within_radius(delta: DVec3, radius: f64) -> bool {
    delta.abs().max_element() <= radius && (delta / radius).length_squared() <= 1.0
}

/// Spatial partitions with a diameter no larger than the chaining radius.
/// Bounds remain in source coordinates, so integer saturation cannot alias cells.
struct ComponentTree {
    low: DVec3,
    high: DVec3,
    cell: usize,
    children: Option<Box<(Self, Self)>>,
}

impl ComponentTree {
    fn build(
        points: &[DVec3],
        members: &mut [usize],
        radius: f64,
        cells: &mut Vec<Vec<usize>>,
        cell_of_point: &mut [usize],
    ) -> Self {
        let mut low = points[members[0]];
        let mut high = low;
        for &member in members.iter().skip(1) {
            low = low.min(points[member]);
            high = high.max(points[member]);
        }
        let extent = high - low;
        if members.len() == 1 || within_radius(extent, radius) {
            let cell = cells.len();
            for &member in members.iter() {
                cell_of_point[member] = cell;
            }
            cells.push(members.to_vec());
            return Self {
                low,
                high,
                cell,
                children: None,
            };
        }
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let middle = members.len() / 2;
        members.select_nth_unstable_by(middle, |left, right| {
            points[*left][axis]
                .total_cmp(&points[*right][axis])
                .then(left.cmp(right))
        });
        let (left, right) = members.split_at_mut(middle);
        let children = Box::new((
            Self::build(points, left, radius, cells, cell_of_point),
            Self::build(points, right, radius, cells, cell_of_point),
        ));
        Self {
            low,
            high,
            cell: usize::MAX,
            children: Some(children),
        }
    }

    fn join_neighbors(
        &self,
        other: &Self,
        points: &[DVec3],
        cells: &[Vec<usize>],
        radius: f64,
        parent: &mut [usize],
    ) {
        let gap = (self.low - other.high)
            .max(other.low - self.high)
            .max(DVec3::ZERO);
        if !within_radius(gap, radius) {
            return;
        }
        match (&self.children, &other.children) {
            (Some(children), _) => {
                children
                    .0
                    .join_neighbors(other, points, cells, radius, parent);
                children
                    .1
                    .join_neighbors(other, points, cells, radius, parent);
            }
            (None, Some(children)) => {
                self.join_neighbors(&children.0, points, cells, radius, parent);
                self.join_neighbors(&children.1, points, cells, radius, parent);
            }
            (None, None)
                if self.cell < other.cell
                    && find(parent, self.cell) != find(parent, other.cell) =>
            {
                if cells_touch(points, &cells[self.cell], &cells[other.cell], radius) {
                    union(parent, self.cell, other.cell);
                }
            }
            (None, None) => {}
        }
    }
}

/// Whether any pair across two spatial partitions falls inside the radius.
fn cells_touch(points: &[DVec3], left: &[usize], right: &[usize], radius: f64) -> bool {
    left.iter().any(|first| {
        right
            .iter()
            .any(|second| within_radius(points[*first] - points[*second], radius))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_partition_labels_match_the_distance_graph() {
        let points: Vec<_> = (0..96u32)
            .map(|n| {
                DVec3::new(
                    f64::from(n * 37 % 101) / 20.0,
                    f64::from(n * 17 % 97) / 20.0,
                    f64::from(n * 23 % 89) / 20.0,
                )
            })
            .collect();
        for radius in [0.2, 0.75, 3.0] {
            let mut labels = vec![usize::MAX; points.len()];
            let mut count = 0;
            for seed in 0..points.len() {
                if labels[seed] != usize::MAX {
                    continue;
                }
                let mut pending = vec![seed];
                labels[seed] = count;
                while let Some(point) = pending.pop() {
                    for other in 0..points.len() {
                        if labels[other] == usize::MAX
                            && points[point].distance(points[other]) <= radius
                        {
                            labels[other] = count;
                            pending.push(other);
                        }
                    }
                }
                count += 1;
            }
            assert_eq!(connected_components(&points, radius), (labels, count));
        }
        for scale in [1.0e-200, 1.0e200] {
            let points = [DVec3::ZERO, DVec3::X * scale, DVec3::X * (scale * 10.0)];
            assert_eq!(
                connected_components(&points, scale * 2.0),
                (vec![0, 0, 1], 2)
            );
        }
        assert_eq!(connected_components(&[], 1.0), (vec![], 0));
        assert_eq!(connected_components(&[DVec3::ZERO], 1.0), (vec![0], 1));
    }

    #[test]
    fn component_labels_are_translation_invariant_at_large_coordinates() {
        let points = [
            DVec3::ZERO,
            DVec3::new(0.5, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.5, 0.0, 0.0),
        ];
        let expected = connected_components(&points, 0.75);
        assert_eq!(expected, (vec![0, 0, 1, 1], 2));
        for offset in [
            DVec3::splat(1.0e10),
            DVec3::splat(-1.0e10),
            DVec3::splat(1.0e14),
        ] {
            let translated = points.map(|point| point + offset);
            assert_eq!(
                connected_components(&translated, 0.75),
                expected,
                "{offset}"
            );
        }
    }

    #[test]
    fn extreme_coordinate_patches_are_never_merged_by_grid_saturation() {
        let points = [
            DVec3::splat(f64::from(f32::MAX)),
            DVec3::splat(f64::from(f32::MAX) / 2.0),
            DVec3::splat(-f64::from(f32::MAX)),
        ];
        assert_eq!(connected_components(&points, 3.0), (vec![0, 1, 2], 3));
    }
}
