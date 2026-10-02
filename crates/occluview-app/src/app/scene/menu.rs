//! Scene actions shown by the empty-viewport context menu.
//!
//! OccluView has no project file, so aligned transforms persist through export.

use super::super::SceneContext;
use crate::edit_mode::EditModeCommand;
use crate::layers_overlay::SceneContextAction;
use eframe::egui;
use glam::Affine3A;

impl SceneContext<'_> {
    /// Whether the scene menu has anything to offer: any layer at all, and any
    /// layer that has actually been moved.
    pub(in crate::app) fn scene_menu_state(&self) -> (bool, bool) {
        let Some(scene) = self.document.scene.as_ref() else {
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
    pub(in crate::app) fn apply_scene_context_action(
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
    pub(in crate::app) fn reset_layer_positions(&mut self, ctx: &egui::Context) {
        let Some(scene) = self.document.scene.clone() else {
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
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("scene-already-origin")),
            );
            return;
        }

        let Some(token) =
            self.document
                .edit_mode
                .begin_scene_edit(&next, focus, EditModeCommand::MoveLayer)
        else {
            return;
        };
        for entry in next.meshes_mut() {
            entry.transform = Affine3A::IDENTITY;
        }
        self.document
            .edit_mode
            .finish_scene_edit_success(token, &next);
        let moved: Vec<occluview_core::SceneMeshId> = next
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();
        self.set_scene(next, false);
        for layer in moved {
            self.document.mark_mesh_edits_unsaved(layer);
        }
        self.scene_ui.status_message = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("scene-positions-reset")),
        );
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
    use glam::Vec3;
    use std::sync::Arc;

    /// Reset Positions restores all poses and creates one undo step.
    #[test]
    fn resetting_positions_is_one_undoable_step() {
        let mut app = test_app("reset-positions");
        let mut scene = named_scene("lower", 0.0);
        push_named_layer(&mut scene, "upper", 5.0);
        scene.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::new(2.0, 1.0, 0.0));
        scene.meshes_mut()[1].transform = Affine3A::from_translation(Vec3::new(-3.0, 0.5, 1.0));
        let moved = [scene.meshes()[0].transform, scene.meshes()[1].transform];
        assert_ne!(moved[0], Affine3A::IDENTITY, "fixture: layer 0 is moved");
        assert_ne!(moved[1], Affine3A::IDENTITY, "fixture: layer 1 is moved");
        app.workspace.scenes[0].document.scene = Some(Arc::new(scene));

        app.active_context()
            .expect("live test scene")
            .reset_layer_positions(&egui::Context::default());

        let scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        assert!(
            scene
                .meshes()
                .iter()
                .all(|entry| entry.transform == Affine3A::IDENTITY),
            "every layer returns to the identity pose"
        );
        assert!(
            app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
            "a reset is unsaved work: the viewer has no project file"
        );

        // One step back restores both poses, not just the focused layer's.
        app.active_context()
            .expect("live test scene")
            .apply_history_navigation_now(false, &egui::Context::default());

        let restored = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        assert_eq!(restored.meshes()[0].transform, moved[0]);
        assert_eq!(restored.meshes()[1].transform, moved[1]);
    }
}
