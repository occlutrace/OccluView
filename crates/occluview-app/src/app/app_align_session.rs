//! The Align Scans session: what Cancel and Done mean, and how the un-mapped
//! scan gets out of the way.
//!
//! Split from `app_align` because it answers a different question. That module
//! routes clicks and jobs; this one owns the transaction the operator commits
//! or throws away, and the display trick that makes one coloured surface
//! readable.

use eframe::egui;
use occluview_core::SceneMeshId;

use super::OccluViewApp;
use crate::edit_mode::EditModeCommand;
use glam::Affine3A;
use std::sync::Arc;

/// How solid the un-mapped scan stays while the heatmap is up. Enough to keep
/// the shape readable, faint enough that it never covers the coloured surface.
const GHOST_OPACITY: f32 = 0.16;

impl OccluViewApp {
    /// Close the tool and put every scan back where the session found it.
    pub(super) fn cancel_align_session(&mut self, ctx: &egui::Context) {
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

    /// Which layer carries the map: the one that moved.
    ///
    /// There used to be a control that put the map on the other surface
    /// instead. It asked the operator a rendering question dressed up as a
    /// measurement one — the distances are the same either way — so it is gone,
    /// and the answer is now always "the scan you are placing".
    pub(super) fn align_mapped_layer(&self) -> Option<SceneMeshId> {
        self.align.moving_layer()
    }

    /// The layer the map is *not* on, which is the one that has to get out of
    /// the way.
    fn align_other_layer(&self) -> Option<SceneMeshId> {
        self.align.fixed_layer()
    }

    /// Fade the other scan while the map is up.
    ///
    /// Two solid surfaces a fraction of a millimetre apart interpenetrate, and
    /// the coloured one is then only visible in patches. Lab software shows one
    /// clean coloured surface; this is how.
    pub(super) fn ghost_other_layer(&mut self) {
        if !self.align_ghosted.is_empty() {
            return;
        }
        let Some(other) = self.align_other_layer() else {
            return;
        };
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        // Opacity is a material, not a structure: mutate the live scene in
        // place rather than replacing it, or fading one layer would copy every
        // mesh in the scene and force a full GPU rebuild.
        let live = Arc::make_mut(scene);
        let mut remembered = Vec::new();
        for entry in live.meshes_mut() {
            if entry.id() == other {
                remembered.push((entry.id(), entry.opacity));
                entry.opacity = GHOST_OPACITY;
            }
        }
        if remembered.is_empty() {
            return;
        }
        self.align_ghosted = remembered;
        self.mark_scene_materials_changed();
    }

    /// Bring the faded scan back.
    pub(super) fn unghost_layers(&mut self) {
        if self.align_ghosted.is_empty() {
            return;
        }
        let restore = std::mem::take(&mut self.align_ghosted);
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        let live = Arc::make_mut(scene);
        for (id, opacity) in restore {
            if let Some(entry) = live.meshes_mut().iter_mut().find(|entry| entry.id() == id) {
                entry.opacity = opacity;
            }
        }
        self.mark_scene_materials_changed();
    }
}
