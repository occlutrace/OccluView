use super::*;

/// Which way a journal is replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayDirection {
    /// Undo: records run in reverse and each is taken back.
    Inverse,
    /// Redo: records run in order and each is re-applied.
    Forward,
}

impl SculptSession {
    /// Apply one journal record in one direction, refusing on any mismatch.
    ///
    /// Both directions of a replay run through this single dispatch, so a new
    /// [`TopoEvent`] variant cannot be handled in one direction only: the match
    /// is exhaustive over `(event, direction)` and the compiler rejects a missing
    /// pair. Where the directions differ only in which recorded value they write
    /// back, they share one arm.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per record and direction; splitting it hides the pairing"
    )]
    fn apply_event(
        &mut self,
        journal: &TopoJournal,
        event: TopoEvent,
        direction: ReplayDirection,
    ) -> bool {
        match (event, direction) {
            (TopoEvent::AddedVert(index), ReplayDirection::Inverse) => {
                // Vertex arrays truncate once below; the record only
                // needs to be addressable.
                journal.added_verts.get(index as usize).is_some()
            }
            (TopoEvent::AddedVert(index), ReplayDirection::Forward) => {
                let Some(added) = journal.added_verts.get(index as usize) else {
                    return false;
                };
                if (self.verts.len() / 3) as u32 != added.vertex {
                    return false;
                }
                self.verts.extend_from_slice(&added.pos);
                self.brush_normals.extend_from_slice(&added.nrm);
                self.stroke_mark.push(0);
                self.material_mark.push(0);
                self.dirty_marks.push(0);
                self.reference_verts.extend_from_slice(&added.reference);
                self.reference_normals.extend_from_slice(&added.ref_nrm);
                if self.topology.append_group(added.vertex) != added.group {
                    return false;
                }
                self.grow_group_slot();
                self.sheet_component[added.group as usize] = added.component;
                self.step_budget[added.group as usize] = added.budget;
                self.group_area[added.group as usize] = added.area;
                self.brush_grid
                    .insert(added.group, self.group_v(added.group));
                true
            }
            (TopoEvent::AddedTri(index), ReplayDirection::Inverse) => {
                let Some(corners) = journal.added_tris.get(index as usize) else {
                    return false;
                };
                let live = self.tris.len() / 3;
                if live == 0 {
                    return false;
                }
                let offset = (live - 1) * 3;
                if self.tris.get(offset..offset + 3) != Some(&corners[..]) {
                    return false;
                }
                self.tris.truncate(offset);
                self.topology.truncate_triangles((live - 1) as u32);
                self.face_origin.truncate(live - 1);
                self.tri_marks.truncate(live - 1);
                self.ray_test_marks.truncate(live - 1);
                self.live_tris = (live - 1) as u32;
                true
            }
            (TopoEvent::AddedTri(index), ReplayDirection::Forward) => {
                let Some(corners) = journal.added_tris.get(index as usize) else {
                    return false;
                };
                let id = self.topology.append_triangle([
                    self.topology.group_of(corners[0]),
                    self.topology.group_of(corners[1]),
                    self.topology.group_of(corners[2]),
                ]);
                if id as usize != self.tris.len() / 3 {
                    return false;
                }
                self.tris.extend_from_slice(corners);
                self.face_origin.push(
                    journal
                        .added_origins
                        .get(index as usize)
                        .copied()
                        .unwrap_or(u32::MAX),
                );
                self.live_tris += 1;
                self.tri_marks.push(0);
                self.ray_test_marks.push(0);
                true
            }
            // Both directions write back one of the two corner lists the record
            // kept, so they share the lookup and the bounds check.
            (TopoEvent::Rewire(index), direction) => {
                let Some(rewire) = journal.rewired.get(index as usize) else {
                    return false;
                };
                let offset = rewire.tri as usize * 3;
                if self.tris.get(offset..offset + 3).is_none() {
                    return false;
                }
                let corners = match direction {
                    ReplayDirection::Inverse => rewire.before,
                    ReplayDirection::Forward => rewire.after,
                };
                self.tris[offset..offset + 3].copy_from_slice(&corners);
                self.topology.rewrite_triangle(
                    rewire.tri,
                    [
                        self.topology.group_of(corners[0]),
                        self.topology.group_of(corners[1]),
                        self.topology.group_of(corners[2]),
                    ],
                );
                true
            }
            (TopoEvent::Collapse(index), ReplayDirection::Inverse) => {
                let Some(slot) = journal.collapsed.get(index as usize) else {
                    return false;
                };
                if slot.removed > slot.last {
                    return false;
                }
                // Later events are already undone, so the prefix ends
                // exactly where this collapse truncated it.
                if self.tris.len() / 3 != slot.last as usize {
                    return false;
                }
                let need_faces = slot.last as usize + 1;
                if self.face_origin.len() < need_faces {
                    self.face_origin.resize(need_faces, 0);
                    self.tri_marks.resize(need_faces, 0);
                    self.ray_test_marks.resize(need_faces, 0);
                }
                let need_words = need_faces * 3;
                if self.tris.len() < need_words {
                    self.tris.resize(need_words, 0);
                }
                let last_off = slot.last as usize * 3;
                self.tris[last_off..last_off + 3].copy_from_slice(&slot.at_last_corners);
                self.topology.rewrite_triangle(
                    slot.last,
                    [
                        self.topology.group_of(slot.at_last_corners[0]),
                        self.topology.group_of(slot.at_last_corners[1]),
                        self.topology.group_of(slot.at_last_corners[2]),
                    ],
                );
                let rem_off = slot.removed as usize * 3;
                self.tris[rem_off..rem_off + 3].copy_from_slice(&slot.at_removed_corners);
                self.topology.rewrite_triangle(
                    slot.removed,
                    [
                        self.topology.group_of(slot.at_removed_corners[0]),
                        self.topology.group_of(slot.at_removed_corners[1]),
                        self.topology.group_of(slot.at_removed_corners[2]),
                    ],
                );
                self.face_origin[slot.last as usize] = slot.at_last_origin;
                self.face_origin[slot.removed as usize] = slot.at_removed_origin;
                self.live_tris = slot.last + 1;
                true
            }
            (TopoEvent::Collapse(index), ReplayDirection::Forward) => {
                let Some(slot) = journal.collapsed.get(index as usize) else {
                    return false;
                };
                // The paired swap rewire has already moved the tail face
                // into the removed slot; this step checks the tail still
                // holds what the record says and then truncates the dense
                // prefix. Appends that followed in the stroke land after
                // this point on the same ids they used live.
                // The swap takes the current tail; a journal whose
                // collapse does not sit at the end of the prefix has
                // already diverged, and the display mirror refuses the
                // same shape.
                if self.tris.len() / 3 != slot.last as usize + 1 {
                    return false;
                }
                let last_off = slot.last as usize * 3;
                if self.tris.get(last_off..last_off + 3) != Some(&slot.at_last_corners[..]) {
                    return false;
                }
                let rem_off = slot.removed as usize * 3;
                if self.tris.get(rem_off..rem_off + 3).is_none() {
                    return false;
                }
                self.tris[rem_off..rem_off + 3].copy_from_slice(&slot.at_last_corners);
                self.topology.rewrite_triangle(
                    slot.removed,
                    [
                        self.topology.group_of(slot.at_last_corners[0]),
                        self.topology.group_of(slot.at_last_corners[1]),
                        self.topology.group_of(slot.at_last_corners[2]),
                    ],
                );
                self.face_origin[slot.removed as usize] = slot.at_last_origin;
                self.tris.truncate(last_off);
                self.topology.truncate_triangles(slot.last);
                self.face_origin.truncate(slot.last as usize);
                self.tri_marks.truncate(slot.last as usize);
                self.ray_test_marks.truncate(slot.last as usize);
                self.live_tris = slot.last;
                true
            }
            // Both directions resolve the same two ids before flipping the
            // bitmap, so they share the lookups.
            (TopoEvent::Retire(index), direction) => {
                let Some(&group) = journal.retired.get(index as usize) else {
                    return false;
                };
                let Some(retired) = self.group_retired.get_mut(group as usize) else {
                    return false;
                };
                match direction {
                    ReplayDirection::Inverse => {
                        // Grid entries persist through retirement (consumers
                        // gate on the bitmap), so unretiring needs no grid write
                        // here.
                        if *retired {
                            *retired = false;
                            self.retired_groups = self.retired_groups.saturating_sub(1);
                        }
                    }
                    ReplayDirection::Forward => {
                        if !*retired {
                            *retired = true;
                            self.retired_groups += 1;
                        }
                    }
                }
                true
            }
        }
    }

    /// Every group that is a corner of a face the journal changes, below
    /// `limit`. These are exactly the groups whose adjacency rows the stroke
    /// changed. Read while the journal's appended vertices still map to
    /// groups.
    fn journal_corner_groups(&self, journal: &TopoJournal, limit: u32) -> Vec<u32> {
        let present = self.verts.len() / 3;
        let mut groups: Vec<u32> = Vec::new();
        let mut take = |session: &Self, corners: &[u32; 3]| {
            for &vertex in corners {
                if (vertex as usize) < present {
                    let group = session.topology.group_of(vertex);
                    if group < limit {
                        groups.push(group);
                    }
                }
            }
        };
        for rewire in &journal.rewired {
            take(self, &rewire.before);
            take(self, &rewire.after);
        }
        for slot in &journal.collapsed {
            take(self, &slot.at_removed_corners);
            take(self, &slot.at_last_corners);
        }
        for corners in &journal.added_tris {
            take(self, corners);
        }
        groups.extend(
            journal
                .retired
                .iter()
                .copied()
                .filter(|&group| group < limit),
        );
        groups.sort_unstable();
        groups.dedup();
        groups
    }

    /// Undo one split-stroke: restore rewired triangles from the journal,
    /// truncate every appended tail, then derive the adjacency of every group
    /// the stroke touched from the restored faces. Spatial indices are
    /// repaired before any array shrinks, while dropped ids are still
    /// addressable.
    // one replay pass restores every journal record in order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_topo_inverse(&mut self, journal: &TopoJournal) -> bool {
        // Dense live prefix: the vec length is the live count. Collapse
        // shrinks it below base+appended, so the live word (not the tail
        // arithmetic) is the fail-closed expectation here.
        if self.tris.len() / 3 != journal.live_tris as usize
            || self.topology.triangle_len() as usize != journal.live_tris as usize
            || self.face_origin.len() != journal.live_tris as usize
        {
            return false;
        }
        if self.topology.group_count() as u32
            != journal.base_groups + journal.added_verts.len() as u32
        {
            return false;
        }
        // Validate the RECORDS before replaying any of them, against the
        // current array length rather than the base one. Phase 0 restores each
        // collapse's corners through `topology.group_of`, which indexes the
        // vertex-group map directly; a record naming a vertex that is not in
        // the array at all panics there, before any later check can run. The
        // appended vertices this stroke added are still present at this point,
        // so a corner legitimately at or past `base_verts` is fine here — what
        // is refused is a corner the array cannot address at all.
        {
            let present = self.verts.len() / 3;
            let live_groups = self.topology.group_count();
            let corner_ok = |corners: &[u32; 3]| corners.iter().all(|&c| (c as usize) < present);
            let group_ok = |group: u32| (group as usize) < live_groups;
            if journal
                .added_verts
                .iter()
                .any(|added| !group_ok(added.group))
            {
                return false;
            }
            if journal.added_tris.iter().any(|corners| !corner_ok(corners)) {
                return false;
            }
            for rewire in &journal.rewired {
                if !corner_ok(&rewire.before) || !corner_ok(&rewire.after) {
                    return false;
                }
            }
            for slot in &journal.collapsed {
                if !corner_ok(&slot.at_removed_corners) || !corner_ok(&slot.at_last_corners) {
                    return false;
                }
            }
        }
        // Reverse chronological replay removes appended faces before
        // restoring slots freed by earlier collapses.
        for event in journal.events.iter().rev() {
            if !self.apply_event(journal, *event, ReplayDirection::Inverse) {
                return false;
            }
        }
        self.live_tris = journal.base_live_tris as u32;
        // The reverse replay must have restored every base face to base
        // vertices. Keep the guard: a journal that cannot satisfy it is
        // inconsistent, and the contract for a stale record is a refusal,
        // not a silent half-inverse.
        {
            let live_verts = journal.base_verts;
            for face in self.tris[..self.live_tris as usize * 3].as_chunks::<3>().0 {
                if face.iter().any(|&vertex| vertex as usize >= live_verts) {
                    return false;
                }
            }
        }
        let touched = self.journal_corner_groups(journal, journal.base_groups);
        self.brush_grid.drop_groups(journal.base_groups);
        self.topology
            .truncate_appended(journal.base_groups, journal.base_tris as u32);
        let base_v3 = journal.base_verts * 3;
        self.tris.truncate(journal.base_tris * 3);
        self.verts.truncate(base_v3);
        self.brush_normals.truncate(base_v3);
        self.stroke_mark.truncate(journal.base_verts);
        self.material_mark.truncate(journal.base_verts);
        self.dirty_marks.truncate(journal.base_verts);
        self.reference_verts.truncate(base_v3);
        self.reference_normals.truncate(base_v3);
        let base_groups = journal.base_groups as usize;
        self.group_stamp.truncate(base_groups);
        self.snapshot_stamp.truncate(base_groups);
        self.selection_stamp.truncate(base_groups);
        self.pre_pos.truncate(base_groups);
        self.step_budget.truncate(base_groups);
        self.region_distance.truncate(base_groups);
        self.region_normal.truncate(base_groups);
        self.rollback_marks.truncate(base_groups);
        self.rollback_factor.truncate(base_groups);
        self.group_area.truncate(base_groups);
        self.denoise_amount.truncate(base_groups);
        self.sheet_axis.truncate(base_groups);
        self.sheet_axis_mark.truncate(base_groups);
        self.stroke_normal.truncate(base_groups);
        self.stroke_normal_mark.truncate(base_groups);
        self.reference_wall_mm.truncate(base_groups);
        self.reference_wall_at.truncate(base_groups);
        self.sheet_component.truncate(base_groups);
        self.group_retired.truncate(base_groups);
        self.tri_marks.truncate(journal.base_tris);
        self.ray_test_marks.truncate(journal.base_tris);
        self.face_origin.truncate(journal.base_tris);
        self.live_tris = journal.base_live_tris as u32;
        // Restored tails reuse default stamps: bump every generation so a
        // resurrected zero can never masquerade as a live stamp.
        self.next_tri_stamp();
        self.ray_test_epoch = self.ray_test_epoch.wrapping_add(1);
        if self.ray_test_epoch == 0 {
            self.ray_test_marks.fill(0);
            self.ray_test_epoch = 1;
        }
        self.next_stamp();
        self.rebuild_rows(&touched);
        let mut changed: Vec<u32> = journal.rewired.iter().map(|item| item.tri).collect();
        changed.extend(
            journal
                .collapsed
                .iter()
                .flat_map(|slot| [slot.removed, slot.last]),
        );
        self.rays
            .reconcile_history(&self.verts, &self.tris, self.live_tris, &changed);
        self.hit_triangle = None;
        true
    }

    /// Redo one split-stroke: re-append journaled vertices and triangles,
    /// rewrite rewired corners, then derive the touched adjacency from the
    /// faces. Appends land on the same ids by construction (validated first);
    /// anything else fails closed.
    // one replay pass applies every journal record in order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_topo_forward(&mut self, journal: &TopoJournal) -> bool {
        if self.verts.len() / 3 != journal.base_verts
            || self.tris.len() / 3 != journal.base_tris
            || self.topology.group_count() as u32 != journal.base_groups
            || self.live_tris != journal.base_live_tris as u32
            || self.face_origin.len() != journal.base_tris
        {
            return false;
        }
        // Apply each operation in journal order. A collapse can free face ids
        // that later appends reuse, and every record must have one event.
        for event in &journal.events {
            if !self.apply_event(journal, *event, ReplayDirection::Forward) {
                return false;
            }
        }
        self.live_tris = journal.live_tris;
        let touched = self.journal_corner_groups(journal, u32::MAX);
        self.rebuild_rows(&touched);
        let mut changed: Vec<u32> = journal.rewired.iter().map(|item| item.tri).collect();
        changed.extend(
            journal
                .collapsed
                .iter()
                .flat_map(|slot| [slot.removed, slot.last]),
        );
        self.rays
            .reconcile_history(&self.verts, &self.tris, self.live_tris, &changed);
        self.next_tri_stamp();
        self.ray_test_epoch = self.ray_test_epoch.wrapping_add(1);
        if self.ray_test_epoch == 0 {
            self.ray_test_marks.fill(0);
            self.ray_test_epoch = 1;
        }
        self.next_stamp();
        self.hit_triangle = None;
        true
    }
}
