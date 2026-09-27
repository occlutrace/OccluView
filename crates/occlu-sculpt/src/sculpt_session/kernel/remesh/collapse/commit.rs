use super::*;

impl SculptSession {
    /// Apply a validated collapse. Cannot fail: every condition was decided by
    /// `plan_collapse_edge` against the geometry the plan was built from.
    // the validated collapse plan commits in one order.
    #[allow(clippy::too_many_lines)]
    pub(super) fn commit_collapse_edge(
        &mut self,
        plan: CollapsePlan,
        journal: &mut TopoJournal,
    ) -> bool {
        let CollapsePlan {
            survivor,
            retired,
            survivor_raw,
            retired_raw,
            survivor_target,
            survivor_material,
            rewires,
            removals,
        } = plan;
        // Commit: survivor and the retired slot take the midpoint bits.
        // Both groups relocate in the brush grid first: later maintenance
        // and undo relocate from these cells and fault on stale ones.
        self.record_stroke_vertex(survivor_raw);
        self.record_stroke_vertex(retired_raw);
        self.set_v(survivor_raw, survivor_target);
        self.set_v(retired_raw, survivor_target);
        if let Some(material) = survivor_material {
            self.set_group_material(journal, survivor, material);
        }
        self.brush_grid.relocate(survivor, survivor_target);
        self.brush_grid.relocate(retired, survivor_target);
        for (face, before, after) in &rewires {
            let offset = *face as usize * 3;
            self.tris[offset..offset + 3].copy_from_slice(after);
            self.topology.rewrite_triangle(
                *face,
                [
                    self.topology.group_of(after[0]),
                    self.topology.group_of(after[1]),
                    self.topology.group_of(after[2]),
                ],
            );
            journal.push_rewire(topology_journal::TopoRewire {
                tri: *face,
                before: *before,
                after: *after,
            });
        }
        // Swap-delete the two dying faces in captured order.
        for removal in &removals {
            let rem_off = removal.slot as usize * 3;
            journal.push_rewire(topology_journal::TopoRewire {
                tri: removal.slot,
                before: removal.at_removed,
                after: removal.at_last,
            });
            journal.push_collapse(topology_journal::CollapsedSlot {
                removed: removal.slot,
                last: removal.last,
                at_removed_corners: removal.at_removed,
                at_removed_origin: removal.at_removed_origin,
                at_last_corners: removal.at_last,
                at_last_origin: removal.at_last_origin,
            });
            if removal.slot != removal.last {
                self.tris[rem_off..rem_off + 3].copy_from_slice(&removal.at_last);
                self.topology
                    .rewrite_triangle(removal.slot, removal.last_triple);
                self.face_origin[removal.slot as usize] = removal.at_last_origin;
            }
            let last_off = removal.last as usize * 3;
            self.tris.truncate(last_off);
            self.topology.truncate_triangles(removal.last);
            self.face_origin.truncate(removal.last as usize);
            self.tri_marks.truncate(removal.last as usize);
            self.ray_test_marks.truncate(removal.last as usize);
            self.live_tris = removal.last;
            journal.live_tris = removal.last;
        }
        // Retire the group: clear its rows and mark it retired. Grid entries
        // stay but every consumer gates on the bitmap plus the emptied rows.
        // The ray grid catches up once, at the end of the step: nothing between
        // here and there casts a ray.
        // Exact incident rows, one journal patch per group (pre-op row
        // straight to final row). Intermediate patches would replay stale
        // slot ids on undo: a slot that dies later in this same op must
        // never appear in a journaled row. A face both rewired and swapped
        // ends up listed exactly once, under its final slot.
        let group_triple = |session: &Self, raw: &[u32; 3]| -> [u32; 3] {
            [
                session.topology.group_of(raw[0]),
                session.topology.group_of(raw[1]),
                session.topology.group_of(raw[2]),
            ]
        };
        let mut affected: Vec<u32> = vec![retired];
        for (face, before, after) in &rewires {
            let _ = face;
            for &group in group_triple(self, before)
                .iter()
                .chain(group_triple(self, after).iter())
            {
                if !affected.contains(&group) {
                    affected.push(group);
                }
            }
        }
        for removal in &removals {
            for &group in group_triple(self, &removal.at_removed)
                .iter()
                .chain(group_triple(self, &removal.at_last).iter())
            {
                if !affected.contains(&group) {
                    affected.push(group);
                }
            }
        }
        // Every corner of a rewired or swap-deleted face needs new neighbor
        // and edge rows, including the retired group whose old rows must clear.
        let scope = affected;
        for &group in &scope {
            let mut row = self.topology.incident_or_empty(group);
            for (face, before, after) in &rewires {
                let before_groups = group_triple(self, before);
                let after_groups = group_triple(self, after);
                if before_groups.contains(&group) && !after_groups.contains(&group) {
                    row.retain(|&t| t != *face);
                } else if after_groups.contains(&group)
                    && !before_groups.contains(&group)
                    && !row.contains(face)
                {
                    row.push(*face);
                }
            }
            for removal in &removals {
                if removal.slot == removal.last {
                    // Tail removal: the face dies outright.
                    row.retain(|&t| t != removal.slot);
                    continue;
                }
                let in_moved = group_triple(self, &removal.at_last).contains(&group);
                // Drop both slots first: the removed slot takes new
                // content below and the tail slot dies here.
                row.retain(|&t| t != removal.slot && t != removal.last);
                if in_moved && !row.contains(&removal.slot) {
                    row.push(removal.slot);
                }
            }
            Self::set_incident_row(&mut self.topology, group, row);
        }
        Self::refresh_neighbor_rows(&mut self.topology, &scope);
        self.group_retired[retired as usize] = true;
        self.retired_groups += 1;
        journal.push_retired(retired);
        self.topo_touched.extend_from_slice(&scope);
        true
    }
}
