//! Uniform-grid spatial index over surface-group positions, for the brush
//! radius query a freeform dab needs: built once, cell size
//! matched to the brush radius on a deliberate size change, kept exact during
//! strokes by O(touched) relocation instead of any per-dab rebuild.

use crate::hash::FxHashMap;
use glam::DVec3;

type CellKey = (i32, i32, i32);

/// Grid resolution as a fraction of the mesh's bounding-box diagonal
/// a few-millimeter brush on a dental arch still visits only a handful of cells
/// per query.
const CELLS_ACROSS_DIAGONAL: f64 = 96.0;

/// Largest per-axis cell reach a radius query scans before falling back to a
/// linear pass over every occupied cell.
const MAX_NEIGHBORHOOD_REACH: i32 = 16;

pub(super) struct GroupGrid {
    cell_size: f64,
    origin: DVec3,
    cells: FxHashMap<CellKey, Vec<u32>>,
    /// Slot inside the group's current bucket; swap removal repairs the moved id.
    slots: Vec<usize>,
    keys: Vec<CellKey>,
}

fn cell_key(position: DVec3, origin: DVec3, cell_size: f64) -> CellKey {
    let relative = (position - origin) * (1.0 / cell_size);
    let floor = |value: f64| -> i32 {
        if !value.is_finite() {
            return 0;
        }
        value
            .floor()
            .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i32
    };
    (floor(relative.x), floor(relative.y), floor(relative.z))
}

impl GroupGrid {
    /// Build over one position per surface group, cell size from the mesh's
    /// own scale.
    pub(super) fn build(positions: impl Iterator<Item = DVec3> + Clone) -> GroupGrid {
        let mut lo = DVec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = DVec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in positions.clone() {
            if p.is_finite() {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        if !lo.x.is_finite() {
            lo = DVec3::new(0.0, 0.0, 0.0);
            hi = DVec3::new(0.0, 0.0, 0.0);
        }
        let diagonal = (hi - lo).length();
        let cell_size = if diagonal.is_finite() && diagonal > 1e-12 {
            diagonal / CELLS_ACROSS_DIAGONAL
        } else {
            1.0
        };
        Self::build_with_cell_size(positions, lo, cell_size)
    }

    /// Build with an EXPLICIT cell size so a session can match the grid to
    /// the current brush radius.
    pub(super) fn build_with_cell_size(
        positions: impl Iterator<Item = DVec3>,
        origin: DVec3,
        cell_size: f64,
    ) -> GroupGrid {
        let cell_size = if cell_size.is_finite() && cell_size > 1e-12 {
            cell_size
        } else {
            1.0
        };
        let mut cells: FxHashMap<CellKey, Vec<u32>> = FxHashMap::default();
        let mut slots = Vec::new();
        let mut keys = Vec::new();
        for (index, position) in positions.enumerate() {
            let key = cell_key(position, origin, cell_size);
            keys.push(key);
            let bucket = cells.entry(key).or_default();
            slots.push(bucket.len());
            bucket.push(index as u32);
        }
        GroupGrid {
            cell_size,
            origin,
            cells,
            slots,
            keys,
        }
    }

    pub(super) fn origin(&self) -> DVec3 {
        self.origin
    }

    /// The index owns membership. A caller cannot strand a group by supplying
    /// an old position after several moves in the same transaction.
    pub(super) fn relocate(&mut self, group: u32, to: DVec3) {
        let key = cell_key(to, self.origin, self.cell_size);
        if self.keys[group as usize] == key {
            return;
        }
        self.remove(group);
        let bucket = self.cells.entry(key).or_default();
        self.keys[group as usize] = key;
        self.slots[group as usize] = bucket.len();
        bucket.push(group);
    }

    fn remove(&mut self, group: u32) {
        let key = self.keys[group as usize];
        // The index owns membership, so an entry without a bucket is a broken
        // invariant: report it in a debug build and leave the key alone in a
        // release build rather than creating a second row for the group.
        debug_assert!(self.cells.contains_key(&key), "indexed group has a bucket");
        let Some(bucket) = self.cells.get_mut(&key) else {
            return;
        };
        let slot = self.slots[group as usize];
        debug_assert_eq!(bucket[slot], group);
        bucket.swap_remove(slot);
        if slot < bucket.len() {
            self.slots[bucket[slot] as usize] = slot;
        }
        if bucket.is_empty() {
            self.cells.remove(&key);
        }
    }

    pub(super) fn insert(&mut self, group: u32, position: DVec3) {
        debug_assert_eq!(group as usize, self.slots.len());
        let key = cell_key(position, self.origin, self.cell_size);
        let bucket = self.cells.entry(key).or_default();
        self.keys.push(key);
        self.slots.push(bucket.len());
        bucket.push(group);
    }

    pub(super) fn drop_groups(&mut self, from_group: u32) {
        for group in (from_group..self.slots.len() as u32).rev() {
            self.remove(group);
        }
        self.slots.truncate(from_group as usize);
        self.keys.truncate(from_group as usize);
    }

    /// Every group id within `radius` of `center` by cell coverage — a
    /// conservative superset; callers filter by exact distance. Deterministic
    /// for a fixed update sequence: fixed `(dx, dy, dz)` scan order and
    /// deterministic swap-removal within each cell.
    pub(super) fn query_radius(&self, center: DVec3, radius: f64, found: &mut Vec<u32>) {
        found.clear();
        if !(radius.is_finite() && radius > 0.0) {
            return;
        }
        let reach = ((radius / self.cell_size).ceil() as i32).saturating_add(1);
        if reach > MAX_NEIGHBORHOOD_REACH {
            // The radius dwarfs the grid: one linear pass over every occupied
            // cell beats an O(reach^3) neighborhood scan and can never freeze.
            for bucket in self.cells.values() {
                found.extend_from_slice(bucket);
            }
            found.sort_unstable();
            found.dedup();
            return;
        }
        let center_key = cell_key(center, self.origin, self.cell_size);
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    let key = (center_key.0 + dx, center_key.1 + dy, center_key.2 + dz);
                    if let Some(bucket) = self.cells.get(&key) {
                        found.extend_from_slice(bucket);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_radius_query_returns_every_group_without_overflow() {
        let grid = GroupGrid::build_with_cell_size(
            [DVec3::ZERO, DVec3::X, DVec3::Y].into_iter(),
            DVec3::ZERO,
            1.0,
        );
        let mut found = Vec::new();
        for radius in [f64::from(i32::MAX), f64::MAX] {
            grid.query_radius(DVec3::ZERO, radius, &mut found);
            assert_eq!(found, [0, 1, 2]);
        }
    }
}
