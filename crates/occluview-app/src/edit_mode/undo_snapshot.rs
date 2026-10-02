use std::mem::{size_of, size_of_val};
use std::sync::Arc;

use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId};

use super::state::{BusyFinish, EditModeCommand, EditModeState, EditSessionToken, LayerKey};
use super::EditModeController;
use crate::app::workspace::history::{HistoryDirection, HistorySnapshot, HistoryStepKind};

#[derive(Clone, Debug)]
pub(super) enum MeshEditUndoSnapshot {
    Layer(SceneMesh),
    Scene {
        scene: Scene,
        /// The exact set of layer ids the live scene is expected to hold for
        /// this whole-scene snapshot to be safely restorable. A structural
        /// undo/redo swaps the entire scene, so it is only safe when the live
        /// scene still holds exactly these ids: a layer appended since would be
        /// deleted, and a layer removed since would be resurrected.
        /// Stamped to the post-op id-set when the op finishes (undo direction),
        /// or to the restored scene's id-set when a reverse step is pushed.
        guard_ids: Vec<SceneMeshId>,
    },
}

/// Outcome of attempting a structural (whole-scene) undo or redo step.
pub(crate) enum StructuralHistoryStep {
    /// No structural step is available in this direction: the history is empty
    /// or busy, targets a different layer, or the next step is a single-layer
    /// edit rather than a whole-scene snapshot.
    NotAvailable,
    /// A structural step exists, but the live scene gained or lost layers since
    /// it was recorded. A blind whole-scene restore would delete a layer added
    /// since (or resurrect one removed since), so it is refused — the caller
    /// reports a "scene changed since" status.
    SceneChanged,
    /// The scene to swap into the viewport.
    Restored(Scene),
}

/// The set of layer ids in `scene`, in scene order (ids are globally unique, so
/// this doubles as a set fingerprint for the structural-history guard).
fn scene_layer_ids(scene: &Scene) -> Vec<SceneMeshId> {
    scene.meshes().iter().map(SceneMesh::id).collect()
}

/// Whether the live scene holds exactly the guarded id-set (order-independent).
/// Ids are unique within a scene, so equal length plus containment is set
/// equality — no layer was appended or removed since the snapshot.
fn scene_matches_guard(scene: &Scene, guard_ids: &[SceneMeshId]) -> bool {
    scene.meshes().len() == guard_ids.len()
        && scene
            .meshes()
            .iter()
            .all(|entry| guard_ids.contains(&entry.id()))
}

impl EditModeController {
    pub(crate) fn begin_layer_edit(
        &mut self,
        layer: &SceneMesh,
        command: EditModeCommand,
    ) -> Option<EditSessionToken> {
        self.begin_layer_edit_with_snapshot(layer, layer.mesh.clone(), command)
    }

    /// Begin a layer edit using a prebuilt mesh snapshot. The layer's identity
    /// and display settings still come from the live entry, so undo restores
    /// the exact instance rather than a new layer.
    ///
    /// The snapshot is shared, not copied: geometry is immutable, so the
    /// history and the live scene can hold the same vertices until one of them
    /// is replaced.
    pub(crate) fn begin_layer_edit_with_snapshot(
        &mut self,
        layer: &SceneMesh,
        snapshot_mesh: Arc<Mesh>,
        command: EditModeCommand,
    ) -> Option<EditSessionToken> {
        let layer_key = LayerKey::from_scene_mesh_id(layer.id());
        if !self.state.start(layer_key) {
            return None;
        }
        let token = self.next_session_token();
        if !self.state.begin_busy(command, token) {
            return None;
        }
        let snapshot_bytes = scene_mesh_snapshot_bytes_from_mesh(&snapshot_mesh);
        self.last_undo_push_stored = false;
        self.pending_history_command = self.history.borrow_mut().push_pending_edit(
            self.history_scope,
            HistoryStepKind::LayerEdit {
                layer_id: layer.id(),
            },
            MeshEditUndoSnapshot::Layer(layer.with_mesh(snapshot_mesh)),
            snapshot_bytes,
        );
        self.last_undo_push_stored = self.pending_history_command.is_some();
        Some(token)
    }

    pub(crate) fn begin_scene_edit(
        &mut self,
        scene: &Scene,
        layer_id: SceneMeshId,
        command: EditModeCommand,
    ) -> Option<EditSessionToken> {
        let layer_key = LayerKey::from_scene_mesh_id(layer_id);
        if !self.state.start(layer_key) {
            return None;
        }
        let token = self.next_session_token();
        if !self.state.begin_busy(command, token) {
            return None;
        }
        self.last_undo_push_stored = false;
        self.pending_history_command = self.history.borrow_mut().push_pending_edit(
            self.history_scope,
            HistoryStepKind::SceneEdit { layer_id },
            MeshEditUndoSnapshot::Scene {
                scene: scene.clone(),
                // Placeholder = pre-op id-set. It is correct as-is if the op
                // never mutates the scene (the error path); the success path
                // re-stamps it to the post-op id-set in
                // `finish_scene_edit_success`.
                guard_ids: scene_layer_ids(scene),
            },
            scene_snapshot_bytes(scene),
        );
        self.last_undo_push_stored = self.pending_history_command.is_some();
        Some(token)
    }

    pub(crate) fn finish_layer_edit_success(&mut self, token: EditSessionToken) -> BusyFinish {
        let finish = self.state.finish_busy_success(token, true);
        if finish != BusyFinish::Applied {
            return finish;
        }
        // The op changed content: the redo history displaced by this op's
        // pre-op snapshot is now permanently invalid.
        self.last_undo_push_stored = self
            .pending_history_command
            .take()
            .is_some_and(|id| self.history.borrow_mut().commit_pending_edit(id));
        self.session_dirty = true;
        finish
    }

    /// Finish a structural (whole-scene) op that changed the scene. Stamps the
    /// just-pushed scene snapshot with the id-set the live scene now holds
    /// (post-op), so a later undo can tell whether the scene changed underneath
    /// the snapshot before restoring it wholesale.
    pub(crate) fn finish_scene_edit_success(
        &mut self,
        token: EditSessionToken,
        post_op_scene: &Scene,
    ) -> BusyFinish {
        let finish = self.state.finish_busy_success(token, true);
        if finish != BusyFinish::Applied {
            return finish;
        }
        if self.last_undo_push_stored {
            let post_op_ids = scene_layer_ids(post_op_scene);
            if let Some(command_id) = self.pending_history_command {
                self.history
                    .borrow_mut()
                    .update_pending::<MeshEditUndoSnapshot>(command_id, |snapshot| {
                        if let MeshEditUndoSnapshot::Scene { guard_ids, .. } = snapshot {
                            *guard_ids = post_op_ids;
                        }
                    });
            }
        }
        self.last_undo_push_stored = self
            .pending_history_command
            .take()
            .is_some_and(|id| self.history.borrow_mut().commit_pending_edit(id));
        self.session_dirty = true;
        finish
    }

    pub(crate) fn finish_layer_edit_error(
        &mut self,
        token: EditSessionToken,
        message: String,
    ) -> BusyFinish {
        let finish = self.state.finish_busy_error(token, message);
        if finish != BusyFinish::Applied {
            return finish;
        }
        if let Some(command_id) = self.pending_history_command.take() {
            self.history.borrow_mut().discard_pending(command_id);
        }
        self.last_undo_push_stored = false;
        finish
    }

    /// Finish a busy op that turned out to change nothing: the pre-op undo
    /// snapshot is discarded (no phantom undo step) and the session's dirty
    /// state stays exactly as it was before the op.
    pub(crate) fn finish_layer_edit_noop(&mut self, token: EditSessionToken) -> BusyFinish {
        let finish = self.state.finish_busy_success(token, false);
        if finish == BusyFinish::Applied {
            if let Some(command_id) = self.pending_history_command.take() {
                self.history.borrow_mut().discard_pending(command_id);
            }
            self.last_undo_push_stored = false;
        }
        finish
    }

    /// Whether the most recent `begin_*_edit` stored its pre-op snapshot. An
    /// oversized snapshot is skipped: the edit applies but cannot be undone,
    /// and the caller reports that to the operator.
    pub(crate) fn last_edit_undoable(&self) -> bool {
        self.last_undo_push_stored
    }

    pub(crate) fn undo_layer_id(&self) -> Option<SceneMeshId> {
        let step = self
            .history
            .borrow()
            .top_step(self.history_scope, HistoryDirection::Undo)?;
        match step.kind {
            HistoryStepKind::LayerEdit { layer_id } | HistoryStepKind::SceneEdit { layer_id } => {
                Some(layer_id)
            }
            HistoryStepKind::Transfer => None,
        }
    }

    pub(crate) fn redo_layer_id(&self) -> Option<SceneMeshId> {
        let step = self
            .history
            .borrow()
            .top_step(self.history_scope, HistoryDirection::Redo)?;
        match step.kind {
            HistoryStepKind::LayerEdit { layer_id } | HistoryStepKind::SceneEdit { layer_id } => {
                Some(layer_id)
            }
            HistoryStepKind::Transfer => None,
        }
    }

    /// Re-apply the last undone layer edit (Ctrl+Y). The redo stack is
    /// cleared whenever a new op pushes an undo snapshot, so this can only
    /// replay the undone chain.
    pub(crate) fn redo_last_layer_edit(&mut self, current: &SceneMesh) -> Option<SceneMesh> {
        self.navigate_layer_edit(current, HistoryDirection::Redo)
    }

    /// Re-apply the last undone structural (whole-scene) edit.
    pub(crate) fn redo_last_scene_edit(
        &mut self,
        current: &Scene,
        layer_id: SceneMeshId,
    ) -> StructuralHistoryStep {
        self.navigate_scene_edit(current, layer_id, HistoryDirection::Redo)
    }

    pub(crate) fn undo_last_layer_edit(&mut self, current: &SceneMesh) -> Option<SceneMesh> {
        self.navigate_layer_edit(current, HistoryDirection::Undo)
    }

    pub(crate) fn undo_last_scene_edit(
        &mut self,
        current: &Scene,
        layer_id: SceneMeshId,
    ) -> StructuralHistoryStep {
        self.navigate_scene_edit(current, layer_id, HistoryDirection::Undo)
    }

    /// Undo/redo a single-layer snapshot in `direction`, or bail (`None`) if
    /// busy, the layer doesn't match, or the top-of-stack snapshot isn't a
    /// layer snapshot (it's a structural scene snapshot instead).
    fn navigate_layer_edit(
        &mut self,
        current: &SceneMesh,
        direction: HistoryDirection,
    ) -> Option<SceneMesh> {
        if matches!(self.state, EditModeState::Busy { .. }) {
            return None;
        }
        let step = self
            .history
            .borrow()
            .top_step(self.history_scope, direction)?;
        if step.kind
            != (HistoryStepKind::LayerEdit {
                layer_id: current.id(),
            })
        {
            return None;
        }
        if !matches!(
            self.history
                .borrow()
                .payload::<MeshEditUndoSnapshot>(step.command_id),
            Some(MeshEditUndoSnapshot::Layer(_))
        ) {
            return None;
        }
        let restored = match self.history.borrow_mut().navigate_edit(
            self.history_scope,
            direction,
            step.command_id,
            HistorySnapshot {
                value: MeshEditUndoSnapshot::Layer(current.clone()),
                bytes: scene_mesh_snapshot_bytes(current),
            },
        )? {
            MeshEditUndoSnapshot::Layer(restored) => restored,
            MeshEditUndoSnapshot::Scene { .. } => return None,
        };
        self.state = EditModeState::ActiveDirty {
            layer: LayerKey::from_scene_mesh_id(restored.id()),
        };
        Some(restored)
    }

    /// Undo/redo a structural (whole-scene) snapshot in `direction`. Mirrors
    /// `navigate_layer_edit` for the `MeshEditUndoSnapshot::Scene` variant, plus
    /// the structural-history guard: a whole-scene restore is refused when the
    /// live scene gained or lost layers since the snapshot was recorded, so an
    /// appended layer is never deleted (nor a removed one resurrected).
    fn navigate_scene_edit(
        &mut self,
        current: &Scene,
        layer_id: SceneMeshId,
        direction: HistoryDirection,
    ) -> StructuralHistoryStep {
        if matches!(self.state, EditModeState::Busy { .. }) {
            return StructuralHistoryStep::NotAvailable;
        }
        let Some(step) = self
            .history
            .borrow()
            .top_step(self.history_scope, direction)
        else {
            return StructuralHistoryStep::NotAvailable;
        };
        if step.kind != (HistoryStepKind::SceneEdit { layer_id }) {
            return StructuralHistoryStep::NotAvailable;
        }
        // Inspect the step under the immutable borrow: it must be a scene
        // snapshot, its guard must still match the live scene, and the reverse
        // step we are about to push must guard against the scene we restore.
        let reverse_guard = {
            let history = self.history.borrow();
            let Some(MeshEditUndoSnapshot::Scene {
                scene: restore_scene,
                guard_ids,
                ..
            }) = history.payload::<MeshEditUndoSnapshot>(step.command_id)
            else {
                return StructuralHistoryStep::NotAvailable;
            };
            if !scene_matches_guard(current, guard_ids) {
                return StructuralHistoryStep::SceneChanged;
            }
            scene_layer_ids(restore_scene)
        };
        let restored = match self.history.borrow_mut().navigate_edit(
            self.history_scope,
            direction,
            step.command_id,
            HistorySnapshot {
                value: MeshEditUndoSnapshot::Scene {
                    scene: current.clone(),
                    guard_ids: reverse_guard,
                },
                bytes: scene_snapshot_bytes(current),
            },
        ) {
            Some(MeshEditUndoSnapshot::Scene { scene, .. }) => scene,
            Some(MeshEditUndoSnapshot::Layer(_)) | None => {
                return StructuralHistoryStep::NotAvailable;
            }
        };
        self.state = EditModeState::ActiveDirty {
            layer: LayerKey::from_scene_mesh_id(layer_id),
        };
        StructuralHistoryStep::Restored(restored)
    }
}

pub(super) fn scene_mesh_snapshot_bytes(layer: &SceneMesh) -> usize {
    scene_mesh_snapshot_bytes_from_mesh(&layer.mesh)
}

fn scene_mesh_snapshot_bytes_from_mesh(mesh: &Mesh) -> usize {
    let texture_bytes = mesh.texture().map_or(0, |texture| texture.rgba.len());
    size_of::<SceneMesh>()
        .saturating_add(size_of_val(mesh.vertices()))
        .saturating_add(size_of_val(mesh.indices()))
        .saturating_add(texture_bytes)
}

pub(super) fn scene_snapshot_bytes(scene: &Scene) -> usize {
    scene
        .meshes()
        .iter()
        .fold(size_of::<Scene>(), |bytes, layer| {
            bytes.saturating_add(scene_mesh_snapshot_bytes(layer))
        })
}

/// Edit Cancel clones `SceneMesh` entries and shares their immutable `Arc<Mesh>`
/// values, so its checkpoint accounts for structural metadata, not vertices.
pub(super) fn scene_checkpoint_bytes(scene: &Scene) -> usize {
    size_of::<Scene>() + scene.meshes().len().saturating_mul(size_of::<SceneMesh>())
}
