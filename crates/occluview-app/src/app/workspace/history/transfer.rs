//! The two-scene transfer: prepare, commit, and linked navigation.

use super::journal::{EntryOwner, HistoryEntry, PendingReservation};
use super::{
    HistoryCommandId, HistoryDirection, HistoryError, HistoryStepKind, NavigationError, SceneKey,
    TransferRecord, WorkspaceHistory,
};

impl WorkspaceHistory {
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
}
