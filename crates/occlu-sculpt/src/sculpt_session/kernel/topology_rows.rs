//! Adjacency and group allocation maintained by topology transactions.
//!
//! Adjacency rows are a function of the faces. The remesh keeps them current
//! as it edits, and history derives them again from the faces it restores, so
//! no row is ever journaled: a journal holds only faces, vertices and flags.

use super::*;

impl SculptSession {
    /// Grow every per-group slot by one. The vertex arrays (`verts`,
    /// `normals`, `mask`, `stroke_mark`, `dirty_marks`, reference pair) are
    /// extended by the caller beside this, so a missed vector traps loudly
    /// here rather than as an index panic deep in a dab.
    pub(super) fn grow_group_slot(&mut self) {
        self.sheet_component.push(u32::MAX);
        self.group_retired.push(false);
        self.group_stamp.push(0);
        self.snapshot_stamp.push(0);
        self.selection_stamp.push(0);
        self.pre_pos.push([0.0; 3]);
        self.step_budget.push(0.0);
        self.region_distance.push(f64::INFINITY);
        self.region_normal.push([0.0; 3]);
        self.rollback_marks.push(0);
        self.rollback_factor.push(1.0);
        self.group_area.push(0.0);
        self.denoise_amount.push(0.0);
        debug_assert_eq!(self.group_stamp.len(), self.topology.group_count());
    }

    /// Replace a group's incident-face row. An unchanged row is left alone,
    /// so a no-op edit does not grow the overlay.
    pub(super) fn set_incident_row(topology: &mut SurfaceTopology, group: u32, mut row: Vec<u32>) {
        row.sort_unstable();
        row.dedup();
        if topology.incident_triangles_or_empty(group) == row.as_slice() {
            return;
        }
        topology.set_incident(group, row);
    }

    /// Neighbor rows for `groups`, rebuilt from their incident triangles.
    pub(super) fn refresh_neighbor_rows(topology: &mut SurfaceTopology, groups: &[u32]) {
        for &group in groups {
            let mut row: Vec<u32> = Vec::new();
            for &tri in topology.incident_triangles_or_empty(group) {
                if let Some(triple) = topology.triangle(tri) {
                    for &member in &triple {
                        if member != group && !row.contains(&member) {
                            row.push(member);
                        }
                    }
                }
            }
            row.sort_unstable();
            if topology.neighbors_or_empty(group) == row.as_slice() {
                continue;
            }
            topology.set_neighbors(group, row);
        }
    }

    /// Incident and neighbor rows of `groups`, derived from the live faces.
    ///
    /// History replays faces only. Every group whose rows a stroke changed is
    /// a corner of a face the stroke changed, so the caller names those
    /// corners and this rebuilds exactly their rows from the restored faces.
    /// One pass over the live faces: undo is a single action, and a pass is
    /// simpler to trust than replaying thousands of row patches in order.
    pub(super) fn rebuild_rows(&mut self, groups: &[u32]) {
        let group_count = self.topology.group_count();
        let stamp = self.next_selection_stamp();
        let mut slot: crate::hash::FxHashMap<u32, usize> = crate::hash::FxHashMap::default();
        let mut rows: Vec<Vec<u32>> = Vec::new();
        for &group in groups {
            if (group as usize) < group_count && self.selection_stamp[group as usize] != stamp {
                self.selection_stamp[group as usize] = stamp;
                slot.insert(group, rows.len());
                rows.push(Vec::new());
            }
        }
        if rows.is_empty() {
            return;
        }
        for triangle in 0..self.live_tris {
            let Some(corners) = self.topology.triangle(triangle) else {
                continue;
            };
            for (index, &group) in corners.iter().enumerate() {
                if self.selection_stamp.get(group as usize) != Some(&stamp)
                    || corners[..index].contains(&group)
                {
                    continue;
                }
                if let Some(&at) = slot.get(&group) {
                    rows[at].push(triangle);
                }
            }
        }
        let mut members: Vec<u32> = Vec::with_capacity(rows.len());
        for (&group, &at) in &slot {
            members.push(group);
            self.topology
                .set_incident(group, std::mem::take(&mut rows[at]));
        }
        members.sort_unstable();
        Self::refresh_neighbor_rows(&mut self.topology, &members);
    }
}
