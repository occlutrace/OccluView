//! The Align Scans session: what Cancel and Done mean, and how the un-mapped
//! scan gets out of the way.
//!
//! Split from `app_align` because it answers a different question. That module
//! routes clicks and jobs; this one owns the transaction the operator commits
//! or throws away, and the display trick that makes one coloured surface
//! readable.

use eframe::egui;
use occluview_core::SceneMeshId;

use super::app_align::layer_of;
use super::OccluViewApp;

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
    pub(super) fn align_session_moved(&self) -> bool {
        let Some(scene) = self.scene.as_ref() else {
            return false;
        };
        self.align_session_poses
            .iter()
            .any(|(id, pose)| layer_of(scene, *id).is_some_and(|entry| entry.transform != *pose))
    }

    /// Put every layer back to the pose the session started from.
    fn restore_session_poses(&mut self) -> bool {
        if !self.align_session_moved() {
            return false;
        }
        let Some(scene) = self.scene.clone() else {
            return false;
        };
        let mut next = scene.as_ref().clone();
        for (id, pose) in &self.align_session_poses {
            if let Some(entry) = next.meshes_mut().iter_mut().find(|entry| entry.id() == *id) {
                entry.transform = *pose;
            }
        }
        self.set_scene(next, false);
        true
    }

    /// Which layer carries the map. The measured surface is the one that moved,
    /// but an operator often wants to read the map on the one that stayed —
    /// that is a display choice, not a role.
    pub(super) fn align_mapped_layer(&self) -> Option<SceneMeshId> {
        if self.align_map_on_fixed {
            self.align.fixed_layer()
        } else {
            self.align.moving_layer()
        }
    }

    /// The layer the map is *not* on, which is the one that has to get out of
    /// the way.
    fn align_other_layer(&self) -> Option<SceneMeshId> {
        if self.align_map_on_fixed {
            self.align.moving_layer()
        } else {
            self.align.fixed_layer()
        }
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
        let (Some(scene), Some(other)) = (self.scene.clone(), self.align_other_layer()) else {
            return;
        };
        let mut next = scene.as_ref().clone();
        let mut remembered = Vec::new();
        for entry in next.meshes_mut() {
            if entry.id() == other {
                remembered.push((entry.id(), entry.opacity));
                entry.opacity = GHOST_OPACITY;
            }
        }
        if remembered.is_empty() {
            return;
        }
        self.align_ghosted = remembered;
        self.set_scene(next, false);
    }

    /// Bring the faded scan back.
    pub(super) fn unghost_layers(&mut self) {
        if self.align_ghosted.is_empty() {
            return;
        }
        let restore = std::mem::take(&mut self.align_ghosted);
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let mut next = scene.as_ref().clone();
        for (id, opacity) in restore {
            if let Some(entry) = next.meshes_mut().iter_mut().find(|entry| entry.id() == id) {
                entry.opacity = opacity;
            }
        }
        self.set_scene(next, false);
    }
}
