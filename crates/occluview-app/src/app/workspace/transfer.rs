//! Atomic layer moves and their linked workspace history navigation.
//!
//! The payload keeps `SceneMesh` values, which retain the original `Arc<Mesh>`.
//! A transfer therefore moves layer ownership and metadata without copying the
//! scan's vertices or texture buffers.

use super::commands::{LayerIds, SplitSide, TransferDestination};
use super::history::{
    AutoCreatedDestination, HistoryDirection, HistoryError, HistoryStepKind, NavigationError,
    TransferRecord, TransferredLayer,
};
use super::id::SceneKey;
use super::layout::WorkspaceLayout;
use super::state::{SceneSession, WorkspaceState};
use crate::app::state_document::DocumentState;
use crate::app::OccluViewApp;
use crate::viewer::home_camera_for_scene;
use occluview_core::{Scene, SceneMesh, SceneMeshId};
use std::collections::{BTreeSet, HashMap};
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveDirection {
    InitialForward,
    Undo,
    Redo,
}

/// A private draft for one document in a two-document transaction.
///
/// Both drafts are completed before either live document is replaced. The
/// scene clone copies only the small layer records: each `SceneMesh` continues
/// to refer to the same `Arc<Mesh>` geometry.
struct DocumentDraft {
    scene: Scene,
    was_uninitialized: bool,
    paths: Vec<PathBuf>,
    focused_layer_id: Option<SceneMeshId>,
    unsaved_edit_layer_ids: BTreeSet<SceneMeshId>,
    hidden_layer_stack: Vec<SceneMeshId>,
    translucent_layer_restore: HashMap<SceneMeshId, f32>,
}

struct TransferIntent<'a> {
    source: SceneKey,
    destination: SceneKey,
    layers: &'a LayerIds,
    active_before: SceneKey,
    auto_created_destination: Option<AutoCreatedDestination>,
}

struct HistoryNavigationRollback<'a> {
    history_id: u64,
    before: &'a TransferRecord,
    working: &'a TransferRecord,
    applied: MoveDirection,
    rekeyed: bool,
}

impl DocumentDraft {
    fn from_document(document: &DocumentState) -> Self {
        let was_uninitialized = document.scene.is_none();
        let scene = document
            .scene
            .as_deref()
            .cloned()
            .unwrap_or_else(Scene::new);
        let paths = crate::app::app_scene_commit::reconcile_scene_paths(
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

    fn install(self, document: &mut DocumentState, restore_uninitialized: bool) {
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
    /// Move one or more layers to a live scene, or atomically create a peer
    /// scene and move them there. The only fallible history operation is
    /// reserved before either document changes.
    #[allow(clippy::too_many_lines)]
    pub(in crate::app) fn transfer_layers(
        &mut self,
        source: SceneKey,
        destination: TransferDestination,
        layers: LayerIds,
    ) -> Result<(), String> {
        if self.ui.command_dialog_open() {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-close-dialog")));
        }
        if self.workspace.input.capture().is_some() {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-gesture")));
        }

        let active_before = self.workspace.input.active().scene;
        let source_index = self
            .workspace
            .scenes
            .iter()
            .position(|scene| scene.key == source)
            .ok_or_else(|| {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target"))
            })?;
        if self.loader.has_work_for(source) {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-load")));
        }
        self.ensure_transfer_ready(source)?;
        if !self
            .workspace
            .scenes
            .iter()
            .any(|scene| scene.key == active_before)
        {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-stale-target")));
        }

        let (destination_key, auto_created) = match destination {
            TransferDestination::Existing(key) => {
                if key == source {
                    return Err("Choose another scene for the layer.".to_owned());
                }
                if !self.workspace.scenes.iter().any(|scene| scene.key == key) {
                    return Err(self
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-stale-target")));
                }
                (key, None)
            }
            TransferDestination::CreateBeside {
                scene: destination,
                pane,
                side,
            } => {
                if self.workspace.scenes.len() >= 2 {
                    return Err(self
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-two-scene-limit")));
                }
                if self.workspace.scenes.iter().any(|scene| scene.pane == pane) {
                    return Err("That viewport is already in use.".to_owned());
                }
                if self
                    .workspace
                    .scenes
                    .iter()
                    .any(|scene| scene.key.id == destination.id)
                {
                    return Err("That scene identity is already in use.".to_owned());
                }
                let new_key = destination;
                let name = self.ui.locale.tr_with(
                    crate::i18n::message_id!("workspace-scene-name"),
                    &[("number", &new_key.id.get().to_string())],
                );
                let session = self.make_scene_session(new_key, pane, name.clone())?;
                self.workspace
                    .scenes
                    .try_reserve(1)
                    .map_err(|error| format!("Could not create a scene: {error}"))?;
                self.workspace.scenes.push(session);
                let anchor_pane = self.workspace.scenes[source_index].pane;
                let (left, right) = match side {
                    SplitSide::Left => (pane, anchor_pane),
                    SplitSide::Right => (anchor_pane, pane),
                };
                let layout_with_destination = WorkspaceLayout::SideBySide {
                    left,
                    right,
                    ratio: 0.5,
                };
                (
                    new_key,
                    Some(AutoCreatedDestination {
                        pane_id: pane,
                        name,
                        previous_layout: self.workspace.layout,
                        previous_saved_split: self.workspace.saved_split,
                        layout_with_destination,
                        preserve_on_undo: false,
                        retained_after_undo: false,
                    }),
                )
            }
        };

        if self.loader.has_work_for(destination_key) {
            self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-load")));
        }

        if !self
            .workspace
            .scenes
            .iter()
            .any(|scene| scene.key == destination_key)
        {
            self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-stale-target")));
        }

        if let Err(error) = self.ensure_transfer_ready(destination_key) {
            self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
            return Err(error);
        }

        let record = match self.make_transfer_record(TransferIntent {
            source,
            destination: destination_key,
            layers: &layers,
            active_before,
            auto_created_destination: auto_created.clone(),
        }) {
            Ok(record) => record,
            Err(error) => {
                self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
                return Err(error);
            }
        };
        let bytes = transfer_payload_bytes(&record);
        let prepare_result = {
            self.workspace
                .history
                .borrow_mut()
                .prepare_transfer(record.clone(), bytes)
        };
        let history_id = match prepare_result {
            Ok(id) => id,
            Err(error) => {
                self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
                return Err(history_error_text(error, &self.ui.locale));
            }
        };

        let result = self.apply_transfer_transaction(&record, MoveDirection::InitialForward);
        if let Err(error) = result {
            self.workspace
                .history
                .borrow_mut()
                .discard_pending(history_id);
            self.remove_uncommitted_destination(auto_created.as_ref(), destination_key);
            return Err(error);
        }

        if let Some(auto) = record.auto_created_destination.as_ref() {
            self.workspace.layout = auto.layout_with_destination;
            self.workspace.saved_split = Some(auto.layout_with_destination);
            self.workspace.scene_tab_rects.clear();
        }

        let commit_result = self
            .workspace
            .history
            .borrow_mut()
            .commit_transfer(history_id);
        if let Err(error) = commit_result {
            // The pending reservation made this commit infallible on the UI
            // thread after preflight. Keep a defensive reverse path in case
            // journal accounting changes later.
            let rollback = self.apply_transfer_transaction(&record, MoveDirection::Undo);
            self.workspace
                .history
                .borrow_mut()
                .discard_pending(history_id);
            if let Some(auto) = record.auto_created_destination.as_ref() {
                self.remove_uncommitted_destination(Some(auto), record.destination);
            }
            let _ = self.activate_scene(record.active_scene_before);
            return Err(format!(
                "Transfer history could not be committed ({error:?}); rollback: {}",
                rollback.err().unwrap_or_else(|| "complete".to_owned())
            ));
        }

        // The destination was validated live before the content commit and
        // there is no captured gesture, so activation is guaranteed here.
        self.activate_scene(destination_key)?;
        self.finish_transfer_scene_change(source, destination_key, source, layers.as_slice());
        self.workspace_status(
            destination_key,
            self.ui.locale.tr_with(
                crate::i18n::message_id!("workspace-layer-move-scene"),
                &[(
                    "scene",
                    self.scene_name(destination_key)
                        .as_deref()
                        .unwrap_or("Scene"),
                )],
            ),
        );
        Ok(())
    }

    /// Undo or redo the active scene's workspace journal head. Local mesh
    /// commands stay in their original `SceneContext`; a transfer is applied to
    /// both participant documents as one transaction.
    pub(in crate::app) fn navigate_workspace_history(
        &mut self,
        scene_key: SceneKey,
        direction: HistoryDirection,
    ) -> Result<(), String> {
        if self.ui.command_dialog_open() {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-close-dialog")));
        }
        let step = self
            .workspace
            .history
            .borrow()
            .top_step(Some(scene_key), direction);
        let Some(step) = step else {
            let message = self.ui.locale.tr(match direction {
                HistoryDirection::Undo => crate::i18n::message_id!("undo-nothing"),
                HistoryDirection::Redo => crate::i18n::message_id!("redo-nothing"),
            });
            self.workspace_status(scene_key, message);
            return Ok(());
        };

        if step.kind != HistoryStepKind::Transfer {
            if self
                .workspace
                .scenes
                .iter()
                .all(|scene| scene.key != scene_key)
            {
                return Err(self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target")));
            }
            let ctx = self.ui.repaint_ctx.clone();
            let Some(mut scene) = self.scene_context(scene_key) else {
                return Err(self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target")));
            };
            if scene.tools.sculpt.is_busy()
                || scene.document.edit_mode.is_busy()
                || scene.document.edit_mode.has_active_session()
                || scene.tools.align.drag.is_some()
                || scene.document.unsaved_drag_pose
            {
                return Err(self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-wait-edit")));
            }
            scene.apply_history_navigation_now(direction == HistoryDirection::Redo, &ctx);
            return Ok(());
        }

        self.navigate_transfer_history(step.command_id, direction)
    }

    #[allow(clippy::too_many_lines)]
    fn navigate_transfer_history(
        &mut self,
        history_id: u64,
        direction: HistoryDirection,
    ) -> Result<(), String> {
        let record = self
            .workspace
            .history
            .borrow()
            .transfer_payload(history_id)
            .cloned()
            .ok_or_else(|| {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target"))
            })?;
        if self.workspace.input.capture().is_some() {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-gesture")));
        }
        if self.loader.has_work_for(record.source) || self.loader.has_work_for(record.destination) {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-load")));
        }
        self.ensure_transfer_ready(record.source)?;

        self.workspace
            .history
            .borrow()
            .validate_navigation(history_id, direction)
            .map_err(|error| navigation_error_text(error, &self.ui.locale))?;

        let auto = record.auto_created_destination.as_ref();
        let has_other_destination_history = self
            .workspace
            .history
            .borrow()
            .has_other_commands_for_scene(record.destination, history_id);
        let should_retain_destination = if direction == HistoryDirection::Undo {
            auto.is_some_and(|created| {
                has_other_destination_history
                    || self
                        .workspace
                        .scenes
                        .iter()
                        .find(|scene| scene.key == record.destination)
                        .is_some_and(|scene| {
                            auto_created_destination_was_touched(scene, created, &record)
                        })
            })
        } else {
            true
        };

        let recreate_destination = direction == HistoryDirection::Redo
            && auto.is_some_and(|created| !created.retained_after_undo)
            && self
                .workspace
                .scenes
                .iter()
                .all(|scene| scene.key != record.destination);

        if recreate_destination && self.workspace.scenes.len() >= 2 {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-two-scene-limit")));
        }

        if !recreate_destination {
            self.ensure_transfer_ready(record.destination)?;
        }

        let mut staged = if recreate_destination {
            let auto =
                auto.ok_or_else(|| "The created destination metadata is unavailable.".to_owned())?;
            let new_key = self
                .workspace
                .ids
                .allocate_scene_key(record.destination.id)
                .map_err(|error| error.to_string())?;
            let scene = self.make_scene_session(new_key, auto.pane_id, auto.name.clone())?;
            self.workspace
                .history
                .borrow()
                .validate_transfer_scene_key_replacement(history_id, record.destination, new_key)
                .map_err(|error| navigation_error_text(error, &self.ui.locale))?;
            Some(scene)
        } else {
            None
        };

        if direction == HistoryDirection::Redo && !recreate_destination {
            self.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == record.destination)
                .ok_or_else(|| {
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("workspace-stale-target"))
                })?;
        }

        let mut working_record = record.clone();
        if let Some(scene) = staged.as_ref() {
            working_record.destination = scene.key;
            working_record.active_scene_after = scene.key;
        }

        if let Some(scene) = staged.take() {
            self.workspace
                .scenes
                .try_reserve(1)
                .map_err(|error| format!("Could not restore the destination scene: {error}"))?;
            self.workspace.scenes.push(scene);
        }

        let move_direction = match direction {
            HistoryDirection::Undo => MoveDirection::Undo,
            HistoryDirection::Redo => MoveDirection::Redo,
        };
        if let Err(error) = self.apply_transfer_transaction(&working_record, move_direction) {
            if recreate_destination {
                self.workspace
                    .scenes
                    .retain(|scene| scene.key != working_record.destination);
            }
            return Err(error);
        }

        let mut updated_record = working_record.clone();
        if let Some(created) = updated_record.auto_created_destination.as_mut() {
            if direction == HistoryDirection::Undo {
                created.preserve_on_undo = should_retain_destination;
                created.retained_after_undo = should_retain_destination;
            } else {
                created.retained_after_undo = false;
            }
        }

        if recreate_destination {
            let rekey_result = {
                self.workspace
                    .history
                    .borrow_mut()
                    .replace_transfer_scene_key(
                        history_id,
                        record.destination,
                        working_record.destination,
                    )
            };
            if let Err(error) = rekey_result {
                let rollback =
                    self.apply_transfer_transaction(&working_record, opposite(move_direction));
                self.workspace
                    .scenes
                    .retain(|scene| scene.key != working_record.destination);
                return Err(format!(
                    "{}; rollback: {}",
                    navigation_error_text(error, &self.ui.locale),
                    rollback.err().unwrap_or_else(|| "complete".to_owned())
                ));
            }
        }
        let history_updated = self
            .workspace
            .history
            .borrow_mut()
            .update_transfer_record(history_id, |stored| *stored = updated_record.clone());
        if !history_updated {
            self.rollback_history_navigation(HistoryNavigationRollback {
                history_id,
                before: &record,
                working: &working_record,
                applied: move_direction,
                rekeyed: recreate_destination,
            });
            return Err("The transfer history entry changed during navigation.".to_owned());
        }
        let navigation_commit = self
            .workspace
            .history
            .borrow_mut()
            .commit_navigation(history_id, direction);
        if let Err(error) = navigation_commit {
            self.rollback_history_navigation(HistoryNavigationRollback {
                history_id,
                before: &record,
                working: &working_record,
                applied: move_direction,
                rekeyed: recreate_destination,
            });
            return Err(navigation_error_text(error, &self.ui.locale));
        }

        if direction == HistoryDirection::Undo {
            if let Some(created) = auto {
                if !should_retain_destination {
                    self.retain_scene_frame_until_next_pass(record.destination);
                    self.workspace
                        .scenes
                        .retain(|scene| scene.key != record.destination);
                    self.workspace.layout = created.previous_layout;
                    self.workspace.saved_split = created.previous_saved_split;
                    self.workspace.scene_tab_rects.clear();
                }
            }
            self.activate_scene(record.active_scene_before)?;
        } else {
            if recreate_destination {
                if let Some(created) = auto {
                    self.workspace.layout = created.layout_with_destination;
                    self.workspace.saved_split = Some(created.layout_with_destination);
                }
            }
            self.activate_scene(working_record.active_scene_after)?;
        }

        self.finish_transfer_scene_change(
            record.source,
            working_record.destination,
            match direction {
                HistoryDirection::Undo => working_record.destination,
                HistoryDirection::Redo => working_record.source,
            },
            &record
                .layers
                .iter()
                .map(|layer| layer.layer_id)
                .collect::<Vec<_>>(),
        );
        Ok(())
    }

    fn rollback_history_navigation(&mut self, rollback: HistoryNavigationRollback<'_>) {
        let _ = self.apply_transfer_transaction(rollback.working, opposite(rollback.applied));
        if rollback.rekeyed {
            let _ = self
                .workspace
                .history
                .borrow_mut()
                .replace_transfer_scene_key(
                    rollback.history_id,
                    rollback.working.destination,
                    rollback.before.destination,
                );
            self.workspace
                .scenes
                .retain(|scene| scene.key != rollback.working.destination);
        }
        self.workspace
            .history
            .borrow_mut()
            .update_transfer_record(rollback.history_id, |stored| {
                *stored = rollback.before.clone();
            });
    }

    fn ensure_transfer_ready(&self, key: SceneKey) -> Result<(), String> {
        let scene = self
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .ok_or_else(|| {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target"))
            })?;
        if scene.document.edit_mode.has_active_session()
            || scene.document.edit_mode.is_busy()
            || self
                .workspace
                .history
                .borrow()
                .has_active_edit_checkpoint(key)
        {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-edit")));
        }
        if scene.document.unsaved_drag_pose
            || scene.tools.align.drag.is_some()
            || scene
                .tools
                .align
                .worker
                .as_ref()
                .is_some_and(crate::align::align_worker::AlignWorker::is_busy)
            || scene.document.unsaved_sculpt_stroke
            || scene.tools.sculpt.is_busy()
            || scene.tools.bridge_split_active()
        {
            return Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-edit")));
        }
        Ok(())
    }

    fn make_transfer_record(&self, intent: TransferIntent<'_>) -> Result<TransferRecord, String> {
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
        let source_paths = crate::app::app_scene_commit::reconcile_scene_paths(
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

    fn apply_transfer_transaction(
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

    fn finish_transfer_scene_change(
        &mut self,
        source: SceneKey,
        destination: SceneKey,
        removed_from: SceneKey,
        layer_ids: &[SceneMeshId],
    ) {
        let ctx = self.ui.repaint_ctx.clone();
        for key in [source, destination] {
            self.retain_scene_frame_until_next_pass(key);
        }
        for key in [source, destination] {
            if let Some(mut scene) = self.scene_context(key) {
                scene.render.prepared_scene = None;
                scene.render.prepared_selection_overlay = None;
                scene.render.rendered = None;
                scene.render.section_cache = occluview_edit::scene::SectionCache::new();
                scene.render.invalidation.scene_geometry_changed();
                scene.document.mesh_selection_drag = None;
                let contact_pair = scene.tools.contacts.pair();
                if contact_pair.is_some_and(|pair| {
                    layer_ids.contains(&pair.subject) || layer_ids.contains(&pair.antagonist)
                }) {
                    scene.close_contacts(&ctx);
                }
                let align_role_is_moved = layer_ids.iter().any(|id| {
                    scene.tools.align.tool.moving_layer() == Some(*id)
                        || scene.tools.align.tool.fixed_layer() == Some(*id)
                });
                for id in layer_ids {
                    scene.tools.align.tool.forget_layer(*id);
                }
                if align_role_is_moved {
                    let reason = scene
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("align-status-scan-changed"));
                    scene.forget_align_fit(&reason);
                    scene.tools.align.markings.clear();
                    scene.tools.align.brush.reset_target();
                }
                if scene
                    .tools
                    .sculpt
                    .worker
                    .as_ref()
                    .is_some_and(|worker| layer_ids.contains(&worker.layer_id))
                {
                    scene.tools.sculpt.invalidate_session();
                }
                if key == removed_from && scene.tools.cut_view.is_probe_linked() {
                    scene.tools.cut_view.disable();
                } else if scene.can_render_cut_view() {
                    scene.tools.cut_view.mark_dirty();
                } else if key == removed_from {
                    scene.tools.cut_view.disable();
                }
                if key != removed_from {
                    if let Some(live) = scene.document.live_scene_mut() {
                        for entry in live.meshes_mut() {
                            if layer_ids.contains(&entry.id()) {
                                entry.clear_overlay();
                            }
                        }
                    }
                }
                if let Some(scene_data) = scene.document.scene.as_deref() {
                    scene.document.edit_mode.sync_after_transfer(scene_data);
                }
                let has_measurable_layer = scene.document.scene.as_deref().is_some_and(|scene| {
                    scene
                        .meshes()
                        .iter()
                        .any(|layer| layer.visible && !layer.mesh.is_point_cloud())
                });
                if has_measurable_layer {
                    scene.enrol_align_arrivals();
                }
                if key == removed_from {
                    scene.tools.measure.clear_measurements();
                }
                ctx.request_repaint();
            }
        }
    }

    fn activate_scene(&mut self, key: SceneKey) -> Result<(), String> {
        let target = self
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .map(SceneSession::target)
            .ok_or_else(|| {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-stale-target"))
            })?;
        match self.workspace.input.request_activation(target) {
            super::input::ActivationResult::DeferredUntilGestureEnds
            | super::input::ActivationResult::IgnoredWhileGestureCaptured => Err(self
                .ui
                .locale
                .tr(crate::i18n::message_id!("workspace-wait-gesture"))),
            super::input::ActivationResult::Activated
            | super::input::ActivationResult::Unchanged => Ok(()),
        }
    }

    fn scene_name(&self, key: SceneKey) -> Option<String> {
        self.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .map(|scene| scene.name.clone())
    }

    fn remove_uncommitted_destination(
        &mut self,
        auto: Option<&AutoCreatedDestination>,
        destination: SceneKey,
    ) {
        let Some(auto) = auto else { return };
        self.workspace
            .scenes
            .retain(|scene| scene.key != destination);
        self.workspace.layout = auto.previous_layout;
        self.workspace.saved_split = auto.previous_saved_split;
        self.workspace.scene_tab_rects.clear();
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

fn auto_created_destination_was_touched(
    scene: &SceneSession,
    created: &AutoCreatedDestination,
    record: &TransferRecord,
) -> bool {
    if scene.preserve_on_transfer_undo || scene.name != created.name {
        return true;
    }
    let moved = record
        .layers
        .iter()
        .map(|item| item.layer_id)
        .collect::<BTreeSet<_>>();
    let mut recorded_paths = record
        .layers
        .iter()
        .map(|item| {
            (
                item.destination_index,
                item.source_path.clone().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    recorded_paths.sort_by_key(|(index, _)| *index);
    let recorded_paths = recorded_paths
        .into_iter()
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    scene.document.scene.as_deref().is_some_and(|live| {
        live.meshes()
            .iter()
            .any(|layer| !moved.contains(&layer.id()))
    }) || scene
        .document
        .unsaved_edit_layer_ids
        .iter()
        .any(|layer_id| !moved.contains(layer_id))
        || scene
            .document
            .hidden_layer_stack
            .iter()
            .any(|layer_id| !moved.contains(layer_id))
        || scene
            .document
            .translucent_layer_restore
            .keys()
            .any(|layer_id| !moved.contains(layer_id))
        || scene.document.current_paths != recorded_paths
}

fn opposite(direction: MoveDirection) -> MoveDirection {
    match direction {
        MoveDirection::InitialForward | MoveDirection::Redo => MoveDirection::Undo,
        MoveDirection::Undo => MoveDirection::Redo,
    }
}

fn transfer_payload_bytes(record: &TransferRecord) -> usize {
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

fn history_error_text(error: HistoryError, locale: &crate::i18n::LocaleManager) -> String {
    match error {
        HistoryError::EditSessionActive(_) => {
            locale.tr(crate::i18n::message_id!("workspace-wait-edit"))
        }
        HistoryError::BudgetExceeded => {
            locale.tr(crate::i18n::message_id!("workspace-history-budget"))
        }
        HistoryError::EmptyTransfer => "Choose at least one layer to move.".to_owned(),
        HistoryError::SameSceneTransfer => "Choose another scene for the layer.".to_owned(),
        HistoryError::UnknownCommand(_) => {
            locale.tr(crate::i18n::message_id!("workspace-stale-target"))
        }
    }
}

fn navigation_error_text(error: NavigationError, locale: &crate::i18n::LocaleManager) -> String {
    match error {
        NavigationError::EditSessionActive(_) => {
            locale.tr(crate::i18n::message_id!("workspace-wait-edit"))
        }
        NavigationError::SceneClosed(_) | NavigationError::UnknownCommand(_) => {
            locale.tr(crate::i18n::message_id!("workspace-stale-target"))
        }
        NavigationError::CommandNotAtHead(_) => {
            locale.tr(crate::i18n::message_id!("workspace-history-order"))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::app::workspace::id::PaneId;
    use crate::app::workspace::state::SceneSession;
    use crate::edit_mode::{BusyFinish, EditModeCommand};
    use eframe::egui;
    use glam::Vec3;
    use occluview_core::{Mesh, SceneMesh};

    fn app_with_layer() -> (OccluViewApp, SceneKey, SceneMeshId, Arc<Mesh>) {
        let mut app = OccluViewApp::new_for_tests(egui::Context::default());
        let source_key = app.workspace.scenes[0].key;
        let mesh = Arc::new(Mesh::empty());
        let layer = SceneMesh::new(Arc::clone(&mesh));
        let layer_id = layer.id();
        let mut scene = Scene::new();
        scene.add(layer);
        app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
        (app, source_key, layer_id, mesh)
    }

    fn reserved_destination(app: &mut OccluViewApp) -> (SceneKey, PaneId) {
        (
            app.workspace.ids.allocate_scene().unwrap(),
            app.workspace.ids.allocate_pane_id().unwrap(),
        )
    }

    fn move_to_new_scene(
        app: &mut OccluViewApp,
        source: SceneKey,
        layer: SceneMeshId,
    ) -> (SceneKey, PaneId) {
        let (destination, pane) = reserved_destination(app);
        app.transfer_layers(
            source,
            TransferDestination::CreateBeside {
                scene: destination,
                pane,
                side: SplitSide::Right,
            },
            LayerIds::one(layer),
        )
        .unwrap();
        (destination, pane)
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn save_after_transfer_keeps_layer_state_through_undo_redo() {
        let (mut app, source, layer_id, mesh) = app_with_layer();
        let (reserved_key, pane) = reserved_destination(&mut app);
        let source_document = &mut app.workspace.scenes[0].document;
        source_document.current_paths = vec![PathBuf::from("source-scan.stl")];
        source_document.unsaved_edit_layer_ids.insert(layer_id);
        source_document.hidden_layer_stack.push(layer_id);
        source_document
            .translucent_layer_restore
            .insert(layer_id, 0.35);
        app.ui.popup_open_at_frame_start = true;

        app.transfer_layers(
            source,
            TransferDestination::CreateBeside {
                scene: reserved_key,
                pane,
                side: SplitSide::Right,
            },
            LayerIds::one(layer_id),
        )
        .unwrap();

        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(app.workspace.scenes[1].key, reserved_key);
        let moved = app.workspace.scenes[1]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone();
        assert_eq!(moved.id(), layer_id);
        assert!(Arc::ptr_eq(&moved.mesh, &mesh));
        assert_eq!(
            app.workspace.scenes[1].document.current_paths,
            [PathBuf::from("source-scan.stl")]
        );
        assert!(app.workspace.scenes[1]
            .document
            .unsaved_edit_layer_ids
            .contains(&layer_id));
        assert_eq!(
            app.workspace.scenes[1].document.hidden_layer_stack,
            [layer_id]
        );
        assert_eq!(
            app.workspace.scenes[1]
                .document
                .translucent_layer_restore
                .get(&layer_id),
            Some(&0.35)
        );

        let moved_document = &mut app.workspace.scenes[1].document;
        moved_document.unsaved_edit_layer_ids.remove(&layer_id);
        moved_document.hidden_layer_stack.clear();
        moved_document
            .translucent_layer_restore
            .insert(layer_id, 0.8);
        let moved_layer = moved_document
            .live_scene_mut()
            .unwrap()
            .meshes_mut()
            .first_mut()
            .unwrap();
        moved_layer.visible = false;
        moved_layer.opacity = 0.45;

        app.navigate_workspace_history(reserved_key, HistoryDirection::Undo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 1);
        let restored = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone();
        assert_eq!(restored.id(), layer_id);
        assert!(Arc::ptr_eq(&restored.mesh, &mesh));
        assert_eq!(
            app.workspace.scenes[0].document.current_paths,
            [PathBuf::from("source-scan.stl")]
        );
        assert!(!app.workspace.scenes[0]
            .document
            .unsaved_edit_layer_ids
            .contains(&layer_id));
        assert!(app.workspace.scenes[0]
            .document
            .hidden_layer_stack
            .is_empty());
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .translucent_layer_restore
                .get(&layer_id),
            Some(&0.8)
        );
        assert!(!restored.visible);
        assert_eq!(restored.opacity, 0.45);

        let restored_document = &mut app.workspace.scenes[0].document;
        restored_document.current_paths = vec![PathBuf::from("changed-after-undo.stl")];
        restored_document.unsaved_edit_layer_ids.remove(&layer_id);
        restored_document.hidden_layer_stack.clear();
        restored_document
            .translucent_layer_restore
            .insert(layer_id, 0.9);
        let restored_layer = restored_document
            .live_scene_mut()
            .unwrap()
            .meshes_mut()
            .first_mut()
            .unwrap();
        restored_layer.visible = true;
        restored_layer.opacity = 0.65;

        app.navigate_workspace_history(source, HistoryDirection::Redo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 2);
        let recreated = app.workspace.scenes[1].key;
        assert_eq!(recreated.id, reserved_key.id);
        assert_ne!(recreated.epoch, reserved_key.epoch);
        let redone = app.workspace.scenes[1]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone();
        assert_eq!(redone.id(), layer_id);
        assert!(Arc::ptr_eq(&redone.mesh, &mesh));
        assert_eq!(
            app.workspace.scenes[1].document.current_paths,
            [PathBuf::from("changed-after-undo.stl")]
        );
        assert!(!app.workspace.scenes[1]
            .document
            .unsaved_edit_layer_ids
            .contains(&layer_id));
        assert!(app.workspace.scenes[1]
            .document
            .hidden_layer_stack
            .is_empty());
        assert_eq!(
            app.workspace.scenes[1]
                .document
                .translucent_layer_restore
                .get(&layer_id),
            Some(&0.9)
        );
        assert!(redone.visible);
        assert_eq!(redone.opacity, 0.65);
    }

    #[test]
    fn rejected_preflights_do_not_leave_an_empty_auto_created_scene() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let original_layout = app.workspace.layout;
        let (destination, pane) = reserved_destination(&mut app);

        app.workspace.scenes[0].document.unsaved_sculpt_stroke = true;
        assert!(app
            .transfer_layers(
                source,
                TransferDestination::CreateBeside {
                    scene: destination,
                    pane,
                    side: SplitSide::Left,
                },
                LayerIds::one(layer_id),
            )
            .is_err());
        assert_eq!(app.workspace.scenes.len(), 1);
        assert_eq!(app.workspace.layout, original_layout);

        app.workspace.scenes[0].document.unsaved_sculpt_stroke = false;
        let unknown_layer = SceneMesh::new(Mesh::empty()).id();
        assert!(app
            .transfer_layers(
                source,
                TransferDestination::CreateBeside {
                    scene: destination,
                    pane,
                    side: SplitSide::Left,
                },
                LayerIds::one(unknown_layer),
            )
            .is_err());
        assert_eq!(app.workspace.scenes.len(), 1);
        assert_eq!(app.workspace.layout, original_layout);
        assert!(app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .iter()
            .any(|layer| layer.id() == layer_id));
    }

    #[test]
    fn transfer_undo_keeps_previously_committed_scene_history() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let layer = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone();
        let token = app.workspace.scenes[0]
            .document
            .edit_mode
            .begin_layer_edit(&layer, EditModeCommand::InvertNormals)
            .unwrap();
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .edit_mode
                .finish_layer_edit_success(token),
            BusyFinish::Applied
        );
        app.workspace.scenes[0]
            .document
            .edit_mode
            .finish_edit_session();

        let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
        assert_eq!(
            app.workspace
                .history
                .borrow()
                .top_step(Some(source), HistoryDirection::Undo)
                .unwrap()
                .kind,
            HistoryStepKind::Transfer
        );

        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        let previous = app
            .workspace
            .history
            .borrow()
            .top_step(Some(source), HistoryDirection::Undo)
            .unwrap();
        assert_eq!(previous.kind, HistoryStepKind::LayerEdit { layer_id });
    }

    #[test]
    fn undo_preserves_a_touched_auto_created_destination_for_redo() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
        let dest = app
            .workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == destination)
            .unwrap();
        dest.name = "Edited destination".to_owned();
        dest.preserve_on_transfer_undo = true;

        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        let retained = app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap();
        assert_eq!(retained.name, "Edited destination");
        assert!(retained.preserve_on_transfer_undo);
        assert!(retained
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .is_empty());

        app.navigate_workspace_history(source, HistoryDirection::Redo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 2);
        let retained = app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap();
        assert_eq!(retained.name, "Edited destination");
        assert_eq!(
            retained.document.scene.as_ref().unwrap().meshes()[0].id(),
            layer_id
        );
    }

    #[test]
    fn auto_created_destination_with_later_transfer_redo_keeps_both_commands_navigable() {
        let (mut app, source, layer_a, _) = app_with_layer();
        let layer_b = SceneMesh::new(Arc::new(Mesh::empty()));
        let layer_b_id = layer_b.id();
        let source_document = &mut app.workspace.scenes[0].document;
        source_document.current_paths =
            vec![PathBuf::from("scan-a.stl"), PathBuf::from("scan-b.stl")];
        source_document.live_scene_mut().unwrap().add(layer_b);

        let (destination, _) = move_to_new_scene(&mut app, source, layer_a);
        app.transfer_layers(
            source,
            TransferDestination::Existing(destination),
            LayerIds::one(layer_b_id),
        )
        .unwrap();

        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();

        assert_eq!(app.workspace.scenes.len(), 2);
        let retained = app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap();
        assert!(retained
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .is_empty());

        app.navigate_workspace_history(source, HistoryDirection::Redo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(
            app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .key,
            destination
        );
        assert!(app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .iter()
            .any(|layer| layer.id() == layer_b_id));

        app.navigate_workspace_history(destination, HistoryDirection::Redo)
            .unwrap();
        let destination_scene = app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .clone();
        assert!(destination_scene
            .meshes()
            .iter()
            .any(|layer| layer.id() == layer_a));
        assert!(destination_scene
            .meshes()
            .iter()
            .any(|layer| layer.id() == layer_b_id));
    }

    #[test]
    // Keep the complete transfer/edit/undo/redo scenario readable in one test.
    #[allow(clippy::too_many_lines)]
    fn undone_local_edit_keeps_auto_created_destination_for_transfer_redo() {
        let (mut app, source, layer_id, original_mesh) = app_with_layer();
        let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
        let layer = app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone();
        let token = app
            .workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .edit_mode
            .begin_layer_edit(&layer, EditModeCommand::InvertNormals)
            .unwrap();
        let changed_mesh = Arc::new(Mesh::empty());
        app.workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .live_scene_mut()
            .unwrap()
            .meshes_mut()[0]
            .mesh = Arc::clone(&changed_mesh);
        assert_eq!(
            app.workspace
                .scenes
                .iter_mut()
                .find(|scene| scene.key == destination)
                .unwrap()
                .document
                .edit_mode
                .finish_layer_edit_success(token),
            BusyFinish::Applied
        );
        app.workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .edit_mode
            .finish_edit_session();

        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        assert!(Arc::ptr_eq(
            &app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[0]
                .mesh,
            &original_mesh
        ));
        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 2);
        assert!(app
            .workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .is_empty());

        app.navigate_workspace_history(source, HistoryDirection::Redo)
            .unwrap();
        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(
            app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .key,
            destination
        );
        assert_eq!(
            app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[0]
                .id(),
            layer_id
        );
        assert!(Arc::ptr_eq(
            &app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[0]
                .mesh,
            &original_mesh
        ));
        app.navigate_workspace_history(destination, HistoryDirection::Redo)
            .unwrap();
        assert!(Arc::ptr_eq(
            &app.workspace
                .scenes
                .iter()
                .find(|scene| scene.key == destination)
                .unwrap()
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[0]
                .mesh,
            &changed_mesh
        ));
    }

    #[test]
    fn edit_checkpoint_in_either_participant_blocks_linked_undo() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
        let checkpoint = app
            .workspace
            .history
            .borrow_mut()
            .begin_edit_checkpoint(Some(destination), Scene::new(), 1)
            .unwrap();

        assert!(app
            .navigate_workspace_history(source, HistoryDirection::Undo)
            .is_err());
        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(
            app.workspace
                .history
                .borrow()
                .top_step(Some(source), HistoryDirection::Undo)
                .unwrap()
                .kind,
            HistoryStepKind::Transfer
        );
        assert!(app
            .workspace
            .history
            .borrow_mut()
            .cancel_edit_checkpoint(checkpoint)
            .is_some());
    }

    #[test]
    fn pending_load_in_either_participant_blocks_linked_undo() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
        app.loader.enqueue(crate::scene_loading::SceneLoadRequest {
            scene_key: destination,
            paths: vec![PathBuf::from("scan.stl")],
            source: "test",
            mode: crate::scene_loading::SceneLoadMode::Append,
            content_revision_at_request: 0,
            dirty_at_request: false,
            requested_at: std::time::Instant::now(),
        });

        assert!(app
            .navigate_workspace_history(source, HistoryDirection::Undo)
            .is_err());
        assert_eq!(app.workspace.scenes.len(), 2);
        assert_eq!(
            app.workspace
                .history
                .borrow()
                .top_step(Some(source), HistoryDirection::Undo)
                .unwrap()
                .kind,
            HistoryStepKind::Transfer
        );
    }

    #[test]
    fn transfer_drops_derived_layer_overlay_and_clears_only_removed_scene_rulers() {
        let (mut app, source, layer_id, _) = app_with_layer();
        let colors = Arc::new(Vec::new());
        let analyzed_layer = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .clone()
            .with_overlay(occluview_core::OverlayKind::Measured, Some(colors));
        let mut source_scene = Scene::new();
        source_scene.add(analyzed_layer);
        app.workspace.scenes[0].document.scene = Some(Arc::new(source_scene));
        app.workspace.scenes[0]
            .tools
            .measure
            .place_ruler_point(Vec3::ZERO);
        app.workspace.scenes[0]
            .tools
            .measure
            .place_ruler_point(Vec3::X);

        let destination = app.workspace.ids.allocate_scene().unwrap();
        let pane = app.workspace.ids.allocate_pane_id().unwrap();
        let mut destination_session = SceneSession::new(
            destination,
            pane,
            "Destination".to_owned(),
            None,
            &app.workspace.history,
        );
        let mut destination_scene = Scene::new();
        destination_scene.add(SceneMesh::new(Mesh::empty()));
        destination_session.document.scene = Some(Arc::new(destination_scene));
        destination_session
            .tools
            .measure
            .place_ruler_point(Vec3::ZERO);
        destination_session.tools.measure.place_ruler_point(Vec3::Y);
        app.workspace.scenes.push(destination_session);
        app.workspace.layout = WorkspaceLayout::SideBySide {
            left: app.workspace.scenes[0].pane,
            right: pane,
            ratio: 0.5,
        };

        app.ui.popup_open_at_frame_start = true;
        app.transfer_layers(
            source,
            TransferDestination::Existing(destination),
            LayerIds::one(layer_id),
        )
        .unwrap();
        assert_eq!(app.workspace.scenes[0].tools.measure.ruler_count(), 0);
        assert_eq!(app.workspace.scenes[1].tools.measure.ruler_count(), 1);
        assert_eq!(
            app.workspace.scenes[1]
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[1]
                .overlay_kind(),
            None
        );

        app.workspace.scenes[0]
            .tools
            .measure
            .place_ruler_point(Vec3::ZERO);
        app.workspace.scenes[0]
            .tools
            .measure
            .place_ruler_point(Vec3::Z);
        app.navigate_workspace_history(destination, HistoryDirection::Undo)
            .unwrap();
        assert_eq!(app.workspace.scenes[0].tools.measure.ruler_count(), 1);
        assert_eq!(app.workspace.scenes[1].tools.measure.ruler_count(), 0);
        assert_eq!(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .unwrap()
                .meshes()[0]
                .overlay_kind(),
            None
        );
    }
}
