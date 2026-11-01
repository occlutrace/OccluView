//! The transfer command: preflight, record, and apply.

use super::apply::transfer_payload_bytes;
use super::record::{MoveDirection, TransferIntent};
use crate::app::workspace::commands::{LayerIds, SplitSide, TransferDestination};
use crate::app::workspace::history::{AutoCreatedDestination, HistoryError, NavigationError};
use crate::app::workspace::id::SceneKey;
use crate::app::workspace::layout::WorkspaceLayout;
use crate::app::workspace::state::SceneSession;
use crate::app::OccluViewApp;

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
    pub(super) fn ensure_transfer_ready(&self, key: SceneKey) -> Result<(), String> {
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
    pub(super) fn activate_scene(&mut self, key: SceneKey) -> Result<(), String> {
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
            crate::app::workspace::input::ActivationResult::DeferredUntilGestureEnds
            | crate::app::workspace::input::ActivationResult::IgnoredWhileGestureCaptured => {
                Err(self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("workspace-wait-gesture")))
            }
            crate::app::workspace::input::ActivationResult::Activated
            | crate::app::workspace::input::ActivationResult::Unchanged => Ok(()),
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
pub(super) fn navigation_error_text(
    error: NavigationError,
    locale: &crate::i18n::LocaleManager,
) -> String {
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
