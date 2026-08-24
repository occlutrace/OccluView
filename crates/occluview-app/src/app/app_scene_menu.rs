//! Scene actions shown by the empty-viewport context menu.
//!
//! OccluView has no project file, so aligned transforms persist through export.

use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
use crate::layers_overlay::SceneContextAction;
use eframe::egui;
use glam::Affine3A;

impl OccluViewApp {
    /// Whether the scene menu has anything to offer: any layer at all, and any
    /// layer that has actually been moved.
    pub(super) fn scene_menu_state(&self) -> (bool, bool) {
        let Some(scene) = self.scene.as_ref() else {
            return (false, false);
        };
        let has_layers = !scene.meshes().is_empty();
        let any_moved = scene
            .meshes()
            .iter()
            .any(|entry| entry.transform != Affine3A::IDENTITY);
        (has_layers, any_moved)
    }

    /// Run one scene action.
    pub(super) fn apply_scene_context_action(
        &mut self,
        action: SceneContextAction,
        ctx: &egui::Context,
    ) {
        match action {
            SceneContextAction::SaveScene => self.save_scene_dialog(),
            SceneContextAction::SaveEachLayer => self.save_each_layer_dialog(),
            SceneContextAction::ResetPositions => self.reset_layer_positions(ctx),
            SceneContextAction::FitView => {
                self.reset_camera_to_home();
                ctx.request_repaint();
            }
        }
    }

    /// Return every layer to the identity pose, as one undo step.
    fn reset_layer_positions(&mut self, ctx: &egui::Context) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let mut next = scene.as_ref().clone();
        let Some(focus) = next.meshes().first().map(occluview_core::SceneMesh::id) else {
            return;
        };
        if next
            .meshes()
            .iter()
            .all(|entry| entry.transform == Affine3A::IDENTITY)
        {
            self.status_message = Some("Every layer is already at its original position".into());
            return;
        }

        let Some(token) = self
            .edit_mode
            .begin_scene_edit(&next, focus, EditModeCommand::MoveLayer)
        else {
            return;
        };
        for entry in next.meshes_mut() {
            entry.transform = Affine3A::IDENTITY;
        }
        self.edit_mode.finish_scene_edit_success(token, &next);
        let moved: Vec<occluview_core::SceneMeshId> = next
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();
        self.set_scene(next, false);
        for layer in moved {
            self.mark_mesh_edits_unsaved(layer);
        }
        self.status_message = Some("Layer positions reset (Ctrl+Z undoes)".into());
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    /// Resetting positions must go through the scene history, or Ctrl+Z would
    /// silently do nothing after the operator wiped an alignment.
    #[test]
    fn resetting_positions_is_recorded_as_one_undoable_scene_edit() {
        let source = crate::primary_ui_tests::production_source(include_str!("app_scene_menu.rs"));
        assert!(
            source.contains("begin_scene_edit(&next, focus, EditModeCommand::MoveLayer)"),
            "a reset must open a scene history step"
        );
        assert!(
            source.contains("finish_scene_edit_success(token, &next)"),
            "a reset must close the scene history step it opened"
        );
    }
}
