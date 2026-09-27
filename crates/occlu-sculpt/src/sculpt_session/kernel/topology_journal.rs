//! Ordered reversible topology edits and per-call publication.

use super::*;

mod codec;
mod replay;

/// One rewired triangle, journaled both ways (vertex ids: the index buffer
/// is addressed by vertex, not group).
#[derive(Clone, Debug)]
pub(crate) struct TopoRewire {
    pub(crate) tri: u32,
    pub(crate) before: [u32; 3],
    pub(crate) after: [u32; 3],
}

/// One appended vertex with everything a redo needs to reproduce it.
#[derive(Clone, Debug)]
pub(crate) struct TopoAddedVert {
    pub(crate) vertex: u32,
    pub(crate) group: u32,
    pub(crate) component: u32,
    pub(crate) pos: [f32; 3],
    pub(crate) nrm: [f32; 3],
    pub(crate) ref_nrm: [f32; 3],
    /// The vertex's position on the opening surface, which a fill records
    /// separately because it is not the vertex's live pose. Reconstructing it
    /// from `pos` would make every guard that measures against the baseline
    /// measure against the deformed mesh after undo/redo.
    pub(crate) reference: [f32; 3],
    pub(crate) budget: f32,
    pub(crate) area: f32,
}

/// One swap-deleted face removal, journaled both ways. Collapse never
/// renumbers: the last live face moves into the removed slot and the
/// dense live prefix shrinks by one. Undo extends the prefix and writes
/// both slots back; the raw corners plus origins restore bit-exactly and
/// group triples re-derive from the stable vertex-group mapping.
#[derive(Clone, Debug)]
pub(crate) struct CollapsedSlot {
    pub(crate) removed: u32,
    pub(crate) last: u32,
    pub(crate) at_removed_corners: [u32; 3],
    pub(crate) at_removed_origin: u32,
    pub(crate) at_last_corners: [u32; 3],
    pub(crate) at_last_origin: u32,
}

/// One vertex whose material coordinate the stroke moved: its value before the
/// stroke first moved it and when the stroke closed. It is session state, not
/// display topology, so per-dab slices never carry it.
#[derive(Clone, Debug)]
pub(crate) struct MaterialEdit {
    pub(crate) vertex: u32,
    pub(crate) before: [f32; 3],
    pub(crate) after: [f32; 3],
}

/// Topology changes from one stroke. Empty when the stroke changes positions
/// without changing connectivity.
#[derive(Clone, Debug, Default)]
pub struct TopoJournal {
    pub(crate) base_verts: usize,
    pub(crate) base_tris: usize,
    pub(crate) base_groups: u32,
    pub(crate) base_live_tris: usize,
    pub(crate) live_tris: u32,
    /// Session topology revision when this stroke started (undo expects
    /// the session at `next_revision`; redo expects it at `base`).
    pub(crate) base_revision: u32,
    pub(crate) next_revision: u32,
    pub(crate) rewired: Vec<TopoRewire>,
    pub(crate) added_verts: Vec<TopoAddedVert>,
    pub(crate) added_tris: Vec<[u32; 3]>,
    pub(crate) added_origins: Vec<u32>,
    pub(crate) collapsed: Vec<CollapsedSlot>,
    pub(crate) retired: Vec<u32>,
    /// Chronological order of the records above. A collapse can free face
    /// slots that a later append reuses, so replay applies both operations in
    /// this order to preserve face ids.
    pub(crate) events: Vec<TopoEvent>,
    /// Material coordinates the stroke's remesh moved. Unordered: a material
    /// point never decides topology, so history writes the whole set after the
    /// topology replay.
    pub(crate) material: Vec<MaterialEdit>,
}

/// Where a record sits in the stroke's chronological order. `index` names
/// the record inside its own vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopoEvent {
    AddedVert(u32),
    AddedTri(u32),
    Rewire(u32),
    Collapse(u32),
    Retire(u32),
}

impl TopoEvent {
    const KIND_ADDED_VERT: u32 = 0;
    const KIND_ADDED_TRI: u32 = 1;
    const KIND_REWIRE: u32 = 2;
    const KIND_COLLAPSE: u32 = 3;
    const KIND_RETIRE: u32 = 4;

    fn encode(self) -> (u32, u32) {
        match self {
            TopoEvent::AddedVert(index) => (Self::KIND_ADDED_VERT, index),
            TopoEvent::AddedTri(index) => (Self::KIND_ADDED_TRI, index),
            TopoEvent::Rewire(index) => (Self::KIND_REWIRE, index),
            TopoEvent::Collapse(index) => (Self::KIND_COLLAPSE, index),
            TopoEvent::Retire(index) => (Self::KIND_RETIRE, index),
        }
    }

    fn decode(kind: u32, index: u32) -> Option<TopoEvent> {
        match kind {
            Self::KIND_ADDED_VERT => Some(TopoEvent::AddedVert(index)),
            Self::KIND_ADDED_TRI => Some(TopoEvent::AddedTri(index)),
            Self::KIND_REWIRE => Some(TopoEvent::Rewire(index)),
            Self::KIND_COLLAPSE => Some(TopoEvent::Collapse(index)),
            Self::KIND_RETIRE => Some(TopoEvent::Retire(index)),
            _ => None,
        }
    }
}

impl TopoJournal {
    pub(crate) fn push_added_vert(&mut self, added: TopoAddedVert) {
        let index = self.added_verts.len() as u32;
        self.added_verts.push(added);
        self.events.push(TopoEvent::AddedVert(index));
    }

    pub(crate) fn push_added_tri(&mut self, corners: [u32; 3]) {
        let index = self.added_tris.len() as u32;
        self.added_tris.push(corners);
        self.events.push(TopoEvent::AddedTri(index));
    }

    pub(crate) fn push_rewire(&mut self, rewire: TopoRewire) {
        let index = self.rewired.len() as u32;
        self.rewired.push(rewire);
        self.events.push(TopoEvent::Rewire(index));
    }

    pub(crate) fn push_collapse(&mut self, slot: CollapsedSlot) {
        let index = self.collapsed.len() as u32;
        self.collapsed.push(slot);
        self.events.push(TopoEvent::Collapse(index));
    }

    pub(crate) fn push_retired(&mut self, group: u32) {
        let index = self.retired.len() as u32;
        self.retired.push(group);
        self.events.push(TopoEvent::Retire(index));
    }

    pub(crate) fn push_material(&mut self, edit: MaterialEdit) {
        self.material.push(edit);
    }
}

impl TopoJournal {
    pub(crate) fn is_empty(&self) -> bool {
        self.rewired.is_empty()
            && self.added_verts.is_empty()
            && self.added_tris.is_empty()
            && self.collapsed.is_empty()
            && self.retired.is_empty()
            && self.events.is_empty()
            && self.material.is_empty()
    }

    pub(crate) fn clear_for_stroke(&mut self, verts: usize, tris: usize, groups: u32, live: u32) {
        self.base_verts = verts;
        self.base_tris = tris;
        self.base_groups = groups;
        self.base_live_tris = live as usize;
        self.live_tris = live;
        self.base_revision = 0;
        self.next_revision = 0;
        self.rewired.clear();
        self.added_verts.clear();
        self.added_tris.clear();
        self.added_origins.clear();
        self.collapsed.clear();
        self.retired.clear();
        self.events.clear();
        self.material.clear();
    }
}

/// How many records of each kind the mirror has already received for the
/// stroke. Every topology the session publishes is the journal slice past this
/// mark, so the mirror replays the same ordered operation stream the kernel
/// ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopoSliceMark {
    pub rewired: usize,
    pub added_verts: usize,
    pub added_tris: usize,
    pub collapsed: usize,
    pub retired: usize,
    /// Event offset for an append-only journal. Slice encoding reads the
    /// records at and after this offset.
    pub events: usize,
}

/// The mirror's exact state when a slice starts. Replay depends on these
/// counts and the ordered records, so the base and slice form one contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopoSliceBase {
    pub verts: u32,
    pub live_tris: u32,
    pub groups: u32,
    pub revision: u32,
}

/// One self-contained, ordered journal slice: a valid standalone journal
/// that takes a mirror from `base` to the session's current state.
#[derive(Clone, Debug, Default)]
pub struct TopoSlice {
    /// Encoded integer records of the slice.
    pub words: Vec<u32>,
    /// Encoded float payloads of the slice.
    pub floats: Vec<f32>,
}

impl TopoSlice {
    /// Whether the slice carries no records.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.floats.is_empty()
    }
}

impl SculptSession {
    /// The slice mark and base for the session's current state.
    pub(crate) fn topo_slice_point(&self) -> (TopoSliceMark, TopoSliceBase) {
        let journal = &self.topo_journal;
        (
            TopoSliceMark {
                rewired: journal.rewired.len(),
                added_verts: journal.added_verts.len(),
                added_tris: journal.added_tris.len(),
                collapsed: journal.collapsed.len(),
                retired: journal.retired.len(),
                events: journal.events.len(),
            },
            TopoSliceBase {
                verts: (self.verts.len() / 3) as u32,
                live_tris: self.live_tris,
                groups: self.topology.group_count() as u32,
                revision: self.topo_revision.0,
            },
        )
    }

    /// Publish every topology record recorded since `mark` as its own
    /// ordered journal. The full stroke journal is untouched; this is the
    /// slice the display mirror has not seen.
    pub(crate) fn publish_topology_slice(
        &self,
        mark: &TopoSliceMark,
        base: &TopoSliceBase,
    ) -> TopoSlice {
        TopoSlice {
            words: self.topo_journal.encode_slice_u32(
                mark,
                base,
                self.live_tris,
                self.topo_revision.0,
            ),
            floats: self.topo_journal.encode_slice_f32(mark),
        }
    }
}
