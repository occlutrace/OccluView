//! Edit-session lifecycle: begin/Done/Cancel, dirty/busy state, full reset.

use occluview_core::{Scene, SceneMesh};

use super::state::{EditModeState, LayerKey, SelectGesture};
use super::undo_snapshot::scene_checkpoint_bytes;
use super::{EditModeController, EditSessionStartFailure};

impl EditModeController {
    pub(crate) fn begin_face_selection(&mut self, layer: &SceneMesh, scene: &Scene) -> bool {
        self.session_start_failure = None;
        if matches!(self.state, EditModeState::Busy { .. })
            || !layer.visible
            || layer.mesh.is_point_cloud()
            || layer.mesh.triangle_count() == 0
        {
            return false;
        }
        if self.selections.ensure_for_entry(layer).is_none() {
            return false;
        }
        let starting_session = self.edit_checkpoint.is_none();
        self.active_layer_id = Some(layer.id());
        let _ = self.state.start(LayerKey::from_scene_mesh_id(layer.id()));

        // Capture the pre-edit scene the first time a session opens, so Cancel
        // can revert every edit (including structural additions) in one step.
        if starting_session {
            let checkpoint = self.history.borrow_mut().begin_edit_checkpoint(
                self.history_scope,
                scene.clone(),
                scene_checkpoint_bytes(scene),
            );
            let Some(checkpoint) = checkpoint else {
                self.session_start_failure =
                    Some(EditSessionStartFailure::HistoryCapacityUnavailable);
                self.selections.clear();
                self.active_layer_id = None;
                self.session_layer_id = None;
                self.state.confirm_discard();
                return false;
            };
            self.edit_checkpoint = Some(checkpoint);
            self.session_dirty = false;
            self.gesture = SelectGesture::Lasso;
            self.through_mesh = true;
        }
        self.session_layer_id = Some(layer.id());
        true
    }

    /// Confirm the edit session (Done). Edits are already applied to the live
    /// scene; this closes the session and clears selection/tool state.
    pub(crate) fn finish_edit_session(&mut self) {
        self.selections.clear();
        self.active_layer_id = None;
        if let Some(checkpoint_id) = self.edit_checkpoint.take() {
            self.history
                .borrow_mut()
                .finish_edit_checkpoint(checkpoint_id);
        }
        self.session_dirty = false;
        self.session_layer_id = None;
        self.gesture = SelectGesture::default();
        self.state.confirm_discard();
    }

    /// Revert the whole edit session (Cancel), returning the whole-scene
    /// baseline captured on entry.
    pub(crate) fn cancel_edit_session(&mut self) -> Option<Scene> {
        let checkpoint_id = self.edit_checkpoint.take()?;
        let baseline = self
            .history
            .borrow_mut()
            .cancel_edit_checkpoint(checkpoint_id)?;
        self.selections.clear();
        self.active_layer_id = None;
        self.session_layer_id = None;
        self.session_dirty = false;
        if let Some(command_id) = self.pending_history_command.take() {
            self.history.borrow_mut().discard_pending(command_id);
        }
        self.last_undo_push_stored = false;
        self.gesture = SelectGesture::default();
        self.state.confirm_discard();
        Some(baseline)
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.session_dirty
            || matches!(
                self.state,
                EditModeState::ActiveDirty { .. }
                    | EditModeState::Busy {
                        was_dirty: true,
                        ..
                    }
            )
    }

    pub(crate) fn is_busy(&self) -> bool {
        matches!(self.state, EditModeState::Busy { .. })
    }

    pub(crate) fn clear(&mut self) {
        self.state.confirm_discard();
        if let Some(checkpoint_id) = self.edit_checkpoint.take() {
            self.history
                .borrow_mut()
                .finish_edit_checkpoint(checkpoint_id);
        }
        if let Some(command_id) = self.pending_history_command.take() {
            self.history.borrow_mut().discard_pending(command_id);
        }
        self.history.borrow_mut().clear_scope(self.history_scope);
        self.selections.clear();
        self.active_layer_id = None;
        self.gesture = SelectGesture::default();
        self.session_dirty = false;
        self.session_layer_id = None;
        self.last_undo_push_stored = false;
        self.session_start_failure = None;
    }
}
