//! One bounded command journal shared by every scene in an app workspace.
//!
//! A record lives once in `entries`; scene timelines only contain its command
//! id. A transfer is therefore one command referenced by both timelines, and
//! navigation can validate and advance both sides as one operation.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use occluview_core::{Scene, SceneMeshId};

use super::id::{PaneId, SceneKey};
use super::layout::WorkspaceLayout;

pub(crate) type WorkspaceHistoryHandle = Rc<RefCell<WorkspaceHistory>>;
pub(crate) type HistoryCommandId = u64;
pub(crate) type EditCheckpointId = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryDirection {
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryStepKind {
    LayerEdit { layer_id: SceneMeshId },
    SceneEdit { layer_id: SceneMeshId },
    Transfer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HistoryStepInfo {
    pub(crate) command_id: HistoryCommandId,
    pub(crate) kind: HistoryStepKind,
    pub(crate) participants: Vec<SceneKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryError {
    EmptyTransfer,
    SameSceneTransfer,
    EditSessionActive(SceneKey),
    UnknownCommand(HistoryCommandId),
    BudgetExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NavigationError {
    UnknownCommand(HistoryCommandId),
    CommandNotAtHead(HistoryCommandId),
    SceneClosed(SceneKey),
    EditSessionActive(SceneKey),
}

#[derive(Clone, Debug)]
pub(crate) struct TransferredLayer {
    pub(crate) layer_id: SceneMeshId,
    pub(crate) source_index: usize,
    pub(crate) destination_index: usize,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) source_hidden_stack_position: Option<usize>,
    pub(crate) destination_hidden_stack_position: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct AutoCreatedDestination {
    pub(crate) pane_id: PaneId,
    pub(crate) name: String,
    pub(crate) previous_layout: WorkspaceLayout,
    pub(crate) previous_saved_split: Option<WorkspaceLayout>,
    pub(crate) layout_with_destination: WorkspaceLayout,
    /// Monotonic user activity that means Undo should keep the empty scene.
    pub(crate) preserve_on_undo: bool,
    /// Captured by Undo and consumed by Redo to restore the same scene state.
    pub(crate) retained_after_undo: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TransferRecord {
    pub(crate) source: SceneKey,
    pub(crate) destination: SceneKey,
    pub(crate) layers: Vec<TransferredLayer>,
    pub(crate) auto_created_destination: Option<AutoCreatedDestination>,
    /// Undo restores `None` when a pre-existing pane had not loaded a scene.
    pub(crate) destination_was_uninitialized: bool,
    pub(crate) source_focused_layer_before: Option<SceneMeshId>,
    pub(crate) destination_focused_layer_before: Option<SceneMeshId>,
    pub(crate) active_scene_before: SceneKey,
    pub(crate) active_scene_after: SceneKey,
}

struct HistoryEntry {
    id: HistoryCommandId,
    participants: Vec<SceneKey>,
    owner: EntryOwner,
    kind: HistoryStepKind,
    payload: Box<dyn Any>,
    bytes: usize,
    applied: bool,
}

struct PendingEntry {
    id: HistoryCommandId,
    participants: Vec<SceneKey>,
    owner: EntryOwner,
    kind: HistoryStepKind,
    payload: Box<dyn Any>,
    bytes: usize,
    /// Oldest-command removals reserved during transfer preflight. They are
    /// applied only if the document transaction commits successfully.
    eviction_plan: Vec<HistoryCommandId>,
}

/// Where a command belongs. Standalone controllers use a private local
/// timeline, scene edits belong to one scene, and transfers link two scenes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryOwner {
    Standalone,
    Scene(SceneKey),
    Transfer,
}

struct PendingReservation<T> {
    participants: Vec<SceneKey>,
    owner: EntryOwner,
    kind: HistoryStepKind,
    payload: T,
    bytes: usize,
}

pub(crate) struct HistorySnapshot<T> {
    pub(crate) value: T,
    pub(crate) bytes: usize,
}

#[derive(Clone, Default)]
struct SceneTimeline {
    undo: VecDeque<HistoryCommandId>,
    /// Commands are appended as they are undone; the newest redo is at back.
    redo: VecDeque<HistoryCommandId>,
}

struct EditCheckpoint {
    scope: Option<SceneKey>,
    baseline: Scene,
    baseline_bytes: usize,
    /// First id allocated after the checkpoint. Undo/Redo in an open Edit
    /// session cannot cross this boundary into earlier committed work.
    first_session_command: HistoryCommandId,
    undo_at_start: VecDeque<HistoryCommandId>,
    redo_at_start: VecDeque<HistoryCommandId>,
    /// The pre-session redo branch stays parked until Done, so Cancel can
    /// restore it even after the first real edit commits.
    redo_displaced: bool,
}

/// The workspace owns one command store and per-scene views over it. Detached
/// controllers used by existing single-scene call sites use `None` as a local
/// timeline; they still go through this same journal implementation.
pub(crate) struct WorkspaceHistory {
    timelines: HashMap<Option<SceneKey>, SceneTimeline>,
    entries: HashMap<HistoryCommandId, HistoryEntry>,
    pending: HashMap<HistoryCommandId, PendingEntry>,
    checkpoints: HashMap<EditCheckpointId, EditCheckpoint>,
    entry_order: VecDeque<HistoryCommandId>,
    next_command_id: Option<HistoryCommandId>,
    next_checkpoint_id: Option<EditCheckpointId>,
    used_bytes: usize,
    max_count: usize,
    max_bytes: usize,
}

impl WorkspaceHistory {
    pub(crate) fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            timelines: HashMap::new(),
            entries: HashMap::new(),
            pending: HashMap::new(),
            checkpoints: HashMap::new(),
            entry_order: VecDeque::new(),
            next_command_id: Some(1),
            next_checkpoint_id: Some(1),
            used_bytes: 0,
            max_count,
            max_bytes,
        }
    }

    pub(crate) fn shared(max_count: usize, max_bytes: usize) -> WorkspaceHistoryHandle {
        Rc::new(RefCell::new(Self::new(max_count, max_bytes)))
    }

    pub(crate) fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub(crate) fn constrain_limits(&mut self, max_count: usize, max_bytes: usize) {
        self.max_count = self.max_count.min(max_count);
        self.max_bytes = self.max_bytes.min(max_bytes);
        self.trim_to_budget();
    }

    #[cfg(test)]
    pub(crate) fn undo_len(&self, scope: Option<SceneKey>) -> usize {
        self.timelines
            .get(&scope)
            .map_or(0, |timeline| timeline.undo.len())
    }

    pub(crate) fn has_active_edit_checkpoint(&self, scope: SceneKey) -> bool {
        self.checkpoints
            .values()
            .any(|checkpoint| checkpoint.scope == Some(scope))
    }

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

    pub(crate) fn begin_edit_checkpoint(
        &mut self,
        scope: Option<SceneKey>,
        baseline: Scene,
        baseline_bytes: usize,
    ) -> Option<EditCheckpointId> {
        // The baseline is required to make Cancel truthful. Refuse the Edit
        // session before changing either history or its memory accounting if
        // the shared byte budget cannot retain it.
        if baseline_bytes > self.max_bytes
            || self.used_bytes.saturating_add(baseline_bytes) > self.max_bytes
            || self
                .checkpoints
                .values()
                .any(|checkpoint| checkpoint.scope == scope)
        {
            return None;
        }
        let first_session_command = self.next_command_id?;
        let id = self.allocate_checkpoint_id()?;
        let (undo_at_start, redo_at_start) = {
            let timeline = self.timelines.entry(scope).or_default();
            (timeline.undo.clone(), timeline.redo.clone())
        };
        self.used_bytes = self.used_bytes.saturating_add(baseline_bytes);
        self.checkpoints.insert(
            id,
            EditCheckpoint {
                scope,
                baseline,
                baseline_bytes,
                first_session_command,
                undo_at_start,
                redo_at_start,
                redo_displaced: false,
            },
        );
        Some(id)
    }

    /// Done drops the protected baseline while leaving the journal commands
    /// created during the session available for ordinary Undo.
    pub(crate) fn finish_edit_checkpoint(&mut self, checkpoint_id: EditCheckpointId) -> bool {
        let Some(checkpoint) = self.checkpoints.remove(&checkpoint_id) else {
            return false;
        };
        if checkpoint.redo_displaced {
            self.remove_commands_with_linked_prefixes(
                checkpoint.redo_at_start.iter().copied().collect(),
            );
        }
        self.used_bytes = self.used_bytes.saturating_sub(checkpoint.baseline_bytes);
        self.trim_to_budget();
        true
    }

    /// Cancel restores the exact scene captured on entry and restores the
    /// scene's old undo/redo heads. Other scene timelines are never touched.
    pub(crate) fn cancel_edit_checkpoint(
        &mut self,
        checkpoint_id: EditCheckpointId,
    ) -> Option<Scene> {
        let checkpoint = self.checkpoints.remove(&checkpoint_id)?;
        let scope = checkpoint.scope;
        let mut discard = HashSet::new();
        if let Some(timeline) = self.timelines.get(&scope) {
            discard.extend(
                timeline
                    .undo
                    .iter()
                    .chain(timeline.redo.iter())
                    .copied()
                    .filter(|id| *id >= checkpoint.first_session_command),
            );
        }
        // A transfer cannot be recorded while either side has an open Edit
        // checkpoint. Keep this defensive closure so a stale integration can
        // never leave a half-live cross-scene reference after Cancel.
        self.remove_commands_with_linked_prefixes(discard);

        if let Some(timeline) = self.timelines.get_mut(&scope) {
            timeline.undo = checkpoint.undo_at_start;
            timeline.redo = checkpoint.redo_at_start;
            timeline.undo.retain(|id| self.entries.contains_key(id));
            timeline.redo.retain(|id| self.entries.contains_key(id));
        }
        self.used_bytes = self.used_bytes.saturating_sub(checkpoint.baseline_bytes);
        self.trim_to_budget();
        Some(checkpoint.baseline)
    }

    pub(crate) fn push_pending_edit<T: Any>(
        &mut self,
        scope: Option<SceneKey>,
        kind: HistoryStepKind,
        payload: T,
        bytes: usize,
    ) -> Option<HistoryCommandId> {
        if !matches!(
            kind,
            HistoryStepKind::LayerEdit { .. } | HistoryStepKind::SceneEdit { .. }
        ) {
            return None;
        }
        let owner = scope.map_or(EntryOwner::Standalone, EntryOwner::Scene);
        let id = self.reserve_pending(PendingReservation {
            participants: scope.into_iter().collect(),
            owner,
            kind,
            payload,
            bytes,
        })?;
        Some(id)
    }

    pub(crate) fn update_pending<T: Any>(
        &mut self,
        id: HistoryCommandId,
        update: impl FnOnce(&mut T),
    ) -> bool {
        let Some(entry) = self.pending.get_mut(&id) else {
            return false;
        };
        let Some(payload) = entry.payload.downcast_mut::<T>() else {
            return false;
        };
        update(payload);
        true
    }

    pub(crate) fn commit_pending_edit(&mut self, id: HistoryCommandId) -> bool {
        let Some(pending) = self.pending.remove(&id) else {
            return false;
        };
        let scope = match pending.owner {
            EntryOwner::Standalone => None,
            EntryOwner::Scene(scene) => Some(scene),
            EntryOwner::Transfer => {
                self.pending.insert(id, pending);
                return false;
            }
        };
        self.invalidate_redo_for_edit(scope);
        self.insert_committed(HistoryEntry {
            id: pending.id,
            participants: Vec::new(),
            owner: pending.owner,
            kind: pending.kind,
            payload: pending.payload,
            bytes: pending.bytes,
            applied: true,
        });
        let timeline = self.timelines.entry(scope).or_default();
        timeline.undo.push_back(id);
        self.trim_to_budget();
        self.entries.contains_key(&id)
    }

    pub(crate) fn discard_pending(&mut self, id: HistoryCommandId) -> bool {
        let Some(pending) = self.pending.remove(&id) else {
            return false;
        };
        self.used_bytes = self.used_bytes.saturating_sub(pending.bytes);
        true
    }

    pub(crate) fn top_step(
        &self,
        scope: Option<SceneKey>,
        direction: HistoryDirection,
    ) -> Option<HistoryStepInfo> {
        let timeline = self.timelines.get(&scope)?;
        let id = match direction {
            HistoryDirection::Undo => *timeline.undo.back()?,
            HistoryDirection::Redo => *timeline.redo.back()?,
        };
        if let Some(checkpoint) = self.checkpoints.values().find(|item| item.scope == scope) {
            if id < checkpoint.first_session_command {
                return None;
            }
        }
        let entry = self.entries.get(&id)?;
        let participants = match entry.owner {
            EntryOwner::Standalone => Vec::new(),
            EntryOwner::Scene(scene) => vec![scene],
            EntryOwner::Transfer => entry.participants.clone(),
        };
        Some(HistoryStepInfo {
            command_id: id,
            kind: entry.kind,
            participants,
        })
    }

    pub(crate) fn payload<T: Any>(&self, id: HistoryCommandId) -> Option<&T> {
        self.entries.get(&id)?.payload.downcast_ref::<T>()
    }

    pub(crate) fn navigate_edit<T: Any>(
        &mut self,
        scope: Option<SceneKey>,
        direction: HistoryDirection,
        id: HistoryCommandId,
        current: HistorySnapshot<T>,
    ) -> Option<T> {
        let step = self.top_step(scope, direction)?;
        if step.command_id != id
            || matches!(step.kind, HistoryStepKind::Transfer)
            || self.entries.get(&id)?.payload.downcast_ref::<T>().is_none()
        {
            return None;
        }
        let old_bytes = self.entries.get(&id)?.bytes;
        let current_fits = current.bytes <= self.max_bytes
            && self.make_room(current.bytes.saturating_sub(old_bytes), 0, Some(id));
        let snapshot = {
            let entry = self.entries.get_mut(&id)?;
            std::mem::replace(&mut entry.payload, Box::new(current.value))
        };
        let restored = *snapshot.downcast::<T>().ok()?;
        if current_fits {
            self.used_bytes = self
                .used_bytes
                .saturating_sub(old_bytes)
                .saturating_add(current.bytes);
            let entry = self.entries.get_mut(&id)?;
            entry.bytes = current.bytes;
            entry.applied = direction == HistoryDirection::Redo;
            let timeline = self.timelines.get_mut(&scope)?;
            match direction {
                HistoryDirection::Undo => {
                    if timeline.undo.pop_back() != Some(id) {
                        return None;
                    }
                    timeline.redo.push_back(id);
                }
                HistoryDirection::Redo => {
                    if timeline.redo.pop_back() != Some(id) {
                        return None;
                    }
                    timeline.undo.push_back(id);
                }
            }
        } else {
            self.remove_single_command(id);
        }
        Some(restored)
    }

    pub(crate) fn clear_scope(&mut self, scope: Option<SceneKey>) {
        let ids = self
            .timelines
            .get(&scope)
            .map(|timeline| {
                timeline
                    .undo
                    .iter()
                    .chain(timeline.redo.iter())
                    .copied()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        self.remove_commands_with_linked_prefixes(ids);
        self.timelines.remove(&scope);
        let pending_ids = self
            .pending
            .iter()
            .filter_map(|(id, pending)| {
                (match (scope, pending.owner) {
                    (None, EntryOwner::Standalone) => true,
                    (Some(key), EntryOwner::Scene(scene)) => key == scene,
                    (Some(key), EntryOwner::Transfer) => pending.participants.contains(&key),
                    _ => false,
                })
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in pending_ids {
            self.discard_pending(id);
        }
        self.trim_to_budget();
    }

    /// Retire a scene lifetime. Unlike `clear_scope`, this also releases an
    /// active edit baseline because the caller is removing the scene itself.
    pub(crate) fn close_scope(&mut self, scope: Option<SceneKey>) {
        self.clear_scope(scope);
        let checkpoint_ids = self
            .checkpoints
            .iter()
            .filter_map(|(id, checkpoint)| (checkpoint.scope == scope).then_some(*id))
            .collect::<Vec<_>>();
        for id in checkpoint_ids {
            if let Some(checkpoint) = self.checkpoints.remove(&id) {
                self.used_bytes = self.used_bytes.saturating_sub(checkpoint.baseline_bytes);
            }
        }
        self.trim_to_budget();
    }

    pub(crate) fn prepare_transfer(
        &mut self,
        record: TransferRecord,
        bytes: usize,
    ) -> Result<HistoryCommandId, HistoryError> {
        if record.source == record.destination {
            return Err(HistoryError::SameSceneTransfer);
        }
        if record.layers.is_empty() {
            return Err(HistoryError::EmptyTransfer);
        }
        for scope in [record.source, record.destination] {
            if self.has_active_edit_checkpoint(scope) {
                return Err(HistoryError::EditSessionActive(scope));
            }
        }
        let eviction_plan = self
            .plan_room(bytes, 1, None)
            .ok_or(HistoryError::BudgetExceeded)?;
        let id = self
            .reserve_transfer_pending(
                PendingReservation {
                    participants: vec![record.source, record.destination],
                    owner: EntryOwner::Transfer,
                    kind: HistoryStepKind::Transfer,
                    payload: record,
                    bytes,
                },
                eviction_plan,
            )
            .ok_or(HistoryError::BudgetExceeded)?;
        Ok(id)
    }

    /// Call only after the caller has atomically changed both documents.
    /// `validate_navigation` plus this commit provides a preflight/commit pair
    /// for the UI-thread transaction; this method repeats every head check.
    pub(crate) fn commit_transfer(&mut self, id: HistoryCommandId) -> Result<(), HistoryError> {
        let pending = self
            .pending
            .get(&id)
            .ok_or(HistoryError::UnknownCommand(id))?;
        if pending.kind != HistoryStepKind::Transfer || pending.participants.len() != 2 {
            return Err(HistoryError::UnknownCommand(id));
        }
        let scopes = pending.participants.clone();
        let prepared_evictions = pending.eviction_plan.clone();
        for scope in &scopes {
            if self.has_active_edit_checkpoint(*scope) {
                return Err(HistoryError::EditSessionActive(*scope));
            }
        }
        // Recheck the prepared eviction plan before changing any history. A
        // concurrent history change can alter the oldest safe command, so use
        // a fresh plan only when the original no longer matches live state.
        let current_evictions = self
            .plan_room(0, 0, None)
            .ok_or(HistoryError::BudgetExceeded)?;
        let evictions = if current_evictions == prepared_evictions {
            prepared_evictions
        } else {
            current_evictions
        };
        self.remove_commands_with_linked_prefixes(evictions.into_iter().collect());
        let pending = self
            .pending
            .remove(&id)
            .ok_or(HistoryError::UnknownCommand(id))?;
        for scope in &scopes {
            self.invalidate_redo_branch(Some(*scope));
        }
        self.insert_committed(HistoryEntry {
            id: pending.id,
            participants: scopes.clone(),
            owner: EntryOwner::Transfer,
            kind: HistoryStepKind::Transfer,
            payload: pending.payload,
            bytes: pending.bytes,
            applied: true,
        });
        for scope in scopes {
            self.timelines
                .entry(Some(scope))
                .or_default()
                .undo
                .push_back(id);
        }
        self.trim_to_budget();
        if self.entries.contains_key(&id) {
            Ok(())
        } else {
            Err(HistoryError::BudgetExceeded)
        }
    }

    pub(crate) fn transfer_payload(&self, id: HistoryCommandId) -> Option<&TransferRecord> {
        let entry = self.entries.get(&id)?;
        (entry.kind == HistoryStepKind::Transfer)
            .then(|| entry.payload.downcast_ref::<TransferRecord>())
            .flatten()
    }

    pub(crate) fn update_transfer_record(
        &mut self,
        id: HistoryCommandId,
        update: impl FnOnce(&mut TransferRecord),
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.kind != HistoryStepKind::Transfer {
            return false;
        }
        let Some(record) = entry.payload.downcast_mut::<TransferRecord>() else {
            return false;
        };
        update(record);
        true
    }

    /// Check the narrow auto-created-pane Redo case before the caller mutates
    /// its workspace. The deleted destination's stable `SceneId` is reserved,
    /// but its fresh `SceneEpoch` prevents old async work from targeting it.
    pub(crate) fn validate_transfer_scene_key_replacement(
        &self,
        id: HistoryCommandId,
        old_key: SceneKey,
        new_key: SceneKey,
    ) -> Result<(), NavigationError> {
        if old_key == new_key || old_key.id != new_key.id {
            return Err(NavigationError::CommandNotAtHead(id));
        }
        self.validate_navigation(id, HistoryDirection::Redo)?;
        let entry = self
            .entries
            .get(&id)
            .ok_or(NavigationError::UnknownCommand(id))?;
        let record = entry
            .payload
            .downcast_ref::<TransferRecord>()
            .ok_or(NavigationError::UnknownCommand(id))?;
        let auto = record
            .auto_created_destination
            .as_ref()
            .ok_or(NavigationError::CommandNotAtHead(id))?;
        if record.destination != old_key || auto.preserve_on_undo || auto.retained_after_undo {
            return Err(NavigationError::CommandNotAtHead(id));
        }
        let old_timeline = self
            .timelines
            .get(&Some(old_key))
            .ok_or(NavigationError::SceneClosed(old_key))?;
        if !old_timeline.undo.is_empty()
            || old_timeline.redo.len() != 1
            || old_timeline.redo.back() != Some(&id)
        {
            return Err(NavigationError::CommandNotAtHead(id));
        }
        if self.timelines.contains_key(&Some(new_key)) {
            return Err(NavigationError::CommandNotAtHead(id));
        }
        Ok(())
    }

    /// Re-key an auto-created destination after the application has recreated
    /// its `SceneSession`. No content or journal head changes here.
    pub(crate) fn replace_transfer_scene_key(
        &mut self,
        id: HistoryCommandId,
        old_key: SceneKey,
        new_key: SceneKey,
    ) -> Result<(), NavigationError> {
        self.validate_transfer_scene_key_replacement(id, old_key, new_key)?;
        let timeline = self
            .timelines
            .remove(&Some(old_key))
            .ok_or(NavigationError::SceneClosed(old_key))?;
        self.timelines.insert(Some(new_key), timeline);
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(NavigationError::UnknownCommand(id))?;
        for participant in &mut entry.participants {
            if *participant == old_key {
                *participant = new_key;
            }
        }
        let record = entry
            .payload
            .downcast_mut::<TransferRecord>()
            .ok_or(NavigationError::UnknownCommand(id))?;
        record.destination = new_key;
        if record.active_scene_before == old_key {
            record.active_scene_before = new_key;
        }
        if record.active_scene_after == old_key {
            record.active_scene_after = new_key;
        }
        Ok(())
    }

    pub(crate) fn validate_navigation(
        &self,
        id: HistoryCommandId,
        direction: HistoryDirection,
    ) -> Result<(), NavigationError> {
        let entry = self
            .entries
            .get(&id)
            .ok_or(NavigationError::UnknownCommand(id))?;
        if direction == HistoryDirection::Undo && !entry.applied
            || direction == HistoryDirection::Redo && entry.applied
        {
            return Err(NavigationError::CommandNotAtHead(id));
        }
        for scope in &entry.participants {
            if self.has_active_edit_checkpoint(*scope) {
                return Err(NavigationError::EditSessionActive(*scope));
            }
            let Some(timeline) = self.timelines.get(&Some(*scope)) else {
                return Err(NavigationError::SceneClosed(*scope));
            };
            let actual = match direction {
                HistoryDirection::Undo => timeline.undo.back(),
                HistoryDirection::Redo => timeline.redo.back(),
            };
            if actual != Some(&id) {
                return Err(NavigationError::CommandNotAtHead(id));
            }
        }
        Ok(())
    }

    pub(crate) fn commit_navigation(
        &mut self,
        id: HistoryCommandId,
        direction: HistoryDirection,
    ) -> Result<(), NavigationError> {
        self.validate_navigation(id, direction)?;
        let participants = self
            .entries
            .get(&id)
            .ok_or(NavigationError::UnknownCommand(id))?
            .participants
            .clone();
        for scope in participants {
            let timeline = self
                .timelines
                .get_mut(&Some(scope))
                .ok_or(NavigationError::SceneClosed(scope))?;
            match direction {
                HistoryDirection::Undo => {
                    let popped = timeline.undo.pop_back();
                    debug_assert_eq!(popped, Some(id));
                    timeline.redo.push_back(id);
                }
                HistoryDirection::Redo => {
                    let popped = timeline.redo.pop_back();
                    debug_assert_eq!(popped, Some(id));
                    timeline.undo.push_back(id);
                }
            }
        }
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(NavigationError::UnknownCommand(id))?;
        entry.applied = direction == HistoryDirection::Redo;
        Ok(())
    }

    fn reserve_pending<T: Any>(
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

    fn reserve_transfer_pending<T: Any>(
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

    fn insert_committed(&mut self, entry: HistoryEntry) {
        self.entry_order.push_back(entry.id);
        self.entries.insert(entry.id, entry);
    }

    fn allocate_command_id(&mut self) -> Option<HistoryCommandId> {
        let id = self.next_command_id?;
        self.next_command_id = id.checked_add(1);
        Some(id)
    }

    fn allocate_checkpoint_id(&mut self) -> Option<EditCheckpointId> {
        let id = self.next_checkpoint_id?;
        self.next_checkpoint_id = id.checked_add(1);
        Some(id)
    }

    fn invalidate_redo_branch(&mut self, scope: Option<SceneKey>) {
        let Some(timeline) = self.timelines.get(&scope) else {
            return;
        };
        let ids = timeline.redo.iter().copied().collect::<HashSet<_>>();
        if ids.is_empty() {
            return;
        }
        self.remove_commands_with_linked_prefixes(ids);
    }

    fn invalidate_redo_for_edit(&mut self, scope: Option<SceneKey>) {
        let checkpoint_id = self
            .checkpoints
            .iter()
            .find_map(|(id, checkpoint)| (checkpoint.scope == scope).then_some(*id));
        let Some(checkpoint_id) = checkpoint_id else {
            self.invalidate_redo_branch(scope);
            return;
        };
        let Some(checkpoint) = self.checkpoints.get_mut(&checkpoint_id) else {
            return;
        };
        if !checkpoint.redo_displaced {
            checkpoint.redo_displaced = true;
            if let Some(timeline) = self.timelines.get_mut(&scope) {
                // At the first committed session edit, every visible redo is
                // from the pre-session branch. Park it; Done discards it and
                // Cancel restores it. This preserves no-op/error semantics.
                timeline.redo.clear();
            }
            return;
        }
        // Any redo now visible was created by undoing an edit from this same
        // session, so a new edit invalidates it immediately.
        self.invalidate_redo_branch(scope);
    }

    fn remove_commands_with_linked_prefixes(&mut self, remove: HashSet<HistoryCommandId>) {
        let remove = Self::collect_linked_prefixes(&self.entries, &mut self.timelines, remove);
        for id in remove {
            self.remove_single_command(id);
        }
    }

    fn collect_linked_prefixes(
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

    fn remove_single_command(&mut self, id: HistoryCommandId) {
        if let Some(entry) = self.entries.remove(&id) {
            self.used_bytes = self.used_bytes.saturating_sub(entry.bytes);
            for timeline in self.timelines.values_mut() {
                timeline.undo.retain(|entry_id| *entry_id != id);
                timeline.redo.retain(|entry_id| *entry_id != id);
            }
        }
        self.entry_order.retain(|entry_id| *entry_id != id);
    }

    fn is_protected(&self, id: HistoryCommandId) -> bool {
        self.checkpoints.values().any(|checkpoint| {
            checkpoint.undo_at_start.contains(&id) || checkpoint.redo_at_start.contains(&id)
        })
    }

    fn make_room(
        &mut self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> bool {
        let Some(plan) = self.plan_room(add_bytes, add_count, exclude) else {
            return false;
        };
        self.remove_commands_with_linked_prefixes(plan.into_iter().collect());
        true
    }

    /// Compute which oldest commands must be evicted to make room without
    /// changing history. Transfer admission stores this plan on its pending
    /// entry and only applies it after the document transaction succeeds.
    fn plan_room(
        &self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> Option<Vec<HistoryCommandId>> {
        let mut remaining_count = self
            .entries
            .len()
            .saturating_add(self.pending.len())
            .saturating_add(add_count);
        let mut remaining_bytes = self.used_bytes.saturating_add(add_bytes);
        let mut candidate_order = self.entry_order.clone();
        let mut projected_timelines = self.timelines.clone();
        let mut planned = HashSet::new();
        let mut plan = Vec::new();

        loop {
            let count = remaining_count;
            let bytes = remaining_bytes;
            if count <= self.max_count && bytes <= self.max_bytes {
                return Some(plan);
            }
            let candidate = candidate_order.iter().copied().find(|id| {
                Some(*id) != exclude && !planned.contains(id) && !self.is_protected(*id)
            })?;
            if !self.entries.contains_key(&candidate) {
                candidate_order.retain(|id| *id != candidate);
                continue;
            }

            let ids = Self::collect_linked_prefixes(
                &self.entries,
                &mut projected_timelines,
                HashSet::from([candidate]),
            );
            if ids.iter().any(|id| self.is_protected(*id)) {
                return None;
            }
            candidate_order.retain(|id| !ids.contains(id));
            for id in ids {
                if !planned.insert(id) {
                    continue;
                }
                let Some(entry) = self.entries.get(&id) else {
                    continue;
                };
                remaining_count = remaining_count.saturating_sub(1);
                remaining_bytes = remaining_bytes.saturating_sub(entry.bytes);
                plan.push(id);
            }
        }
    }

    fn trim_to_budget(&mut self) {
        let _ = self.make_room(0, 0, None);
        // Protected checkpoints may temporarily keep the workspace above its
        // soft history cap. New records are refused until Done/Cancel releases
        // the baseline; existing snapshots are not dropped under the operator.
    }
}

impl Default for WorkspaceHistory {
    fn default() -> Self {
        Self::new(16, 512 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutoCreatedDestination, HistoryDirection, HistorySnapshot, HistoryStepKind,
        NavigationError, TransferRecord, TransferredLayer, WorkspaceHistory,
    };
    use crate::app::workspace::id::{PaneId, SceneKey};
    use crate::app::workspace::layout::WorkspaceLayout;
    use occluview_core::{Mesh, Scene, SceneMesh};
    use std::path::PathBuf;

    fn scene_key(id: u64, epoch: u64) -> SceneKey {
        match SceneKey::from_raw_for_test(id, epoch) {
            Some(key) => key,
            None => panic!("test scene identity must be nonzero"),
        }
    }

    fn pane_id(id: u64) -> PaneId {
        match PaneId::from_raw_for_test(id) {
            Some(pane) => pane,
            None => panic!("test pane identity must be nonzero"),
        }
    }

    fn layer_edit(history: &mut WorkspaceHistory, scope: SceneKey, snapshot: &str) -> u64 {
        let layer_id = SceneMesh::new(Mesh::empty()).id();
        let snapshot = snapshot.to_owned();
        let bytes = snapshot.len();
        let Some(id) = history.push_pending_edit(
            Some(scope),
            HistoryStepKind::LayerEdit { layer_id },
            snapshot,
            bytes,
        ) else {
            panic!("test edit must fit in its history budget");
        };
        assert!(history.commit_pending_edit(id));
        id
    }

    fn workspace_layout() -> WorkspaceLayout {
        match WorkspaceLayout::side_by_side(pane_id(1), pane_id(2), 0.5) {
            Ok(layout) => layout,
            Err(error) => panic!("test panes must be distinct: {error:?}"),
        }
    }

    fn transfer_record(
        source: SceneKey,
        destination: SceneKey,
        auto_created: bool,
    ) -> TransferRecord {
        let pane = pane_id(2);
        let layout = workspace_layout();
        TransferRecord {
            source,
            destination,
            layers: vec![TransferredLayer {
                layer_id: SceneMesh::new(Mesh::empty()).id(),
                source_index: 0,
                destination_index: 0,
                source_path: Some(PathBuf::from("scan.stl")),
                source_hidden_stack_position: None,
                destination_hidden_stack_position: None,
            }],
            auto_created_destination: auto_created.then_some(AutoCreatedDestination {
                pane_id: pane,
                name: "Scan 2".to_owned(),
                previous_layout: WorkspaceLayout::single(pane_id(1)),
                previous_saved_split: None,
                layout_with_destination: layout,
                preserve_on_undo: false,
                retained_after_undo: false,
            }),
            destination_was_uninitialized: true,
            source_focused_layer_before: None,
            destination_focused_layer_before: None,
            active_scene_before: source,
            active_scene_after: destination,
        }
    }

    fn commit_transfer(
        history: &mut WorkspaceHistory,
        source: SceneKey,
        destination: SceneKey,
        auto_created: bool,
    ) -> u64 {
        let id =
            match history.prepare_transfer(transfer_record(source, destination, auto_created), 1) {
                Ok(id) => id,
                Err(error) => panic!("test transfer must be accepted: {error:?}"),
            };
        assert_eq!(history.commit_transfer(id), Ok(()));
        id
    }

    #[test]
    fn edit_checkpoint_limits_navigation_and_cancel_restores_prior_redo_only_in_its_scene() {
        let source = scene_key(1, 1);
        let neighbor = scene_key(2, 2);
        let mut history = WorkspaceHistory::new(16, 1024);
        let old_edit = layer_edit(&mut history, source, "before old edit");
        let _neighbor_old = layer_edit(&mut history, neighbor, "neighbor old");

        let Some(old_snapshot) = history.navigate_edit(
            Some(source),
            HistoryDirection::Undo,
            old_edit,
            HistorySnapshot {
                value: "after old edit".to_owned(),
                bytes: "after old edit".len(),
            },
        ) else {
            panic!("the committed edit should undo");
        };
        assert_eq!(old_snapshot, "before old edit");

        let Some(checkpoint) = history.begin_edit_checkpoint(Some(source), Scene::new(), 1) else {
            panic!("checkpoint ID should be available");
        };
        assert!(history
            .top_step(Some(source), HistoryDirection::Undo)
            .is_none());
        assert!(history
            .top_step(Some(source), HistoryDirection::Redo)
            .is_none());

        let session_edit = layer_edit(&mut history, source, "session baseline");
        let neighbor_edit = layer_edit(&mut history, neighbor, "neighbor later");
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(session_edit)
        );

        let baseline = history.cancel_edit_checkpoint(checkpoint);
        assert!(baseline.is_some());
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Redo)
                .map(|step| step.command_id),
            Some(old_edit)
        );
        assert_eq!(
            history
                .top_step(Some(neighbor), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(neighbor_edit)
        );
        assert!(history.payload::<String>(session_edit).is_none());
    }

    #[test]
    fn done_keeps_session_edits_undoable_and_releases_the_checkpoint_boundary() {
        let scene = scene_key(3, 1);
        let mut history = WorkspaceHistory::new(16, 1024);
        let previous = layer_edit(&mut history, scene, "previous");
        let Some(checkpoint) = history.begin_edit_checkpoint(Some(scene), Scene::new(), 1) else {
            panic!("checkpoint ID should be available");
        };
        let during_session = layer_edit(&mut history, scene, "during session");
        assert!(history.finish_edit_checkpoint(checkpoint));

        assert_eq!(
            history
                .top_step(Some(scene), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(during_session)
        );
        let Some(_) = history.navigate_edit(
            Some(scene),
            HistoryDirection::Undo,
            during_session,
            HistorySnapshot {
                value: "after session edit".to_owned(),
                bytes: "after session edit".len(),
            },
        ) else {
            panic!("Done should leave the session edit undoable");
        };
        assert_eq!(
            history
                .top_step(Some(scene), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(previous)
        );
    }

    #[test]
    fn transfer_requires_both_timeline_heads_and_new_work_invalidates_both_redo_branches() {
        let source = scene_key(4, 1);
        let destination = scene_key(5, 1);
        let mut history = WorkspaceHistory::new(16, 1024);
        let _source_old = layer_edit(&mut history, source, "source before");
        let transfer = commit_transfer(&mut history, source, destination, false);
        assert_eq!(
            history
                .transfer_payload(transfer)
                .map(|record| record.destination_was_uninitialized),
            Some(true)
        );

        let destination_edit = layer_edit(&mut history, destination, "destination before");
        assert_eq!(
            history.validate_navigation(transfer, HistoryDirection::Undo),
            Err(NavigationError::CommandNotAtHead(transfer))
        );
        let Some(_) = history.navigate_edit(
            Some(destination),
            HistoryDirection::Undo,
            destination_edit,
            HistorySnapshot {
                value: "destination after".to_owned(),
                bytes: "destination after".len(),
            },
        ) else {
            panic!("the later destination edit should undo first");
        };
        assert_eq!(
            history.validate_navigation(transfer, HistoryDirection::Undo),
            Ok(())
        );
        assert_eq!(
            history.commit_navigation(transfer, HistoryDirection::Undo),
            Ok(())
        );
        for participant in [source, destination] {
            assert_eq!(
                history
                    .top_step(Some(participant), HistoryDirection::Redo)
                    .map(|step| step.command_id),
                Some(transfer)
            );
        }

        let replacement = layer_edit(&mut history, source, "new source edit");
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(replacement)
        );
        assert!(history
            .top_step(Some(source), HistoryDirection::Redo)
            .is_none());
        assert!(history
            .top_step(Some(destination), HistoryDirection::Redo)
            .is_none());
        assert!(history.payload::<TransferRecord>(transfer).is_none());
    }

    #[test]
    fn successive_transfers_evict_oldest_once_and_keep_both_heads_coherent() {
        let source = scene_key(12, 1);
        let destination = scene_key(13, 1);
        let mut history = WorkspaceHistory::new(2, 16);

        let first = commit_transfer(&mut history, source, destination, false);
        let second = commit_transfer(&mut history, destination, source, false);
        let third = commit_transfer(&mut history, source, destination, false);

        assert!(history.transfer_payload(first).is_none());
        assert!(history.transfer_payload(second).is_some());
        assert!(history.transfer_payload(third).is_some());
        assert_eq!(history.entries.len(), 2);
        assert!(history.used_bytes() <= 16);
        for participant in [source, destination] {
            assert_eq!(
                history
                    .top_step(Some(participant), HistoryDirection::Undo)
                    .map(|step| step.command_id),
                Some(third)
            );
        }
        assert_eq!(
            history.validate_navigation(third, HistoryDirection::Undo),
            Ok(())
        );

        assert_eq!(
            history.commit_navigation(third, HistoryDirection::Undo),
            Ok(())
        );
        for participant in [source, destination] {
            assert_eq!(
                history
                    .top_step(Some(participant), HistoryDirection::Redo)
                    .map(|step| step.command_id),
                Some(third)
            );
            assert_eq!(
                history
                    .top_step(Some(participant), HistoryDirection::Undo)
                    .map(|step| step.command_id),
                Some(second)
            );
        }
        assert_eq!(
            history.validate_navigation(second, HistoryDirection::Undo),
            Ok(())
        );
    }

    #[test]
    fn transfer_evicts_oldest_entries_when_only_the_byte_budget_is_full() {
        let source = scene_key(14, 1);
        let destination = scene_key(15, 1);
        let mut history = WorkspaceHistory::new(16, 3);
        let oldest = layer_edit(&mut history, source, "a");
        let retained_source_edit = layer_edit(&mut history, source, "b");
        let retained_destination_edit = layer_edit(&mut history, destination, "c");
        assert_eq!(history.used_bytes(), 3);

        let transfer = commit_transfer(&mut history, source, destination, false);

        assert!(history.payload::<String>(oldest).is_none());
        assert!(history.payload::<String>(retained_source_edit).is_some());
        assert!(history
            .payload::<String>(retained_destination_edit)
            .is_some());
        assert_eq!(history.used_bytes(), 3);
        assert_eq!(history.entries.len(), 3);
        for participant in [source, destination] {
            assert_eq!(
                history
                    .top_step(Some(participant), HistoryDirection::Undo)
                    .map(|step| step.command_id),
                Some(transfer)
            );
        }
        assert_eq!(
            history.validate_navigation(transfer, HistoryDirection::Undo),
            Ok(())
        );
    }

    #[test]
    fn discarding_a_prepared_transfer_does_not_apply_its_eviction_plan() {
        let source = scene_key(16, 1);
        let destination = scene_key(17, 1);
        let mut history = WorkspaceHistory::new(1, 4);
        let existing = layer_edit(&mut history, source, "a");
        let used_bytes = history.used_bytes();
        let transfer =
            match history.prepare_transfer(transfer_record(source, destination, false), 1) {
                Ok(id) => id,
                Err(error) => panic!("the pending transfer should fit after eviction: {error:?}"),
            };

        assert_eq!(history.used_bytes(), used_bytes + 1);
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(existing)
        );
        assert!(history.discard_pending(transfer));

        assert_eq!(history.used_bytes(), used_bytes);
        assert!(history.payload::<String>(existing).is_some());
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(existing)
        );
        assert!(history
            .top_step(Some(destination), HistoryDirection::Undo)
            .is_none());
    }

    #[test]
    fn refused_checkpoint_and_transfer_leave_history_heads_and_accounting_unchanged() {
        let source = scene_key(10, 1);
        let destination = scene_key(11, 1);
        let mut history = WorkspaceHistory::new(1, 4);
        let existing = layer_edit(&mut history, source, "a");
        let used_bytes = history.used_bytes();

        assert!(history
            .begin_edit_checkpoint(Some(source), Scene::new(), 4)
            .is_none());
        assert_eq!(history.used_bytes(), used_bytes);
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(existing)
        );

        assert_eq!(
            history.prepare_transfer(transfer_record(source, destination, false), 5),
            Err(super::HistoryError::BudgetExceeded)
        );
        assert_eq!(history.used_bytes(), used_bytes);
        assert_eq!(
            history
                .top_step(Some(source), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(existing)
        );
        assert!(history
            .top_step(Some(destination), HistoryDirection::Undo)
            .is_none());
    }

    #[test]
    fn auto_created_destination_redo_rekeys_to_a_fresh_scene_epoch() {
        let source = scene_key(6, 1);
        let removed_destination = scene_key(7, 2);
        let recreated_destination = scene_key(7, 3);
        let mut history = WorkspaceHistory::new(16, 1024);
        let transfer = commit_transfer(&mut history, source, removed_destination, true);
        assert_eq!(
            history.commit_navigation(transfer, HistoryDirection::Undo),
            Ok(())
        );
        assert_eq!(
            history.validate_transfer_scene_key_replacement(
                transfer,
                removed_destination,
                recreated_destination
            ),
            Ok(())
        );
        assert_eq!(
            history.replace_transfer_scene_key(
                transfer,
                removed_destination,
                recreated_destination
            ),
            Ok(())
        );
        let Some(record) = history.transfer_payload(transfer) else {
            panic!("transfer payload should still exist");
        };
        assert_eq!(record.destination, recreated_destination);
        assert_eq!(record.active_scene_after, recreated_destination);
        assert_eq!(
            history.validate_navigation(transfer, HistoryDirection::Redo),
            Ok(())
        );
        assert_eq!(
            history.commit_navigation(transfer, HistoryDirection::Redo),
            Ok(())
        );
    }

    #[test]
    fn closing_a_participant_discards_transfer_prefix_but_keeps_later_neighbor_work() {
        let closed_scene = scene_key(8, 1);
        let survivor = scene_key(9, 1);
        let mut history = WorkspaceHistory::new(16, 1024);
        let transfer = commit_transfer(&mut history, closed_scene, survivor, false);
        let later_edit = layer_edit(&mut history, survivor, "independent later edit");

        history.close_scope(Some(closed_scene));
        assert!(matches!(
            history.validate_navigation(transfer, HistoryDirection::Undo),
            Err(NavigationError::UnknownCommand(_))
        ));
        assert_eq!(
            history
                .top_step(Some(survivor), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(later_edit)
        );
    }
}
