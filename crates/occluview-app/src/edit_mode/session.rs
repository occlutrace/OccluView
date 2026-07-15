//! Edit-session lifecycle: begin/Done/Cancel, dirty/busy state, full reset.

use occluview_core::{Scene, SceneMesh};

use super::state::{EditModeState, LayerKey, SelectGesture};
use super::EditModeController;

impl EditModeController {
    pub(crate) fn begin_face_selection(&mut self, layer: &SceneMesh, scene: &Scene) -> bool {
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
        let starting_session = self.baseline_scene.is_none();
        self.compatibility_layer_id = Some(layer.id());
        let _ = self.state.start(LayerKey::from_scene_mesh_id(layer.id()));

        // Capture the pre-edit scene the first time a session opens, so Cancel
        // can revert every edit (including structural additions) in one step.
        if starting_session {
            self.baseline_scene = Some(scene.clone());
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
        self.compatibility_layer_id = None;
        self.baseline_scene = None;
        self.session_dirty = false;
        self.session_layer_id = None;
        self.gesture = SelectGesture::default();
        self.state.confirm_discard();
    }

    /// Revert the whole edit session (Cancel), returning the whole-scene
    /// baseline captured on entry.
    pub(crate) fn cancel_edit_session(&mut self) -> Option<Scene> {
        let baseline = self.baseline_scene.take()?;
        self.selections.clear();
        self.compatibility_layer_id = None;
        self.session_layer_id = None;
        self.session_dirty = false;
        self.undo.clear();
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
        self.undo.clear();
        self.selections.clear();
        self.compatibility_layer_id = None;
        self.gesture = SelectGesture::default();
        self.baseline_scene = None;
        self.session_dirty = false;
        self.session_layer_id = None;
    }
}
