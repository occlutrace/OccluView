//! The record a two-scene transfer commits, and the drafts it is built from.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::state_document::DocumentState;
use crate::app::workspace::commands::LayerIds;
use crate::app::workspace::history::{AutoCreatedDestination, TransferRecord, TransferredLayer};
use crate::app::workspace::id::SceneKey;
use crate::app::OccluViewApp;
use occluview_core::{Scene, SceneMeshId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveDirection {
    InitialForward,
    Undo,
    Redo,
}

/// A private draft for one document in a two-document transaction.
///
/// Both drafts are completed before either live document is replaced. The
/// scene clone copies only the small layer records: each `SceneMesh` continues
/// to refer to the same `Arc<Mesh>` geometry.
pub(super) struct DocumentDraft {
    pub(super) scene: Scene,
    pub(super) was_uninitialized: bool,
    pub(super) paths: Vec<PathBuf>,
    pub(super) focused_layer_id: Option<SceneMeshId>,
    pub(super) unsaved_edit_layer_ids: BTreeSet<SceneMeshId>,
    pub(super) hidden_layer_stack: Vec<SceneMeshId>,
    pub(super) translucent_layer_restore: HashMap<SceneMeshId, f32>,
}

pub(super) struct TransferIntent<'a> {
    pub(super) source: SceneKey,
    pub(super) destination: SceneKey,
    pub(super) layers: &'a LayerIds,
    pub(super) active_before: SceneKey,
    pub(super) auto_created_destination: Option<AutoCreatedDestination>,
}

pub(super) struct HistoryNavigationRollback<'a> {
    pub(super) history_id: u64,
    pub(super) before: &'a TransferRecord,
    pub(super) working: &'a TransferRecord,
    pub(super) applied: MoveDirection,
    pub(super) rekeyed: bool,
}

impl DocumentDraft {
    pub(super) fn from_document(document: &DocumentState) -> Self {
        let was_uninitialized = document.scene.is_none();
        let scene = document
            .scene
            .as_deref()
            .cloned()
            .unwrap_or_else(Scene::new);
        let paths = crate::app::scene::commit::reconcile_scene_paths(
            &scene,
            &document.current_paths,
            &scene,
        );
        Self {
            scene,
            was_uninitialized,
            paths,
            focused_layer_id: document.focused_layer_id,
            unsaved_edit_layer_ids: document.unsaved_edit_layer_ids.clone(),
            hidden_layer_stack: document.hidden_layer_stack.clone(),
            translucent_layer_restore: document.translucent_layer_restore.clone(),
        }
    }

    pub(super) fn install(self, document: &mut DocumentState, restore_uninitialized: bool) {
        let keep_none = restore_uninitialized && self.scene.meshes().is_empty();
        document.scene = if keep_none {
            None
        } else {
            Some(Arc::new(self.scene))
        };
        document.current_paths = self.paths;
        document.focused_layer_id = self.focused_layer_id;
        document.unsaved_edit_layer_ids = self.unsaved_edit_layer_ids;
        document.hidden_layer_stack = self.hidden_layer_stack;
        document.translucent_layer_restore = self.translucent_layer_restore;
        document.content_revision = document.content_revision.wrapping_add(1);
        document.mesh_selection_drag = None;
        if let Some(scene) = document.scene.as_deref() {
            document.edit_mode.sync_after_transfer(scene);
        }
    }
}

impl OccluViewApp {
    pub(super) fn make_transfer_record(
        &self,
        intent: TransferIntent<'_>,
    ) -> Result<TransferRecord, String> {
        let TransferIntent {
            source,
            destination,
            layers: requested,
            active_before,
            auto_created_destination,
        } = intent;
        let source_index = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == source)
            .ok_or_else(|| "The source scene is no longer open.".to_owned())?;
        let destination_index = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == destination)
            .ok_or_else(|| "The destination scene is no longer open.".to_owned())?;
        let from = &self.workspace.scenes[source_index];
        let to = &self.workspace.scenes[destination_index];
        let source_scene = from
            .document
            .scene
            .as_deref()
            .ok_or_else(|| "The source scene has no layers to move.".to_owned())?;
        let source_paths = crate::app::scene::commit::reconcile_scene_paths(
            source_scene,
            &from.document.current_paths,
            source_scene,
        );
        let destination_scene = to.document.scene.as_deref();
        let destination_count = destination_scene.map_or(0, |scene| scene.meshes().len());
        let requested_ids: BTreeSet<_> = requested.as_slice().iter().copied().collect();
        if requested_ids.len() != requested.as_slice().len() {
            return Err("A layer was selected more than once.".to_owned());
        }
        let mut source_rows = source_scene
            .meshes()
            .iter()
            .enumerate()
            .filter(|(_, layer)| requested_ids.contains(&layer.id()))
            .map(|(index, layer)| (index, layer.clone()))
            .collect::<Vec<_>>();
        if source_rows.len() != requested_ids.len() {
            return Err("One of the selected layers is no longer in the source scene.".to_owned());
        }
        source_rows.sort_by_key(|(index, _)| *index);

        let mut destination_hidden_index = to.document.hidden_layer_stack.len();
        let mut transfer_layers = Vec::with_capacity(source_rows.len());
        for (ordinal, (source_layer_index, layer)) in source_rows.into_iter().enumerate() {
            if destination_scene.is_some_and(|scene| {
                scene
                    .meshes()
                    .iter()
                    .any(|existing| existing.id() == layer.id())
            }) {
                return Err(
                    "The destination already contains one of the selected layers.".to_owned(),
                );
            }
            let source_hidden_stack_position = from
                .document
                .hidden_layer_stack
                .iter()
                .position(|hidden| *hidden == layer.id());
            let destination_hidden_stack_position = source_hidden_stack_position.map(|_| {
                let position = destination_hidden_index;
                destination_hidden_index = destination_hidden_index.saturating_add(1);
                position
            });
            let source_path = source_paths
                .get(source_layer_index)
                .filter(|path| !path.as_os_str().is_empty())
                .cloned();
            transfer_layers.push(TransferredLayer {
                layer_id: layer.id(),
                source_index: source_layer_index,
                destination_index: destination_count + ordinal,
                source_path,
                source_hidden_stack_position,
                destination_hidden_stack_position,
            });
        }

        Ok(TransferRecord {
            source,
            destination,
            layers: transfer_layers,
            auto_created_destination,
            destination_was_uninitialized: to.document.scene.is_none(),
            source_focused_layer_before: from.document.focused_layer_id,
            destination_focused_layer_before: to.document.focused_layer_id,
            active_scene_before: active_before,
            active_scene_after: destination,
        })
    }
}
