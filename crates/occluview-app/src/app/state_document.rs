//! Document-owned state: scene content, selection, unsaved tracking, and the
//! load pipeline that mutates them.
//!
//! Owned invariants:
//!
//! - `scene` is the single authoritative content handle; in-place edits go
//!   through [`DocumentState::live_scene_mut`] (borrowed scene) or
//!   [`taken_scene_mut`] (take-edit-restore in the sculpt worker) so a
//!   second live handle fails a test instead of silently deep-copying the
//!   case every frame.
//! - `unsaved_edit_layer_ids` names exactly the layers whose in-scene mesh
//!   differs from disk; every applied mesh edit and its undo/redo routes
//!   through [`DocumentState::mark_mesh_edits_unsaved`].
//! - `edit_mode` owns selection and undo/redo; structural swaps re-sync it.
//! - `active_load` / `queued_loads` mutate only the document; the camera
//!   reset decision and the modified-during-load flag live here with them.
//!
//! Permitted mutation entry points: [`DocumentState::new`] for bootstrap,
//! scene-commit and layer-edit helpers for content, the loading pipeline for
//! arrivals. Cross-domain outputs: committed scenes feed the render mirrors,
//! undo entries feed the UI.

use super::egui;
use crate::edit_mode::EditModeController;
use crate::scene_loading::{LoadQueueCameraReset, PendingSceneLoad, SceneLoadRequest};
use occluview_core::{Scene, SceneMeshId};
use std::sync::Arc;

pub(super) struct DocumentState {
    pub(super) scene: Option<Arc<Scene>>,
    /// Bumped when committed scene content or unsaved mesh edits change.
    pub(super) content_revision: u64,
    pub(super) edit_mode: EditModeController,
    /// Layers carrying unsaved edits: the in-scene mesh differs from what was
    /// loaded from disk. Written by every applied mesh-edit and its undo/redo,
    /// cleared per layer when that layer is written out, and entirely when the
    /// scene is replaced or closed. The close-without-saving guard and the save
    /// flow both read it, through [`Self::has_unsaved_mesh_edits`].
    pub(super) unsaved_edit_layer_ids: std::collections::BTreeSet<SceneMeshId>,
    /// Layers hidden via Ctrl+MiddleClick, in hide order. Shift+Ctrl+Middle
    /// restores the most recently hidden one (LIFO).
    pub(super) hidden_layer_stack: Vec<SceneMeshId>,
    /// Original opacity of layers made translucent via Shift+MiddleClick, so a
    /// second toggle restores exactly the previous value.
    pub(super) translucent_layer_restore: std::collections::HashMap<SceneMeshId, f32>,
    pub(super) mesh_selection_drag: Option<MeshSelectionDrag>,
    pub(super) active_load: Option<PendingSceneLoad>,
    pub(super) queued_loads: std::collections::VecDeque<SceneLoadRequest>,
    pub(super) load_queue_camera_reset: LoadQueueCameraReset,
    pub(super) camera_modified_during_load: bool,
}

/// In-progress mesh selection drag. Rectangle drags (default) track an origin
/// and current corner; an armed lasso collects the freehand outline points.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MeshSelectionDrag {
    Rect {
        origin: egui::Pos2,
        current: egui::Pos2,
    },
    Lasso {
        points: Vec<egui::Pos2>,
    },
}

impl MeshSelectionDrag {
    /// Axis-aligned extent of the drag (the rectangle for `Rect`, the bounding
    /// box of the collected outline for `Lasso`).
    pub(super) fn rect(&self) -> egui::Rect {
        match self {
            Self::Rect { origin, current } => egui::Rect::from_two_pos(*origin, *current),
            Self::Lasso { points } => {
                let mut bbox = egui::Rect::NOTHING;
                for &point in points {
                    bbox.extend_with(point);
                }
                bbox
            }
        }
    }
}

/// Report an unexpected shared scene handle once per process.
fn report_shared_scene_edit(handles: usize) {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    tracing::warn!(
        handles,
        "scene edited in place while another handle was alive; this copies the \
         whole case and will keep doing so until the handle is released"
    );
}

pub(super) fn taken_scene_mut(scene: &mut Arc<Scene>) -> &mut Scene {
    let handles = Arc::strong_count(scene);
    debug_assert_eq!(
        handles, 1,
        "in-place scene edit while another Arc<Scene> is alive: this \
         silently deep-copies every vertex, index and texture of the case"
    );
    if handles != 1 {
        report_shared_scene_edit(handles);
    }
    Arc::make_mut(scene)
}

impl DocumentState {
    pub(super) fn new() -> Self {
        Self {
            scene: None,
            content_revision: 0,
            edit_mode: EditModeController::default(),
            unsaved_edit_layer_ids: std::collections::BTreeSet::new(),
            hidden_layer_stack: Vec::new(),
            translucent_layer_restore: std::collections::HashMap::new(),
            mesh_selection_drag: None,
            active_load: None,
            queued_loads: std::collections::VecDeque::new(),
            load_queue_camera_reset: LoadQueueCameraReset::Idle,
            camera_modified_during_load: false,
        }
    }

    /// The live scene, mutable in place (the borrowed-handle path; the
    /// take-edit-restore path is [`taken_scene_mut`]).
    ///
    /// `Arc::make_mut` copies the whole scene whenever a second handle exists,
    /// and the callers below all run per frame: a slider drag, a brush dab, a
    /// nudge of an aligned layer. On two 945k-vertex arches that is 40 ns as
    /// sole handle against 45.75 ms otherwise -- the entire case copied every
    /// frame to change a few numbers.
    ///
    /// Every in-place scene edit comes through here, so a caller that keeps a
    /// handle alive across the edit fails a test instead of costing frames.
    pub(super) fn live_scene_mut(&mut self) -> Option<&mut Scene> {
        let scene = self.scene.as_mut()?;
        let handles = Arc::strong_count(scene);
        debug_assert_eq!(
            handles, 1,
            "in-place scene edit while another Arc<Scene> is alive: this \
             silently deep-copies every vertex, index and texture of the case"
        );
        if handles != 1 {
            report_shared_scene_edit(handles);
        }
        Some(Arc::make_mut(scene))
    }

    /// Record that `layer_id` now differs from what was loaded from disk.
    /// Every mesh-edit success path (including undo/redo) routes through here
    /// so the save flow knows exactly which layers to offer for export.
    pub(super) fn mark_mesh_edits_unsaved(&mut self, layer_id: SceneMeshId) {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.unsaved_edit_layer_ids.insert(layer_id);
    }

    /// Whether anything in the scene differs from what is on disk.
    ///
    /// Derived from the set of layers with pending edits.
    pub(super) fn has_unsaved_mesh_edits(&self) -> bool {
        !self.unsaved_edit_layer_ids.is_empty()
    }

    /// Forget the unsaved-edit tracking for the layers just written to disk.
    ///
    /// Clear only layers included in the save operation.
    pub(super) fn forget_unsaved_edits(&mut self, layers: &[SceneMeshId]) {
        for layer in layers {
            self.unsaved_edit_layer_ids.remove(layer);
        }
    }

    /// Forget all unsaved-edit tracking (scene replaced, closed, or saved).
    pub(super) fn clear_unsaved_mesh_edits(&mut self) {
        self.unsaved_edit_layer_ids.clear();
    }

    pub(super) fn mark_camera_modified(&mut self) {
        if self.active_load.is_some() || !self.queued_loads.is_empty() {
            self.camera_modified_during_load = true;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use occluview_core::{Mesh, SceneMesh, Vertex};

    fn layer_id() -> SceneMeshId {
        let mesh = Mesh::new(
            Some("unsaved-test".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(-1.0, -1.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, -1.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 1.0, 0.0)),
                Vertex::at(glam::Vec3::new(-1.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("test mesh");
        SceneMesh::new(mesh).id()
    }

    #[test]
    fn document_starts_empty_without_unsaved_edits() {
        let document = DocumentState::new();

        assert!(document.scene.is_none());
        assert!(!document.has_unsaved_mesh_edits());
    }

    #[test]
    fn unsaved_tracking_round_trips_per_layer() {
        let mut document = DocumentState::new();
        let layer = layer_id();

        document.mark_mesh_edits_unsaved(layer);
        assert!(document.has_unsaved_mesh_edits());
        document.forget_unsaved_edits(&[layer]);
        assert!(!document.has_unsaved_mesh_edits());

        document.mark_mesh_edits_unsaved(layer);
        document.clear_unsaved_mesh_edits();
        assert!(!document.has_unsaved_mesh_edits());
    }
}
