//! The Align Scans session: what Cancel and Done mean.
//!
//! Split from `app_align` because it answers a different question. That module
//! routes clicks and jobs; this one owns the transaction the operator commits or
//! throws away. Fading the un-mapped scan used to live here too, and moved to
//! `app_align_display` — every caller was already there.

use eframe::egui;
use occluview_core::SceneMeshId;

use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
use glam::Affine3A;

impl OccluViewApp {
    /// Close the tool and put every scan back where the session found it.
    pub(super) fn cancel_align_session(&mut self, ctx: &egui::Context) {
        // An open hand gesture is dropped, not committed. Escape is read before
        // the drag handler, so the mouse button can still be down here. Restoring
        // first and closing the gesture afterwards — which is what the teardown
        // below does — made the release write a history step out of a pose that
        // the restore had already replaced: a state the scene had never actually
        // been in, recorded as the operator's own edit, and the scene marked
        // unsaved for a Cancel that changed nothing.
        self.align_drag = None;
        let restored = self.restore_session_poses();
        self.disarm_align_tool(ctx);
        self.status_message = Some(if restored {
            "Alignment cancelled — every scan is back where it was".into()
        } else {
            "Alignment closed".to_string()
        });
    }

    /// Close the tool and keep what it did.
    pub(super) fn finish_align_session(&mut self, ctx: &egui::Context) {
        let moved = self.align_session_moved();
        self.disarm_align_tool(ctx);
        self.status_message = Some(if moved {
            "Alignment kept — save the scan to keep it on disk".into()
        } else {
            "Alignment closed".to_string()
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
