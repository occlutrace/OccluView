//! Undo and redo across the workspace journal and transfer records.

use std::collections::BTreeSet;

use super::apply::opposite;
use super::command::navigation_error_text;
use super::record::{HistoryNavigationRollback, MoveDirection};
use crate::app::workspace::history::{
    AutoCreatedDestination, HistoryDirection, HistoryStepKind, TransferRecord,
};
use crate::app::workspace::id::SceneKey;
use crate::app::workspace::state::SceneSession;
use crate::app::OccluViewApp;
use occluview_core::SceneMeshId;

impl OccluViewApp {
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
    pub(super) fn finish_transfer_scene_change(
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
                scene.render.section_cache = occluview_mesh_edit::scene::SectionCache::new();
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
