//! Scene edits: pending commands, navigation, and scope teardown.

use std::any::Any;
use std::collections::HashSet;

use super::journal::{EntryOwner, HistoryEntry, PendingReservation};
use super::{
    HistoryCommandId, HistoryDirection, HistoryPayload, HistorySnapshot, HistoryStepInfo, HistoryStepKind,
    SceneKey, WorkspaceHistory,
};

impl WorkspaceHistory {
    pub(crate) fn push_pending_edit<T: HistoryPayload>(
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
        self.recount_bytes();
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
        self.trim_to_budget_keep(Some(id));
        self.entries.contains_key(&id)
    }
    pub(crate) fn discard_pending(&mut self, id: HistoryCommandId) -> bool {
        let Some(_pending) = self.pending.remove(&id) else {
            return false;
        };
        self.recount_bytes();
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
    pub(crate) fn navigate_edit<T: HistoryPayload>(
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
        let snapshot = {
            let entry = self.entries.get_mut(&id)?;
            entry.bytes = current.bytes;
            entry.applied = direction == HistoryDirection::Redo;
            std::mem::replace(&mut entry.payload, Box::new(current.value))
        };
        let restored = *snapshot.downcast::<T>().ok()?;
        let timeline = self.timelines.get_mut(&scope)?;
        match direction {
            HistoryDirection::Undo => {
                timeline.undo.pop_back();
                timeline.redo.push_back(id);
            }
            HistoryDirection::Redo => {
                timeline.redo.pop_back();
                timeline.undo.push_back(id);
            }
        }
        self.trim_to_budget_keep(Some(id));
        Some(restored)
    }
    pub(crate) fn clear_scope(&mut self, scope: Option<SceneKey>) {
        let ids = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                let belongs = match (scope, entry.owner) {
                    (None, EntryOwner::Standalone) => true,
                    (Some(key), EntryOwner::Scene(scene)) => key == scene,
                    (Some(key), EntryOwner::Transfer) => entry.participants.contains(&key),
                    _ => false,
                };
                belongs.then_some(*id)
            })
            .collect::<HashSet<_>>();
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
        let checkpoint_ids = self
            .checkpoints
            .iter()
            .filter_map(|(id, checkpoint)| (checkpoint.scope == scope).then_some(*id))
            .collect::<Vec<_>>();
        for id in checkpoint_ids {
            if let Some(checkpoint) = self.checkpoints.remove(&id) {
                drop(checkpoint);
            }
        }
        self.clear_scope(scope);
    }
}
