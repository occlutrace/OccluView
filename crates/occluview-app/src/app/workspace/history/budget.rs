//! Soft history retention: shared meshes count once and old steps expire after commit.

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;

use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId};

use super::journal::EditCheckpoint;
use super::{HistoryCommandId, HistoryPayload, WorkspaceHistory};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SharedAllocation {
    Mesh(usize),
    Overlay(usize),
}

#[derive(Default)]
pub(crate) struct MemoryContribution {
    exclusive_bytes: usize,
    shared_allocations: Vec<(SharedAllocation, usize)>,
}

#[derive(Clone, Copy)]
struct AllocationUse {
    bytes: usize,
    references: usize,
}

#[derive(Default)]
struct MemoryLedger {
    used_bytes: usize,
    allocations: HashMap<SharedAllocation, AllocationUse>,
    commands: HashMap<HistoryCommandId, MemoryContribution>,
}

impl MemoryLedger {
    fn for_history(history: &WorkspaceHistory, retain_commands: bool) -> Self {
        let mut ledger = Self::default();
        for checkpoint in history.checkpoints.values() {
            ledger.add_checkpoint(checkpoint);
        }
        for entry in history.entries.values() {
            let memory = payload_memory(entry.payload.as_ref(), entry.bytes);
            if retain_commands {
                ledger.add_command(entry.id, memory);
            } else {
                ledger.add_memory(None, memory);
            }
        }
        for entry in history.pending.values() {
            let memory = payload_memory(entry.payload.as_ref(), entry.bytes);
            if retain_commands {
                ledger.add_command(entry.id, memory);
            } else {
                ledger.add_memory(None, memory);
            }
        }
        ledger
    }

    fn add_checkpoint(&mut self, checkpoint: &EditCheckpoint) {
        let mut memory = MemoryContribution {
            exclusive_bytes: checkpoint
                .baseline_bytes
                .saturating_add(
                    checkpoint
                        .undo_at_start
                        .capacity()
                        .saturating_mul(size_of::<HistoryCommandId>()),
                )
                .saturating_add(
                    checkpoint
                        .redo_at_start
                        .capacity()
                        .saturating_mul(size_of::<HistoryCommandId>()),
                ),
            ..MemoryContribution::default()
        };
        scene_allocations(&checkpoint.baseline, &mut memory);
        self.add_memory(None, memory);
    }

    fn add_command(&mut self, id: HistoryCommandId, memory: MemoryContribution) {
        self.add_memory(Some(id), memory);
    }

    fn add_memory(&mut self, id: Option<HistoryCommandId>, memory: MemoryContribution) {
        self.used_bytes = self.used_bytes.saturating_add(memory.exclusive_bytes);
        for (allocation, bytes) in &memory.shared_allocations {
            let usage = self.allocations.entry(*allocation).or_insert(AllocationUse {
                bytes: *bytes,
                references: 0,
            });
            debug_assert_eq!(usage.bytes, *bytes);
            if usage.references == 0 {
                self.used_bytes = self.used_bytes.saturating_add(usage.bytes);
            }
            usage.references = usage.references.saturating_add(1);
        }
        if let Some(id) = id {
            self.commands.insert(id, memory);
        }
    }

    fn remove_command(&mut self, id: HistoryCommandId) -> bool {
        let Some(memory) = self.commands.remove(&id) else {
            return false;
        };
        self.used_bytes = self.used_bytes.saturating_sub(memory.exclusive_bytes);
        for (allocation, _) in memory.shared_allocations {
            let released_bytes = self.allocations.get_mut(&allocation).and_then(|usage| {
                usage.references = usage.references.saturating_sub(1);
                (usage.references == 0).then_some(usage.bytes)
            });
            if let Some(bytes) = released_bytes {
                self.used_bytes = self.used_bytes.saturating_sub(bytes);
                self.allocations.remove(&allocation);
            }
        }
        true
    }
}

fn mesh_allocation(layer: &SceneMesh) -> (SharedAllocation, usize) {
    let mesh_bytes = usize::try_from(layer.mesh.estimated_memory_bytes()).unwrap_or(usize::MAX);
    let arc_overhead = size_of::<Mesh>()
        .saturating_add(size_of::<usize>().saturating_mul(2));
    (
        SharedAllocation::Mesh(Arc::as_ptr(&layer.mesh) as usize),
        mesh_bytes.saturating_add(arc_overhead),
    )
}

fn scene_mesh_allocations(layer: &SceneMesh, memory: &mut MemoryContribution) {
    memory.shared_allocations.push(mesh_allocation(layer));
    if let Some(colors) = layer.overlay_colors() {
        let color_bytes = colors
            .capacity()
            .saturating_mul(size_of::<[u8; 4]>())
            .saturating_add(size_of::<Vec<[u8; 4]>>())
            .saturating_add(size_of::<usize>().saturating_mul(2));
        memory.shared_allocations.push((
            SharedAllocation::Overlay(Arc::as_ptr(colors) as usize),
            color_bytes,
        ));
    }
}

fn scene_allocations(scene: &Scene, memory: &mut MemoryContribution) {
    for layer in scene.meshes() {
        scene_mesh_allocations(layer, memory);
    }
}

impl MemoryContribution {
    pub(crate) fn exclusive(bytes: usize) -> Self {
        Self { exclusive_bytes: bytes, ..Self::default() }
    }

    pub(crate) fn layer(layer: &SceneMesh) -> Self {
        let mut memory = Self::exclusive(size_of::<SceneMesh>());
        scene_mesh_allocations(layer, &mut memory);
        memory
    }

    pub(crate) fn scene(scene: &Scene, guard_capacity: usize) -> Self {
        let mut memory = Self::exclusive(
            size_of::<Scene>()
                .saturating_add(scene.meshes().len().saturating_mul(size_of::<SceneMesh>()))
                .saturating_add(guard_capacity.saturating_mul(size_of::<SceneMeshId>())),
        );
        scene_allocations(scene, &mut memory);
        memory
    }
}

fn payload_memory(payload: &dyn HistoryPayload, declared_bytes: usize) -> MemoryContribution {
    payload.memory_contribution(declared_bytes)
}

impl WorkspaceHistory {
    pub(crate) fn constrain_limits(&mut self, max_count: usize, max_bytes: usize) {
        self.max_count = self.max_count.min(max_count);
        self.max_bytes = self.max_bytes.min(max_bytes);
        self.trim_to_budget();
    }

    pub(super) fn recount_bytes(&mut self) {
        self.used_bytes = MemoryLedger::for_history(self, false).used_bytes;
    }

    /// Plan eviction in one memory-ledger pass. Shared mesh and overlay buffers
    /// are released only when the final retained snapshot stops referencing them.
    /// Pending edits and Cancel baselines remain owned until their transaction resolves.
    pub(super) fn plan_room(
        &self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> Option<Vec<HistoryCommandId>> {
        self.plan_room_with_bytes(add_bytes, add_count, exclude)
            .map(|(plan, _)| plan)
    }

    fn plan_room_with_bytes(
        &self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> Option<(Vec<HistoryCommandId>, usize)> {
        if self.max_count == 0 {
            return None;
        }
        let count = self
            .entries
            .len()
            .saturating_add(self.pending.len())
            .saturating_add(add_count);
        let mut memory = MemoryLedger::for_history(self, true);
        if count <= self.max_count
            && memory.used_bytes.saturating_add(add_bytes) <= self.max_bytes
        {
            return Some((Vec::new(), memory.used_bytes));
        }

        let mut projected = self.eviction_timelines();
        let mut planned = HashSet::new();
        let mut blocked = HashSet::new();
        let mut candidates = self.entry_order.iter().copied();
        loop {
            let remaining_count = count.saturating_sub(planned.len());
            if remaining_count <= self.max_count
                && memory.used_bytes.saturating_add(add_bytes) <= self.max_bytes
            {
                return Some((ordered_plan(planned), memory.used_bytes));
            }
            let Some(candidate) = candidates.find(|id| {
                Some(*id) != exclude
                    && !planned.contains(id)
                    && !blocked.contains(id)
                    && self.entries.contains_key(id)
            }) else {
                // One large current step or live Cancel baseline may exceed
                // the soft target; keeping it must never block an edit.
                return Some((ordered_plan(planned), memory.used_bytes));
            };

            let entry = self.entries.get(&candidate)?;
            let ids = if entry.participants.len() < 2 {
                // Ordinary edit entries do not alter timeline linkage. Avoid
                // cloning every scene timeline once per old snapshot.
                HashSet::from([candidate])
            } else {
                let mut candidate_timelines = projected.clone();
                let ids = Self::collect_linked_prefixes(
                    &self.entries,
                    &mut candidate_timelines,
                    HashSet::from([candidate]),
                );
                if !exclude.is_some_and(|id| ids.contains(&id)) {
                    projected = candidate_timelines;
                }
                ids
            };

            if exclude.is_some_and(|id| ids.contains(&id)) {
                blocked.insert(candidate);
                continue;
            }
            for id in ids {
                if self.entries.contains_key(&id) && planned.insert(id) {
                    memory.remove_command(id);
                }
            }
        }
    }

    pub(super) fn eviction_timelines(
        &self,
    ) -> HashMap<Option<super::SceneKey>, super::journal::SceneTimeline> {
        let mut timelines = self.timelines.clone();
        for checkpoint in self
            .checkpoints
            .values()
            .filter(|checkpoint| checkpoint.redo_displaced)
        {
            let timeline = timelines.entry(checkpoint.scope).or_default();
            let mut redo = checkpoint.redo_at_start.clone();
            redo.extend(
                timeline
                    .redo
                    .iter()
                    .copied()
                    .filter(|id| !checkpoint.redo_at_start.contains(id)),
            );
            timeline.redo = redo;
        }
        timelines
    }

    pub(super) fn trim_to_budget_keep(&mut self, keep: Option<HistoryCommandId>) {
        if let Some((plan, retained_bytes)) = self.plan_room_with_bytes(0, 0, keep) {
            if !plan.is_empty() {
                self.remove_commands_with_linked_prefixes(plan.into_iter().collect());
            } else {
                self.used_bytes = retained_bytes;
            }
        }
    }

    pub(super) fn trim_to_budget(&mut self) {
        self.trim_to_budget_keep(self.entry_order.back().copied());
    }
}

fn ordered_plan(planned: HashSet<HistoryCommandId>) -> Vec<HistoryCommandId> {
    let mut ordered = planned.into_iter().collect::<Vec<_>>();
    ordered.sort_unstable();
    ordered
}
