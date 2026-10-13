//! Document-owned state: scene content, selection, unsaved tracking, and the
//! load pipeline that mutates them.
//!
//! Owned invariants:
//!
//! - `scene` is the single authoritative content handle; in-place edits go
//!   through [`DocumentState::live_scene_mut`] (borrowed scene) or
//!   [`taken_scene_mut`] (take-edit-restore in the sculpt worker), so a second
//!   live handle — a reader that would not see the edit — fails a test instead
//!   of passing silently.
//! - `unsaved_edit_layer_ids` names exactly the layers whose in-scene mesh
//!   differs from disk; every applied mesh edit and its undo/redo routes
//!   through [`DocumentState::mark_mesh_edits_unsaved`].
//! - `unsaved_drag_pose` tracks an open Align drag separately from committed
//!   edits, so returning to the starting pose cannot clear earlier work.
//! - `unsaved_sculpt_stroke` tracks a live Sculpt stroke until its result is
//!   committed or discarded.
//! - `edit_mode` owns selection and undo/redo; structural swaps re-sync it.
//! - Imports are scheduled by the workspace and carry a scene lifetime key;
//!   camera-reset decisions stay with their target document.
//!
//! Permitted mutation entry points: [`DocumentState::new`] for bootstrap,
//! scene-commit and layer-edit helpers for content, the loading pipeline for
//! arrivals. Cross-domain outputs: committed scenes feed the render mirrors,
//! undo entries feed the UI.

use super::egui;
use crate::edit_mode::EditModeController;
use crate::scene_loading::{replace_result_requires_guard, LoadQueueCameraReset};
use occluview_core::{Scene, SceneMeshId};
use std::sync::Arc;

pub(super) struct DocumentState {
    pub(super) scene: Option<Arc<Scene>>,
    pub(super) current_paths: Vec<std::path::PathBuf>,
    /// Source paths of layers that left the scene while its history can still
    /// bring them back. A history step restores a layer's mesh, and without
    /// this the layer would return with no name and no file to save to.
    pub(super) departed_layer_paths: std::collections::HashMap<SceneMeshId, std::path::PathBuf>,
    pub(super) focused_layer_id: Option<SceneMeshId>,
    edit_metadata: Option<EditMetadata>,
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
    /// second toggle restores the original value.
    pub(super) translucent_layer_restore: std::collections::HashMap<SceneMeshId, f32>,
    pub(super) mesh_selection_drag: Option<MeshSelectionDrag>,
    pub(super) load_queue_camera_reset: LoadQueueCameraReset,
    pub(super) camera_modified_during_load: bool,
    /// Whether an open Align drag differs from its starting pose.
    pub(super) unsaved_drag_pose: bool,
    pub(super) unsaved_sculpt_stroke: bool,
}

/// Document metadata is checkpointed with the Edit scene baseline. Structural
/// edits can remove source paths and dirty flags that Cancel must restore.
struct EditMetadata {
    paths: Vec<std::path::PathBuf>,
    dirty: std::collections::BTreeSet<SceneMeshId>,
    hidden: Vec<SceneMeshId>,
    translucent: std::collections::HashMap<SceneMeshId, f32>,
    focused: Option<SceneMeshId>,
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
        "scene edited in place while another handle was alive; that reader \
         will not observe the edit, and the scene container is copied until \
         the handle is released"
    );
}

/// The take-edit-restore path: the scene handle is out of the document for the
/// duration of the edit, so the same single-owner invariant applies. See
/// [`DocumentState::live_scene_mut`] for what a second handle means and costs.
pub(super) fn taken_scene_mut(scene: &mut Arc<Scene>) -> &mut Scene {
    let handles = Arc::strong_count(scene);
    debug_assert_eq!(
        handles, 1,
        "in-place scene edit while another Arc<Scene> is alive: the edit would \
         be invisible to that reader, and the scene container plus its \
         per-layer metadata are copied in the meantime"
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
            current_paths: Vec::new(),
            departed_layer_paths: std::collections::HashMap::new(),
            focused_layer_id: None,
            edit_metadata: None,
            content_revision: 0,
            edit_mode: EditModeController::default(),
            unsaved_edit_layer_ids: std::collections::BTreeSet::new(),
            hidden_layer_stack: Vec::new(),
            translucent_layer_restore: std::collections::HashMap::new(),
            mesh_selection_drag: None,
            load_queue_camera_reset: LoadQueueCameraReset::Idle,
            camera_modified_during_load: false,
            unsaved_drag_pose: false,
            unsaved_sculpt_stroke: false,
        }
    }

    /// Mutate the document's scene while it holds the only scene handle.
    /// A second handle would read an outdated scene after `Arc::make_mut`.
    /// Mesh geometry remains shared through `Arc<Mesh>`.
    pub(super) fn live_scene_mut(&mut self) -> Option<&mut Scene> {
        let scene = self.scene.as_mut()?;
        let handles = Arc::strong_count(scene);
        debug_assert_eq!(
            handles, 1,
            "in-place scene edit while another Arc<Scene> is alive: the edit \
             would be invisible to that reader, and the scene container plus \
             its per-layer metadata are copied in the meantime"
        );
        if handles != 1 {
            report_shared_scene_edit(handles);
        }
        Some(Arc::make_mut(scene))
    }

    /// Record that `layer_id` now differs from what was loaded from disk.
    /// Every mesh-edit success path (including undo/redo) routes through here
    /// so the save flow knows which layers to offer for export.
    pub(super) fn mark_mesh_edits_unsaved(&mut self, layer_id: SceneMeshId) {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.unsaved_edit_layer_ids.insert(layer_id);
    }

    /// Whether committed edits or an open Align drag need saving.
    pub(super) fn has_unsaved_mesh_edits(&self) -> bool {
        !self.unsaved_edit_layer_ids.is_empty()
            || self.unsaved_drag_pose
            || self.unsaved_sculpt_stroke
    }

    /// Whether replacing the scene now would discard work that is still only
    /// in memory. This is the dirty side of the replacement guard: an active
    /// edit session, an edit that has not been saved, or an unsaved drag or
    /// sculpt stroke all count.
    pub(super) fn replacement_needs_guard(&self) -> bool {
        self.edit_mode.has_active_session()
            || self.edit_mode.is_dirty()
            || self.has_unsaved_mesh_edits()
    }

    /// Recheck a Replace against the authorisation captured when the operator
    /// accepted it. `replace_result_requires_guard` stays the pure revision
    /// contract; this owns the document-side active-session and dirty checks
    /// that are the same at dispatch and completion.
    pub(super) fn replace_requires_guard(
        &self,
        content_revision_at_request: u64,
        dirty_at_request: bool,
    ) -> bool {
        (!dirty_at_request && self.edit_mode.has_active_session())
            || replace_result_requires_guard(
                content_revision_at_request,
                self.content_revision,
                dirty_at_request,
                self.replacement_needs_guard(),
                self.edit_mode.is_busy(),
            )
    }

    /// Begin a scene install before any cross-owner teardown runs.
    pub(super) fn begin_scene_install(&mut self) {
        self.content_revision = self.content_revision.wrapping_add(1);
    }

    /// Point edit-mode selection and history at an incoming scene.
    pub(super) fn sync_edit_mode_to_scene(&mut self, scene: &Scene) {
        self.edit_mode.sync_to_scene(scene);
    }

    /// Replace the document's scene content after the structural owners have
    /// dropped their claims on the outgoing scene.
    pub(super) fn install_scene_content(&mut self, mut scene: Scene) {
        self.unsaved_sculpt_stroke = false;
        // A measured map describes one pose. The incoming scene install clears
        // the alignment result, so its restored overlay must not survive.
        for entry in scene.meshes_mut() {
            if entry.overlay_kind() == Some(occluview_core::OverlayKind::Measured) {
                entry.clear_overlay();
            }
        }
        self.scene = Some(Arc::new(scene));
    }

    /// Replace the scene handle for a material-only update.
    pub(super) fn replace_scene_handle(&mut self, scene: Scene) {
        self.scene = Some(Arc::new(scene));
    }

    /// Detach an interaction that indexes the scene mesh list.
    pub(super) fn clear_mesh_selection_drag(&mut self) {
        self.mesh_selection_drag = None;
    }

    /// Begin clearing a scene. The caller keeps the returned handle alive until
    /// align/worker owners revoke their claims for this generation.
    pub(super) fn take_scene_for_clear(&mut self) -> Option<Arc<Scene>> {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.scene.take()
    }

    /// Drop scene content and its per-layer unsaved tracking after the
    /// departing handle has been released.
    pub(super) fn clear_scene_content(&mut self) {
        self.unsaved_sculpt_stroke = false;
        self.clear_unsaved_mesh_edits();
        self.hidden_layer_stack.clear();
        self.translucent_layer_restore.clear();
        self.scene = None;
    }

    /// Drop paths that belonged to the cleared scene.
    pub(super) fn clear_scene_paths(&mut self) {
        self.current_paths.clear();
        self.departed_layer_paths.clear();
    }

    /// Finish document-side clear bookkeeping after render mirrors are gone.
    pub(super) fn finish_scene_clear(&mut self) {
        self.mesh_selection_drag = None;
        self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
        self.camera_modified_during_load = false;
        self.edit_mode.clear();
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

    pub(super) fn capture_edit_metadata(&mut self) {
        if self.edit_metadata.is_none() {
            self.edit_metadata = Some(EditMetadata {
                paths: self.current_paths.clone(),
                dirty: self.unsaved_edit_layer_ids.clone(),
                hidden: self.hidden_layer_stack.clone(),
                translucent: self.translucent_layer_restore.clone(),
                focused: self.focused_layer_id,
            });
        }
    }

    pub(super) fn restore_edit_metadata(&mut self) {
        if let Some(metadata) = self.edit_metadata.take() {
            self.current_paths = metadata.paths;
            self.unsaved_edit_layer_ids = metadata.dirty;
            self.hidden_layer_stack = metadata.hidden;
            self.translucent_layer_restore = metadata.translucent;
            self.focused_layer_id = metadata.focused;
        }
    }

    pub(super) fn discard_edit_metadata(&mut self) {
        self.edit_metadata = None;
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
