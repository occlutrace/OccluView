//! The command store: entry records, identity allocation, and eviction.

use std::any::Any;
use std::collections::HashSet;

use occluview_core::Scene;

use super::*;

pub(super) struct HistoryEntry {
    pub(super) id: HistoryCommandId,
    pub(super) participants: Vec<SceneKey>,
    pub(super) owner: EntryOwner,
    pub(super) kind: HistoryStepKind,
    pub(super) payload: Box<dyn Any>,
    pub(super) bytes: usize,
    pub(super) applied: bool,
}
pub(super) struct PendingEntry {
    pub(super) id: HistoryCommandId,
    pub(super) participants: Vec<SceneKey>,
    pub(super) owner: EntryOwner,
    pub(super) kind: HistoryStepKind,
    pub(super) payload: Box<dyn Any>,
    pub(super) bytes: usize,
    /// Oldest-command removals reserved during transfer preflight. They are
    /// applied only if the document transaction commits successfully.
    pub(super) eviction_plan: Vec<HistoryCommandId>,
}
/// Where a command belongs. Standalone controllers use a private local
/// timeline, scene edits belong to one scene, and transfers link two scenes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EntryOwner {
    Standalone,
    Scene(SceneKey),
    Transfer,
}
pub(super) struct PendingReservation<T> {
    pub(super) participants: Vec<SceneKey>,
    pub(super) owner: EntryOwner,
    pub(super) kind: HistoryStepKind,
    pub(super) payload: T,
    pub(super) bytes: usize,
}
#[derive(Clone, Default)]
pub(super) struct SceneTimeline {
    pub(super) undo: VecDeque<HistoryCommandId>,
    /// Commands are appended as they are undone; the newest redo is at back.
    pub(super) redo: VecDeque<HistoryCommandId>,
}
pub(super) struct EditCheckpoint {
    pub(super) scope: Option<SceneKey>,
    pub(super) baseline: Scene,
    pub(super) baseline_bytes: usize,
    /// First id allocated after the checkpoint. Undo/Redo in an open Edit
    /// session cannot cross this boundary into earlier committed work.
    pub(super) first_session_command: HistoryCommandId,
    pub(super) undo_at_start: VecDeque<HistoryCommandId>,
    pub(super) redo_at_start: VecDeque<HistoryCommandId>,
    /// The pre-session redo branch stays parked until Done, so Cancel can
    /// restore it even after the first real edit commits.
    pub(super) redo_displaced: bool,
}

impl WorkspaceHistory {
    /// Whether this scene still owns history references besides `except`.
    ///
    /// Auto-created destinations must stay alive while another undo/redo
    /// command still refers to their timeline. Checkpoint snapshots and
    /// pending reservations also count because they can restore those
    /// references later.
    pub(crate) fn has_other_commands_for_scene(
        &self,
        scope: SceneKey,
        except: HistoryCommandId,
    ) -> bool {
        self.timelines.get(&Some(scope)).is_some_and(|timeline| {
            timeline
                .undo
                .iter()
                .chain(&timeline.redo)
                .any(|id| *id != except)
        }) || self
            .pending
            .values()
            .any(|entry| entry.id != except && entry.participants.contains(&scope))
            || self.checkpoints.values().any(|checkpoint| {
                checkpoint.scope == Some(scope)
                    && checkpoint
                        .undo_at_start
                        .iter()
                        .chain(&checkpoint.redo_at_start)
                        .any(|id| *id != except)
            })
    }
    pub(super) fn reserve_pending<T: Any>(
        &mut self,
        reservation: PendingReservation<T>,
    ) -> Option<HistoryCommandId> {
        // A pending operation may still fail or be a no-op. Do not evict an
        // existing branch until success is committed; otherwise the failed
        // operation itself would destroy valid Undo/Redo. Refuse this snapshot
        // if its temporary reservation would exceed the shared byte cap.
        if self.max_count == 0
            || reservation.bytes > self.max_bytes
            || self.used_bytes.saturating_add(reservation.bytes) > self.max_bytes
        {
            return None;
        }
        self.insert_pending_reservation(reservation, Vec::new())
    }
    pub(super) fn reserve_transfer_pending<T: Any>(
        &mut self,
        reservation: PendingReservation<T>,
        eviction_plan: Vec<HistoryCommandId>,
    ) -> Option<HistoryCommandId> {
        if self.max_count == 0 || reservation.bytes > self.max_bytes {
            return None;
        }
        self.insert_pending_reservation(reservation, eviction_plan)
    }
    fn insert_pending_reservation<T: Any>(
        &mut self,
        reservation: PendingReservation<T>,
        eviction_plan: Vec<HistoryCommandId>,
    ) -> Option<HistoryCommandId> {
        let id = self.allocate_command_id()?;
        let mut unique_participants = Vec::with_capacity(reservation.participants.len());
        for participant in reservation.participants {
            if !unique_participants.contains(&participant) {
                unique_participants.push(participant);
            }
        }
        self.pending.insert(
            id,
            PendingEntry {
                id,
                participants: unique_participants,
                owner: reservation.owner,
                kind: reservation.kind,
                payload: Box::new(reservation.payload),
                bytes: reservation.bytes,
                eviction_plan,
            },
        );
        self.used_bytes = self.used_bytes.saturating_add(reservation.bytes);
        Some(id)
    }
    pub(super) fn insert_committed(&mut self, entry: HistoryEntry) {
        self.entry_order.push_back(entry.id);
        self.entries.insert(entry.id, entry);
    }
    fn allocate_command_id(&mut self) -> Option<HistoryCommandId> {
        let id = self.next_command_id?;
        self.next_command_id = id.checked_add(1);
        Some(id)
    }
    pub(super) fn allocate_checkpoint_id(&mut self) -> Option<EditCheckpointId> {
        let id = self.next_checkpoint_id?;
        self.next_checkpoint_id = id.checked_add(1);
        Some(id)
    }
    pub(super) fn invalidate_redo_branch(&mut self, scope: Option<SceneKey>) {
        let Some(timeline) = self.timelines.get(&scope) else {
            return;
        };
        let ids = timeline.redo.iter().copied().collect::<HashSet<_>>();
        if ids.is_empty() {
            return;
        }
        self.remove_commands_with_linked_prefixes(ids);
    }
    pub(super) fn remove_commands_with_linked_prefixes(
        &mut self,
        remove: HashSet<HistoryCommandId>,
    ) {
        let remove = Self::collect_linked_prefixes(&self.entries, &mut self.timelines, remove);
        for id in remove {
            self.remove_single_command(id);
        }
    }
    pub(super) fn collect_linked_prefixes(
        entries: &HashMap<HistoryCommandId, HistoryEntry>,
        timelines: &mut HashMap<Option<SceneKey>, SceneTimeline>,
        mut remove: HashSet<HistoryCommandId>,
    ) -> HashSet<HistoryCommandId> {
        let mut pending_ids = remove.iter().copied().collect::<VecDeque<_>>();
        while let Some(id) = pending_ids.pop_front() {
            let Some(entry) = entries.get(&id) else {
                continue;
            };
            let participants = entry.participants.clone();
            if participants.len() < 2 {
                continue;
            }
            let applied = entry.applied;
            for scope in participants {
                let key = Some(scope);
                let Some(timeline) = timelines.get(&key) else {
                    continue;
                };
                let matching_list = if applied {
                    &timeline.undo
                } else {
                    &timeline.redo
                };
                let Some(index) = matching_list.iter().position(|candidate| *candidate == id)
                else {
                    continue;
                };
                let prefix = matching_list
                    .iter()
                    .take(index + 1)
                    .copied()
                    .collect::<Vec<_>>();
                let Some(timeline) = timelines.get_mut(&key) else {
                    continue;
                };
                let list = if applied {
                    &mut timeline.undo
                } else {
                    &mut timeline.redo
                };
                for candidate in prefix {
                    if remove.insert(candidate) {
                        pending_ids.push_back(candidate);
                    }
                    list.retain(|entry_id| *entry_id != candidate);
                }
            }
        }
        remove
    }
    pub(super) fn remove_single_command(&mut self, id: HistoryCommandId) {
        if let Some(entry) = self.entries.remove(&id) {
            self.used_bytes = self.used_bytes.saturating_sub(entry.bytes);
            for timeline in self.timelines.values_mut() {
                timeline.undo.retain(|entry_id| *entry_id != id);
                timeline.redo.retain(|entry_id| *entry_id != id);
            }
        }
        self.entry_order.retain(|entry_id| *entry_id != id);
    }
}
