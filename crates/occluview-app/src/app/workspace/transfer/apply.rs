//! Applying one transfer transaction to both documents.

use std::collections::HashMap;
use std::mem::size_of;

use super::record::{DocumentDraft, MoveDirection};
use crate::app::workspace::history::{TransferRecord, TransferredLayer};
use crate::app::workspace::state::{SceneSession, WorkspaceState};
use crate::app::OccluViewApp;
use crate::viewer::home_camera_for_scene;
use occluview_core::{Scene, SceneMesh, SceneMeshId};

impl OccluViewApp {
    pub(super) fn apply_transfer_transaction(
        &mut self,
        record: &TransferRecord,
        direction: MoveDirection,
    ) -> Result<(), String> {
        let source_index = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == record.source)
            .ok_or_else(|| "The source scene is no longer open.".to_owned())?;
        let destination_index = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == record.destination)
            .ok_or_else(|| "The destination scene is no longer open.".to_owned())?;
        if source_index == destination_index {
            return Err("A scene cannot receive its own layer.".to_owned());
        }

        let (source_session, destination_session) =
            two_sessions_mut(&mut self.workspace, source_index, destination_index);
        let mut source = DocumentDraft::from_document(&source_session.document);
        let mut destination = DocumentDraft::from_document(&destination_session.document);

        match direction {
            MoveDirection::InitialForward | MoveDirection::Redo => {
                move_forward(&mut source, &mut destination, record)?;
            }
            MoveDirection::Undo => {
                move_backward(&mut source, &mut destination, record)?;
            }
        }

        if direction == MoveDirection::Undo {
            source.focused_layer_id =
                available_focus(record.source_focused_layer_before, &source.scene);
            destination.focused_layer_id =
                available_focus(record.destination_focused_layer_before, &destination.scene);
            if record.destination_was_uninitialized
                && record.auto_created_destination.is_none()
                && destination.scene.meshes().is_empty()
            {
                destination.was_uninitialized = true;
            }
        } else {
            if source
                .focused_layer_id
                .is_some_and(|id| record.layers.iter().any(|item| item.layer_id == id))
            {
                source.focused_layer_id = source.scene.meshes().first().map(SceneMesh::id);
            }
            if destination.focused_layer_id.is_none() {
                destination.focused_layer_id = record.layers.first().map(|item| item.layer_id);
            }
            destination.was_uninitialized = false;
        }

        source.install(&mut source_session.document, false);
        let restore_destination_uninitialized =
            destination.was_uninitialized && record.auto_created_destination.is_none();
        destination.install(
            &mut destination_session.document,
            restore_destination_uninitialized,
        );

        if let Some(scene) = source_session.document.scene.as_deref() {
            if source_session.render.camera.is_none() && !scene.meshes().is_empty() {
                source_session.render.camera = Some(home_camera_for_scene(scene));
            }
        }
        if let Some(scene) = destination_session.document.scene.as_deref() {
            if destination_session.render.camera.is_none() && !scene.meshes().is_empty() {
                destination_session.render.camera = Some(home_camera_for_scene(scene));
            }
        }
        Ok(())
    }
}

fn two_sessions_mut(
    workspace: &mut WorkspaceState,
    source_index: usize,
    destination_index: usize,
) -> (&mut SceneSession, &mut SceneSession) {
    if source_index < destination_index {
        let (left, right) = workspace.scenes.split_at_mut(destination_index);
        (&mut left[source_index], &mut right[0])
    } else {
        let (left, right) = workspace.scenes.split_at_mut(source_index);
        (&mut right[0], &mut left[destination_index])
    }
}
fn move_forward(
    source: &mut DocumentDraft,
    destination: &mut DocumentDraft,
    record: &TransferRecord,
) -> Result<(), String> {
    // Move the live layer state so Save and material changes made after the
    // transfer remain intact when the user replays it.
    let mut positions = record
        .layers
        .iter()
        .map(|item| {
            source
                .scene
                .meshes()
                .iter()
                .position(|layer| layer.id() == item.layer_id)
                .map(|index| (index, item.layer_id))
                .ok_or_else(|| "A layer changed scenes before transfer history applied.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if record.layers.iter().any(|item| {
        destination
            .scene
            .meshes()
            .iter()
            .any(|layer| layer.id() == item.layer_id)
    }) {
        return Err("A transferred layer already exists in the destination.".to_owned());
    }
    positions.sort_by_key(|(index, _)| std::cmp::Reverse(*index));

    let mut moved = HashMap::with_capacity(positions.len());
    for (index, id) in positions {
        let layer = source
            .scene
            .remove(index)
            .ok_or_else(|| "The source layer disappeared during transfer.".to_owned())?;
        let current_path = source.paths.remove(index);
        let source_hidden_position = source
            .hidden_layer_stack
            .iter()
            .position(|entry| *entry == id);
        source.hidden_layer_stack.retain(|entry| *entry != id);
        let was_dirty = source.unsaved_edit_layer_ids.remove(&id);
        let translucent_restore = source.translucent_layer_restore.remove(&id);
        moved.insert(
            id,
            (
                layer,
                current_path,
                was_dirty,
                source_hidden_position,
                translucent_restore,
            ),
        );
    }

    for item in &record.layers {
        let Some((layer, path, was_dirty, source_hidden, translucent)) =
            moved.remove(&item.layer_id)
        else {
            return Err("The transfer payload lost a selected layer.".to_owned());
        };
        let destination_index = item.destination_index.min(destination.scene.meshes().len());
        destination.scene.insert(destination_index, layer.clone());
        destination.paths.insert(destination_index, path);
        if was_dirty {
            destination.unsaved_edit_layer_ids.insert(layer.id());
        }
        if source_hidden.is_some() {
            let index = item
                .destination_hidden_stack_position
                .unwrap_or(destination.hidden_layer_stack.len())
                .min(destination.hidden_layer_stack.len());
            destination.hidden_layer_stack.insert(index, layer.id());
        }
        if let Some(value) = translucent {
            destination
                .translucent_layer_restore
                .insert(layer.id(), value);
        }
    }
    Ok(())
}
fn move_backward(
    source: &mut DocumentDraft,
    destination: &mut DocumentDraft,
    record: &TransferRecord,
) -> Result<(), String> {
    // Consume the live destination entries and return their current state to
    // the source; the layer may have been saved or adjusted since transfer.
    let mut positions = record
        .layers
        .iter()
        .map(|item| {
            destination
                .scene
                .meshes()
                .iter()
                .position(|layer| layer.id() == item.layer_id)
                .map(|index| (index, item.layer_id))
                .ok_or_else(|| "A transferred layer is no longer in the destination.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if record.layers.iter().any(|item| {
        source
            .scene
            .meshes()
            .iter()
            .any(|layer| layer.id() == item.layer_id)
    }) {
        return Err("A transferred layer already exists in the source.".to_owned());
    }
    positions.sort_by_key(|(index, _)| std::cmp::Reverse(*index));

    let mut moved = HashMap::with_capacity(positions.len());
    for (index, id) in positions {
        let layer = destination
            .scene
            .remove(index)
            .ok_or_else(|| "The destination layer disappeared during undo.".to_owned())?;
        let current_path = destination.paths.remove(index);
        let destination_hidden_position = destination
            .hidden_layer_stack
            .iter()
            .position(|entry| *entry == id);
        destination.hidden_layer_stack.retain(|entry| *entry != id);
        let was_dirty = destination.unsaved_edit_layer_ids.remove(&id);
        let translucent_restore = destination.translucent_layer_restore.remove(&id);
        moved.insert(
            id,
            (
                layer,
                current_path,
                was_dirty,
                destination_hidden_position,
                translucent_restore,
            ),
        );
    }

    let mut ordered = record.layers.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|item| item.source_index);
    for item in ordered {
        let Some((layer, path, was_dirty, destination_hidden, translucent)) =
            moved.remove(&item.layer_id)
        else {
            return Err("The transfer payload lost a selected layer.".to_owned());
        };
        let source_index = item.source_index.min(source.scene.meshes().len());
        source.scene.insert(source_index, layer.clone());
        source.paths.insert(source_index, path);
        if was_dirty {
            source.unsaved_edit_layer_ids.insert(layer.id());
        }
        if destination_hidden.is_some() {
            let source_hidden = item
                .source_hidden_stack_position
                .unwrap_or(source.hidden_layer_stack.len())
                .min(source.hidden_layer_stack.len());
            source.hidden_layer_stack.insert(source_hidden, layer.id());
        }
        if let Some(value) = translucent {
            source.translucent_layer_restore.insert(layer.id(), value);
        }
    }
    Ok(())
}
fn available_focus(requested: Option<SceneMeshId>, scene: &Scene) -> Option<SceneMeshId> {
    requested
        .filter(|id| scene.meshes().iter().any(|layer| layer.id() == *id))
        .or_else(|| scene.meshes().first().map(SceneMesh::id))
}
pub(super) fn opposite(direction: MoveDirection) -> MoveDirection {
    match direction {
        MoveDirection::InitialForward | MoveDirection::Redo => MoveDirection::Undo,
        MoveDirection::Undo => MoveDirection::Redo,
    }
}
pub(super) fn transfer_payload_bytes(record: &TransferRecord) -> usize {
    let layers = record.layers.iter().fold(0usize, |bytes, item| {
        bytes
            .saturating_add(size_of::<TransferredLayer>())
            .saturating_add(
                item.source_path
                    .as_ref()
                    .map_or(0, |path| path.as_os_str().len()),
            )
    });
    size_of::<TransferRecord>()
        .saturating_add(layers)
        .saturating_add(
            record
                .auto_created_destination
                .as_ref()
                .map_or(0, |destination| destination.name.len()),
        )
}
