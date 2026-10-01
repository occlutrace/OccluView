//! Reconcile canonical per-layer selections, state-machine targets, and undo
//! history against the live scene.

use occluview_core::Scene;

use super::state::{EditModeState, LayerKey};
use super::EditModeController;
use crate::app::workspace::history::{HistoryDirection, HistoryStepKind};

impl EditModeController {
    pub(crate) fn sync_to_scene(&mut self, scene: &Scene) {
        self.selections.sync_to_scene(scene);
        self.sync_active_layer(scene);
        self.sync_state_to_scene(scene);
        self.sync_undo_to_scene(scene);
    }

    /// Reconcile UI state after a workspace transfer. The transfer is itself
    /// being prepared or committed in the shared journal, so the departing
    /// layer must not make this controller clear the preceding history head.
    pub(crate) fn sync_after_transfer(&mut self, scene: &Scene) {
        self.selections.sync_to_scene(scene);
        self.sync_active_layer(scene);
        self.sync_state_to_scene(scene);
    }

    fn sync_active_layer(&mut self, scene: &Scene) {
        let Some(layer_id) = self.active_layer_id else {
            return;
        };
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            self.active_layer_id = None;
            return;
        };
        if !entry.visible || entry.mesh.is_point_cloud() || entry.mesh.triangle_count() == 0 {
            self.active_layer_id = None;
            return;
        }
        if self.selections.selection_for_layer(layer_id).is_none() {
            if self.session_layer_id == Some(layer_id) {
                let _ = self.selections.ensure_for_entry(entry);
            } else {
                self.active_layer_id = None;
            }
        }
    }

    fn sync_state_to_scene(&mut self, scene: &Scene) {
        let Some(active_layer) = (match self.state {
            EditModeState::Inactive => None,
            EditModeState::ActiveClean { layer }
            | EditModeState::ActiveDirty { layer }
            | EditModeState::Busy { layer, .. } => Some(layer),
            EditModeState::Error { layer, .. } => layer,
        }) else {
            return;
        };

        let layer_exists = scene
            .meshes()
            .iter()
            .any(|entry| LayerKey::from_scene_mesh_id(entry.id()) == active_layer);
        if !layer_exists {
            self.state.confirm_discard();
        }
    }

    fn sync_undo_to_scene(&mut self, scene: &Scene) {
        let Some(step) = self
            .history
            .borrow()
            .top_step(self.history_scope, HistoryDirection::Undo)
        else {
            return;
        };
        let undo_layer_id = match step.kind {
            HistoryStepKind::LayerEdit { layer_id } | HistoryStepKind::SceneEdit { layer_id } => {
                layer_id
            }
            // A linked transfer intentionally leaves its layer absent from
            // one of the participating scenes; the workspace owns its guard.
            HistoryStepKind::Transfer => return,
        };
        let layer_exists = scene
            .meshes()
            .iter()
            .any(|entry| entry.id() == undo_layer_id);
        if !layer_exists && !self.has_active_session() {
            self.history.borrow_mut().clear_scope(self.history_scope);
        }
    }
}
