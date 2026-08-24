//! Align Scans session commit and rollback behavior.

use eframe::egui;
use occluview_core::SceneMeshId;

use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
use glam::Affine3A;

impl OccluViewApp {
    /// Close the tool and put every scan back where the session found it.
    pub(super) fn cancel_align_session(&mut self, ctx: &egui::Context) {
        // Drop an active drag before restoring session poses so Cancel cannot
        // record the discarded gesture as an undo step.
        self.align_drag = None;
        let restored = self.restore_session_poses();
        self.disarm_align_tool(ctx);
        self.status_message = Some(if restored {
            "Alignment cancelled — every scan is back where it was (Ctrl+Z brings it back)".into()
        } else {
            "Alignment closed".to_string()
        });
    }

    /// Close the tool and keep what it did.
    pub(super) fn finish_align_session(&mut self, ctx: &egui::Context) {
        let moved = self.align_session_moved();
        // Read before teardown cancels the worker so the status can report a
        // fit that was still running when the session closed.
        let running = self
            .align_worker
            .as_ref()
            .is_some_and(crate::align_worker::AlignWorker::is_busy);
        self.disarm_align_tool(ctx);
        self.status_message = Some(match (running, moved) {
            (true, _) => {
                "Alignment closed — a fit was still running and was dropped, so the scans are \
                 exactly as you last saw them"
                    .into()
            }
            (false, true) => "Alignment kept — save the scan to keep it on disk".to_string(),
            (false, false) => "Alignment closed".to_string(),
        });
    }

    /// Whether anything actually moved since the tool opened.
    ///
    /// Walks the live scene, not the snapshot: a layer that arrived mid-session
    /// is inside the transaction too, and reporting "nothing moved" after
    /// dragging it would be a lie Cancel then acts on.
    pub(super) fn align_session_moved(&self) -> bool {
        let Some(scene) = self.scene.as_ref() else {
            return false;
        };
        scene.meshes().iter().any(|entry| {
            self.session_pose_of(entry.id())
                .is_none_or(|pose| entry.transform != pose)
        })
    }

    /// The pose a layer had when the session opened, if it was there.
    fn session_pose_of(&self, layer: SceneMeshId) -> Option<Affine3A> {
        self.align_session_poses
            .iter()
            .find(|(id, _)| *id == layer)
            .map(|(_, pose)| *pose)
    }

    /// Put every layer back to the pose the session started from.
    ///
    /// The restore is itself one history step. Without it the scene changed
    /// under a history stack that still described the discarded poses, so
    /// Ctrl+Z after Cancel resurrected work the operator had just thrown away.
    /// As one step, Ctrl+Z means "actually, put the alignment back" — which is
    /// what an operator who cancelled by mistake wants.
    fn restore_session_poses(&mut self) -> bool {
        if !self.align_session_moved() {
            return false;
        }
        let Some(scene) = self.scene.clone() else {
            return false;
        };
        let mut next = scene.as_ref().clone();
        let Some(focus) = next.meshes().first().map(occluview_core::SceneMesh::id) else {
            return false;
        };
        let Some(token) = self
            .edit_mode
            .begin_scene_edit(&next, focus, EditModeCommand::MoveLayer)
        else {
            return false;
        };
        for entry in next.meshes_mut() {
            if let Some(pose) = self.session_pose_of(entry.id()) {
                entry.transform = pose;
            }
        }
        self.edit_mode.finish_scene_edit_success(token, &next);
        self.set_scene(next, false);
        true
    }
}
