//! Edit checkpoints: the baseline an Edit session can cancel back to.

use std::collections::HashSet;

use occluview_core::Scene;

use super::journal::EditCheckpoint;
use super::{EditCheckpointId, SceneKey, WorkspaceHistory};

impl WorkspaceHistory {
    pub(crate) fn has_active_edit_checkpoint(&self, scope: SceneKey) -> bool {
        self.checkpoints
            .values()
            .any(|checkpoint| checkpoint.scope == Some(scope))
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
    pub(super) fn invalidate_redo_for_edit(&mut self, scope: Option<SceneKey>) {
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
}
