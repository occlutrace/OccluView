use super::*;

impl TopoJournal {
    /// Encoded size in 32-bit words, counted without allocating wire buffers.
    /// The result matches the two encoders and feeds the stroke and dab budgets.
    pub fn encoded_size_words(&self) -> usize {
        if self.is_empty() {
            return 0;
        }
        let rewired = self.rewired.len() * 7;
        let added_tris = self.added_tris.len() * 3;
        let origins = self.added_origins.len();
        // Per collapse slot: removed, last, three removed corners, the removed
        // origin, three last corners, the last origin.
        let collapsed = 1 + self.collapsed.len() * 10;
        let retired = 1 + self.retired.len();
        // Chronological event list: count plus (kind, index) per event.
        let events = 1 + self.events.len() * 2;
        // Optional material section: count plus one vertex id per edit.
        let material = if self.material.is_empty() {
            0
        } else {
            1 + self.material.len()
        };
        let u32_words =
            11 + rewired + added_tris + origins + collapsed + retired + events + material;
        // 15 floats per appended vertex and 6 per material edit, 4 bytes each.
        let f32_words = self.added_verts.len() * 15 + self.material.len() * 6;
        u32_words + f32_words
    }

    /// Flat wire encoding: header counts, rewired triples, then appended
    /// triangle triples, appended face origins, collapse removals, and retired
    /// groups. The f32 companion carries appended vertex payloads. The header
    /// keeps its row-count word, always zero: adjacency is derived from the
    /// faces, never journaled.
    pub fn encode_u32(&self) -> Vec<u32> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut out = vec![
            self.base_verts as u32,
            self.base_tris as u32,
            self.base_groups,
            self.rewired.len() as u32,
            0,
            self.added_verts.len() as u32,
            self.added_tris.len() as u32,
            self.base_live_tris as u32,
            self.live_tris,
            self.base_revision,
            self.next_revision,
        ];
        for rewire in &self.rewired {
            out.push(rewire.tri);
            out.extend_from_slice(&rewire.before);
            out.extend_from_slice(&rewire.after);
        }
        for corners in &self.added_tris {
            out.extend_from_slice(corners);
        }
        for origin in &self.added_origins {
            out.push(*origin);
        }
        out.push(self.collapsed.len() as u32);
        for slot in &self.collapsed {
            out.push(slot.removed);
            out.push(slot.last);
            out.extend_from_slice(&slot.at_removed_corners);
            out.push(slot.at_removed_origin);
            out.extend_from_slice(&slot.at_last_corners);
            out.push(slot.at_last_origin);
        }
        out.push(self.retired.len() as u32);
        out.extend_from_slice(&self.retired);
        out.push(self.events.len() as u32);
        for event in &self.events {
            let (kind, index) = event.encode();
            out.push(kind);
            out.push(index);
        }
        // Trailing and optional, so a journal without material edits keeps
        // the exact older layout.
        if !self.material.is_empty() {
            out.push(self.material.len() as u32);
            out.extend(self.material.iter().map(|edit| edit.vertex));
        }
        out
    }

    /// 15 floats per appended vertex: pos, normal, reference normal,
    /// reference position, step budget, area, sheet component. Then 6 per
    /// material edit: before, after.
    pub fn encode_f32(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.added_verts.len() * 15 + self.material.len() * 6);
        for added in &self.added_verts {
            out.extend_from_slice(&added.pos);
            out.extend_from_slice(&added.nrm);
            out.extend_from_slice(&added.ref_nrm);
            out.extend_from_slice(&added.reference);
            out.push(added.budget);
            out.push(added.area);
            out.push(added.component as f32);
        }
        for edit in &self.material {
            out.extend_from_slice(&edit.before);
            out.extend_from_slice(&edit.after);
        }
        out
    }

    /// Encode only the records past `mark` as a standalone journal whose base
    /// is `base`. The event list is filtered and re-indexed so the slice obeys
    /// the same "every record is reachable once, in order" contract the full
    /// journal does. Empty when the mark equals the current record count.
    ///
    /// A slice is the display mirror's instruction: faces and vertices.
    pub fn encode_slice_u32(
        &self,
        mark: &TopoSliceMark,
        base: &TopoSliceBase,
        live_tris: u32,
        next_revision: u32,
    ) -> Vec<u32> {
        let rewired = self.rewired.len().saturating_sub(mark.rewired);
        let added_verts = self.added_verts.len().saturating_sub(mark.added_verts);
        let added_tris = self.added_tris.len().saturating_sub(mark.added_tris);
        let collapsed = self.collapsed.len().saturating_sub(mark.collapsed);
        let retired = self.retired.len().saturating_sub(mark.retired);
        // A revision step still publishes, so the mirror's fence follows it.
        if rewired + added_verts + added_tris + collapsed + retired == 0
            && base.revision == next_revision
        {
            return Vec::new();
        }
        let mut out = vec![
            base.verts,
            base.live_tris,
            base.groups,
            rewired as u32,
            0,
            added_verts as u32,
            added_tris as u32,
            base.live_tris,
            live_tris,
            base.revision,
            next_revision,
        ];
        for rewire in &self.rewired[mark.rewired.min(self.rewired.len())..] {
            out.push(rewire.tri);
            out.extend_from_slice(&rewire.before);
            out.extend_from_slice(&rewire.after);
        }
        for corners in &self.added_tris[mark.added_tris.min(self.added_tris.len())..] {
            out.extend_from_slice(corners);
        }
        for origin in &self.added_origins[mark.added_tris.min(self.added_origins.len())..] {
            out.push(*origin);
        }
        out.push(collapsed as u32);
        for slot in &self.collapsed[mark.collapsed.min(self.collapsed.len())..] {
            out.push(slot.removed);
            out.push(slot.last);
            out.extend_from_slice(&slot.at_removed_corners);
            out.push(slot.at_removed_origin);
            out.extend_from_slice(&slot.at_last_corners);
            out.push(slot.at_last_origin);
        }
        out.push(retired as u32);
        out.extend_from_slice(&self.retired[mark.retired.min(self.retired.len())..]);
        // The ordered event list is what makes appends land in the slots a
        // preceding collapse freed; the slice keeps that order and rebases
        // each record index onto the slice's own streams. The mark names the
        // list's own offset, so this walks only the slice's records.
        let tail = &self.events[mark.events.min(self.events.len())..];
        let count_at = out.len();
        out.push(0);
        let mut count = 0u32;
        for event in tail {
            let (kind, index) = event.encode();
            let first = match kind {
                TopoEvent::KIND_ADDED_VERT => mark.added_verts,
                TopoEvent::KIND_ADDED_TRI => mark.added_tris,
                TopoEvent::KIND_REWIRE => mark.rewired,
                TopoEvent::KIND_COLLAPSE => mark.collapsed,
                _ => mark.retired,
            } as u32;
            out.push(kind);
            out.push(index - first);
            count += 1;
        }
        out[count_at] = count;
        out
    }

    /// The f32 half of a slice: 15 floats per appended vertex past `mark`.
    pub fn encode_slice_f32(&self, mark: &TopoSliceMark) -> Vec<f32> {
        let start = mark.added_verts.min(self.added_verts.len());
        let mut out = Vec::with_capacity((self.added_verts.len() - start) * 15);
        for added in &self.added_verts[start..] {
            out.extend_from_slice(&added.pos);
            out.extend_from_slice(&added.nrm);
            out.extend_from_slice(&added.ref_nrm);
            out.extend_from_slice(&added.reference);
            out.push(added.budget);
            out.push(added.area);
            out.push(added.component as f32);
        }
        out
    }

    /// Fail-closed decode: any truncation or count mismatch refuses the whole
    /// journal rather than applying half a topology.
    // fail-closed decoding walks the wire layout in one pass.
    #[allow(clippy::too_many_lines)]
    pub fn decode(words: &[u32], floats: &[f32]) -> Option<TopoJournal> {
        if words.is_empty() && floats.is_empty() {
            return Some(TopoJournal::default());
        }
        if words.len() < 11 {
            return None;
        }
        let (base_verts, base_tris, base_groups) = (words[0] as usize, words[1] as usize, words[2]);
        let (n_rewired, n_rows, n_added_v, n_added_t) = (
            words[3] as usize,
            words[4] as usize,
            words[5] as usize,
            words[6] as usize,
        );
        let (base_live_tris, live_tris) = (words[7] as usize, words[8]);
        let (base_revision, next_revision) = (words[9], words[10]);
        let mut journal = TopoJournal {
            base_verts,
            base_tris,
            base_groups,
            base_live_tris,
            live_tris,
            base_revision,
            next_revision,
            ..TopoJournal::default()
        };
        let mut cursor = 11;
        let take = |cursor: &mut usize, count: usize| -> Option<Vec<u32>> {
            let end = cursor.checked_add(count)?;
            if end > words.len() {
                return None;
            }
            let slice = words[*cursor..end].to_vec();
            *cursor = end;
            Some(slice)
        };
        for _ in 0..n_rewired {
            let head = take(&mut cursor, 7)?;
            journal.rewired.push(TopoRewire {
                tri: head[0],
                before: [head[1], head[2], head[3]],
                after: [head[4], head[5], head[6]],
            });
        }
        // Adjacency is derived from faces, so row payloads are unsupported.
        if n_rows != 0 {
            return None;
        }
        for _ in 0..n_added_t {
            let corners = take(&mut cursor, 3)?;
            journal
                .added_tris
                .push([corners[0], corners[1], corners[2]]);
        }
        for _ in 0..n_added_t {
            journal.added_origins.push(*take(&mut cursor, 1)?.first()?);
        }
        let n_collapsed = take(&mut cursor, 1)?[0] as usize;
        for _ in 0..n_collapsed {
            let head = take(&mut cursor, 10)?;
            if head[0] > head[1] {
                return None;
            }
            journal.collapsed.push(CollapsedSlot {
                removed: head[0],
                last: head[1],
                at_removed_corners: [head[2], head[3], head[4]],
                at_removed_origin: head[5],
                at_last_corners: [head[6], head[7], head[8]],
                at_last_origin: head[9],
            });
        }
        let n_retired = take(&mut cursor, 1)?[0] as usize;
        journal.retired = take(&mut cursor, n_retired)?;
        if cursor < words.len() {
            let n_events = take(&mut cursor, 1)?[0] as usize;
            for _ in 0..n_events {
                let head = take(&mut cursor, 2)?;
                journal.events.push(TopoEvent::decode(head[0], head[1])?);
            }
        } else if n_rewired > 0
            || n_added_v > 0
            || n_added_t > 0
            || n_collapsed > 0
            || n_retired > 0
        {
            return None;
        }
        let material_vertices = if cursor < words.len() {
            let n_material = take(&mut cursor, 1)?[0] as usize;
            if n_material == 0 {
                return None;
            }
            take(&mut cursor, n_material)?
        } else {
            Vec::new()
        };
        if cursor != words.len() || floats.len() != n_added_v * 15 + material_vertices.len() * 6 {
            return None;
        }
        // The event list must reach every record exactly once, in record
        // order. A journal whose records are not all reachable by replay
        // would redo a different mesh than the stroke produced, so it is
        // refused instead of half-applied.
        let mut next = [0u32; 5];
        for event in &journal.events {
            let (kind, index) = event.encode();
            if next[kind as usize] != index {
                return None;
            }
            next[kind as usize] += 1;
        }
        if next[TopoEvent::KIND_ADDED_VERT as usize] as usize != n_added_v
            || next[TopoEvent::KIND_ADDED_TRI as usize] as usize != n_added_t
            || next[TopoEvent::KIND_REWIRE as usize] as usize != n_rewired
            || next[TopoEvent::KIND_COLLAPSE as usize] as usize != n_collapsed
            || next[TopoEvent::KIND_RETIRE as usize] as usize != n_retired
        {
            return None;
        }
        for index in 0..n_added_v {
            let base = index * 15;
            let field = |offset: usize| floats[base + offset];
            journal.added_verts.push(TopoAddedVert {
                vertex: (base_verts + index) as u32,
                group: base_groups + index as u32,
                pos: [field(0), field(1), field(2)],
                nrm: [field(3), field(4), field(5)],
                ref_nrm: [field(6), field(7), field(8)],
                reference: [field(9), field(10), field(11)],
                budget: field(12),
                area: field(13),
                component: {
                    let c = field(14);
                    if !c.is_finite() || c < 0.0 || c >= 16_777_216.0 {
                        return None;
                    }
                    c as u32
                },
            });
        }
        let material_floats = &floats[n_added_v * 15..];
        for (index, &vertex) in material_vertices.iter().enumerate() {
            let value = &material_floats[index * 6..index * 6 + 6];
            if value.iter().any(|component| !component.is_finite()) {
                return None;
            }
            journal.material.push(MaterialEdit {
                vertex,
                before: [value[0], value[1], value[2]],
                after: [value[3], value[4], value[5]],
            });
        }
        Some(journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_collapse_refuses_a_removed_slot_past_the_tail() {
        let mut journal = TopoJournal {
            base_verts: 3,
            base_tris: 2,
            base_groups: 3,
            base_live_tris: 2,
            live_tris: 1,
            ..TopoJournal::default()
        };
        journal.push_collapse(CollapsedSlot {
            removed: 2,
            last: 1,
            at_removed_corners: [0, 1, 2],
            at_removed_origin: 0,
            at_last_corners: [0, 1, 2],
            at_last_origin: 1,
        });
        assert!(TopoJournal::decode(&journal.encode_u32(), &journal.encode_f32()).is_none());
        let mut session = SculptSession::new(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        );
        assert!(session.restore_topo(&[], &[], false, &journal).is_none());
        assert_eq!(session.faces(), &[0, 1, 2]);
        assert_eq!(session.vertex_count(), 3);
        journal.collapsed[0].removed = 1;
        assert!(TopoJournal::decode(&journal.encode_u32(), &journal.encode_f32()).is_some());
    }
}
