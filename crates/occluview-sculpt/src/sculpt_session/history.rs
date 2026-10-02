//! Stroke records and exact reversible session state.

use super::*;
use crate::hash::FxHashSet as HashSet;

impl SculptSession {
    /// Open a stroke: arm remeshing and start a fresh undo record.
    pub fn start_stroke(&mut self) {
        self.remesh_armed = true;
        self.topo_touched.clear();
        self.stroke_topo_bytes = 0;
        self.topo_budget_open = true;
        self.dab_axis = None;
        self.stroke_indices.clear();
        self.stroke_positions.clear();
        self.topo_journal.clear_for_stroke(
            self.verts.len() / 3,
            self.tris.len() / 3,
            self.topology.group_count() as u32,
            self.live_tris,
        );
        self.topo_journal.base_revision = self.topo_revision.0;
        self.stroke_retired_base = self.retired_groups;
        diag::reset();
        self.stroke_epoch = self.stroke_epoch.wrapping_add(1);
        if self.stroke_epoch == 0 {
            self.stroke_mark.iter_mut().for_each(|s| *s = 0);
            self.material_mark.iter_mut().for_each(|s| *s = 0);
            self.stroke_normal_mark.fill(u32::MAX);
            self.stroke_epoch = 1;
        }
        self.stroke_path = None;
        self.invalidate_sheet_state(false);
    }

    /// Close the stroke: publish the record the pointer-up path needs.
    ///
    /// There is no heal here. Every dab already ran its own cycle of the
    /// isotropic loop on the live surface, so the mesh the operator sees at
    /// release is the mesh the kernel holds — nothing is left to repair, and
    /// releasing the button cannot change the geometry. All this does is
    /// capture both sides of the stroke in one sorted vertex order for undo.
    pub fn end_stroke(&mut self) -> StrokeRecord {
        self.finish_stroke_record()
    }

    /// Abort the stroke and hand back the record the caller reverts.
    ///
    /// Return the stroke record for the caller to apply or revert.
    pub fn abandon_stroke(&mut self) -> StrokeRecord {
        self.finish_stroke_record()
    }

    fn finish_stroke_record(&mut self) -> StrokeRecord {
        self.dab_axis = None;
        self.stroke_path = None;
        self.invalidate_sheet_state(false);
        let count = self.stroke_indices.len();
        let stroke_indices = std::mem::take(&mut self.stroke_indices);
        let stroke_positions = std::mem::take(&mut self.stroke_positions);
        // A vertex a rolled-back cycle truncated is no longer addressable; its
        // undo slot must be dropped rather than read past the live array.
        let live_verts = self.verts.len() / 3;
        let mut order: Vec<u32> = (0..count as u32)
            .filter(|&slot| (stroke_indices[slot as usize] as usize) < live_verts)
            .collect();
        order.sort_unstable_by_key(|&slot| stroke_indices[slot as usize]);
        let mut indices = Vec::with_capacity(count);
        let mut before = Vec::with_capacity(count * 3);
        let mut after = Vec::with_capacity(count * 3);
        let mut normal_indices = Vec::with_capacity(count);
        let mut normal_values = Vec::with_capacity(count * 3);
        for slot in order {
            let slot = slot as usize;
            let vertex = stroke_indices[slot];
            indices.push(vertex);
            before.extend_from_slice(&stroke_positions[slot * 3..slot * 3 + 3]);
            let offset = vertex as usize * 3;
            after.extend_from_slice(&self.verts[offset..offset + 3]);
            normal_values.extend_from_slice(&self.display_normals[offset..offset + 3]);
        }
        normal_indices.extend_from_slice(&indices);
        // Every topology change already reached the display inside the dab
        // that made it: `after_dab_maintenance` refreshes the rewired corners
        // and returns them in that dab's dirty list, so the close has no extra
        // normal scope to publish. It only carries the normals of the vertices
        // this stroke moved.
        let mut journal = std::mem::take(&mut self.topo_journal);
        journal.next_revision = self.topo_revision.0;
        self.seal_material_edits(&mut journal);
        self.remesh_armed = false;
        self.topo_touched.clear();
        StrokeRecord {
            indices,
            before,
            after,
            normal_indices,
            normal_values,
            journal,
            dab: diag::take(),
        }
    }

    /// Write back positions (undo of a stroke that never split): incremental.
    /// Split-strokes go through [`Self::restore_topo`] with their journal.
    /// Returns `None` for malformed positions or unavailable vertices, without
    /// changing the surface or its spatial indices.
    pub fn restore(&mut self, indices: &[u32], positions: &[f32]) -> Option<Vec<u32>> {
        if !position_history_is_valid(indices, positions, self.vertex_count()) {
            return None;
        }
        Some(self.restore_inner(indices, positions))
    }

    /// Undo (`redo = false`) or redo (`redo = true`) one stroke with its
    /// topology journal. A topology mismatch (stale record) fails closed
    /// with `None` so the caller faults the session instead of diverging
    /// from the display mesh. Rewired corners join the normal scope even
    /// when their positions did not move, so a pure rewire still refreshes
    /// display normals.
    pub fn restore_topo(
        &mut self,
        indices: &[u32],
        positions: &[f32],
        redo: bool,
        journal: &TopoJournal,
    ) -> Option<Vec<u32>> {
        let present = if redo && !journal.is_empty() {
            journal.base_verts.checked_add(journal.added_verts.len())?
        } else {
            self.vertex_count()
        };
        if !position_history_is_valid(indices, positions, present) {
            return None;
        }
        // Revision fencing: undo expects the session at the journal's end
        // state, redo at its start state. Anything else is a stale record
        // and fails closed instead of partially mutating the session.
        // Position-only strokes carry no journal and skip fencing.
        if !journal.is_empty() {
            let expected = if redo {
                journal.base_revision
            } else {
                journal.next_revision
            };
            if self.topo_revision.0 != expected {
                return None;
            }
        }
        self.remesh_armed = false;
        if !journal.is_empty() {
            let applied = if redo {
                self.apply_topo_forward(journal)
            } else {
                self.apply_topo_inverse(journal)
            };
            if !applied {
                return None;
            }
            self.restore_material(journal, redo);
        }
        // Rewired corners enter the scope with their current positions (a
        // no-op write that pulls them into the normal refresh below).
        let live = self.verts.len() / 3;
        let mut extra_indices: Vec<u32> = Vec::new();
        let mut extra_positions: Vec<f32> = Vec::new();
        let mut seen: HashSet<u32> = indices.iter().copied().collect();
        for rewire in &journal.rewired {
            for &corner in rewire.before.iter().chain(rewire.after.iter()) {
                if (corner as usize) < live && seen.insert(corner) {
                    let offset = corner as usize * 3;
                    extra_indices.push(corner);
                    extra_positions.extend_from_slice(&self.verts[offset..offset + 3]);
                }
            }
        }
        let mut all_indices = indices.to_vec();
        let mut all_positions = positions.to_vec();
        all_indices.extend_from_slice(&extra_indices);
        all_positions.extend_from_slice(&extra_positions);
        // Undo may reference truncated ids: only live vertices are written.
        let mut kept_indices = Vec::with_capacity(all_indices.len());
        let mut kept_positions = Vec::with_capacity(all_positions.len());
        for (slot, &vertex) in all_indices.iter().enumerate() {
            if (vertex as usize) < live {
                kept_indices.push(vertex);
                kept_positions.extend_from_slice(&all_positions[slot * 3..slot * 3 + 3]);
            }
        }
        if !journal.is_empty() {
            self.rewind_revision(journal, redo);
        }
        Some(self.restore_inner(&kept_indices, &kept_positions))
    }

    /// Rewind the fenced revision after a history operation: undo returns
    /// to the journal's start revision, redo advances to its end.
    fn rewind_revision(&mut self, journal: &TopoJournal, redo: bool) {
        self.topo_revision = TopologyRevision(if redo {
            journal.next_revision
        } else {
            journal.base_revision
        });
    }

    /// Incremental position write-back shared by undo paths: only the
    /// touched region's ray buckets, brush grid cells, normals, and step
    /// budgets, then return every vertex whose position or display normal may
    /// have changed.
    fn restore_inner(&mut self, indices: &[u32], positions: &[f32]) -> Vec<u32> {
        // Affected groups, deduped.
        let generation = self.next_stamp();
        let mut groups: Vec<u32> = Vec::with_capacity(indices.len());
        for &vertex in indices {
            let group = self.topology.group_of(vertex);
            if self.group_stamp[group as usize] != generation {
                self.group_stamp[group as usize] = generation;
                groups.push(group);
            }
        }
        // Old spans of affected triangles, before any write.
        let tri_generation = self.next_tri_stamp();
        let mut affected: Vec<(u32, ((i32, i32, i32), (i32, i32, i32)))> = Vec::new();
        for &group in &groups {
            for &triangle in self.topology.incident_triangles(group) {
                if self.tri_marks[triangle as usize] != tri_generation {
                    self.tri_marks[triangle as usize] = tri_generation;
                    let offset = triangle as usize * 3;
                    let span = self.rays.span_for_points(
                        self.v(self.tris[offset]),
                        self.v(self.tris[offset + 1]),
                        self.v(self.tris[offset + 2]),
                    );
                    affected.push((triangle, span));
                }
            }
        }
        for (k, &i) in indices.iter().enumerate() {
            let p = DVec3::new(
                positions[k * 3] as f64,
                positions[k * 3 + 1] as f64,
                positions[k * 3 + 2] as f64,
            );
            self.set_v(i, p);
        }

        // Relocate ray buckets only for triangles whose cell span changed.
        let mut changed_triangles: Vec<u32> = Vec::new();
        for &(triangle, old_span) in &affected {
            let offset = triangle as usize * 3;
            let new_span = self.rays.span_for_points(
                self.v(self.tris[offset]),
                self.v(self.tris[offset + 1]),
                self.v(self.tris[offset + 2]),
            );
            if new_span != old_span {
                changed_triangles.push(triangle);
            }
        }
        if !changed_triangles.is_empty() {
            self.rays
                .update_triangles(&self.verts, &self.tris, &changed_triangles);
        }
        // Relocate the brush grid.
        for &group in &groups {
            let now = self.group_v(group);
            self.brush_grid.relocate(group, now);
        }
        // Normals + dirty report over the touched groups and their one-ring.
        self.begin_dirty_batch();
        self.merge_dirty_batch(indices);
        let scope = self.collect_normal_scope(&groups);
        self.refresh_step_budget(&scope);
        self.refresh_scope_normals(&scope);
        for &group in &scope {
            for index in 0..self.topology.members(group).len() {
                let vertex = self.topology.members(group)[index];
                self.merge_dirty_batch(&[vertex]);
            }
        }
        self.normal_scope = scope;
        let dirty = self.finish_dirty_batch();
        self.invalidate_sheet_state(true);
        dirty
    }
}

fn position_history_is_valid(indices: &[u32], positions: &[f32], present: usize) -> bool {
    indices.len().checked_mul(3) == Some(positions.len())
        && positions.iter().all(|value| value.is_finite())
        && indices.iter().all(|&vertex| (vertex as usize) < present)
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn malformed_position_history_is_refused_before_any_write() {
        for (indices, positions) in [
            (vec![0, 1], vec![0.0, 0.0, 1.0]),
            (vec![0, u32::MAX], vec![0.0, 0.0, 1.0, 0.0, 0.0, 2.0]),
            (vec![0], vec![f32::NAN, 0.0, 0.0]),
        ] {
            let mut session = SculptSession::new(
                vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                vec![0, 1, 2],
            )
            .expect("valid mesh fixture");
            let before = session.verts.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.restore_topo(&indices, &positions, false, &TopoJournal::default())
            }));
            assert!(outcome.is_ok(), "malformed position history must not panic");
            assert!(
                outcome.expect("no panic").is_none(),
                "malformed history must be refused"
            );
            assert_eq!(
                session.verts, before,
                "refusal must preserve the whole surface"
            );
            assert!(session.restore(&indices, &positions).is_none());
            assert_eq!(session.verts, before);
        }
    }
}
