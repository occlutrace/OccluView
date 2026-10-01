//! Workspace-scoped loading. Each request keeps the scene lifetime selected
//! when the user accepted it, while one coordinator serializes every decoder.

use super::{
    combine_loaded_scene, egui, load_error_dialog, load_failure_summary, load_status_message,
    single_instance, AppErrorAction, AppErrorDialog, LoadQueueCameraReset, PathBuf,
    PendingReplaceOpen, Scene, SceneContext, SceneLoadMode, FOREGROUND_PULSE_DURATION,
};
use crate::scene_loading::{
    replace_result_requires_guard, DecodedSceneLoad, PendingSceneLoad, SceneLoadRequest,
};
use anyhow::Result;
use occluview_formats::hps::RuntimeHpsKeyProvider;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Instant;

#[derive(Clone, Copy)]
struct LoadAuthorization {
    mode: SceneLoadMode,
    requested_at: Instant,
    dirty_at_request: bool,
}

/// Decode against the current total retained memory of every scene.
pub(super) fn load_scene(paths: &[PathBuf], retained_scene_bytes: u64) -> Result<Scene> {
    occluview_formats::read_files_with_memory_budget(
        paths,
        &RuntimeHpsKeyProvider,
        retained_scene_bytes,
    )
    .map_err(|(path, error)| {
        anyhow::Error::new(error).context(format!("{}: format reader failed", path.display()))
    })
}

/// Recheck the resulting workspace size when a decoded scene is about to be
/// installed. Another scene or its shared history may have grown while the
/// single decoder was parsing this request.
fn resulting_workspace_memory_error(
    retained_scene_bytes: u64,
    replaced_scene_bytes: u64,
    imported_scene_bytes: u64,
    mode: SceneLoadMode,
) -> Option<occluview_formats::FormatError> {
    let retained_after_commit = match mode {
        SceneLoadMode::Replace => retained_scene_bytes.saturating_sub(replaced_scene_bytes),
        SceneLoadMode::Append => retained_scene_bytes,
    };
    let estimated_bytes = retained_after_commit.saturating_add(imported_scene_bytes);
    (estimated_bytes > occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES).then_some(
        occluview_formats::FormatError::MemoryBudgetExceeded {
            estimated_bytes,
            limit: occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES,
        },
    )
}

/// Remove identifying paths before recording a load failure in the crash log.
fn failure_without_paths(error: &anyhow::Error, paths: &[PathBuf]) -> String {
    let mut text = format!("{error:#}");
    let mut rendered: Vec<(String, &PathBuf)> = paths
        .iter()
        .map(|path| (path.display().to_string(), path))
        .filter(|(path, _)| !path.is_empty())
        .collect();
    rendered.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    for (path_text, path) in rendered {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|extension| occluview_formats::V1_OPEN_EXTENSIONS.contains(&extension.as_str()))
            .unwrap_or_else(|| "file".to_owned());
        text = text.replace(&path_text, &format!("<{extension}>"));
    }
    text
}

/// Native file drops are routed by the workspace after it resolves a pane.
pub(crate) fn native_drop_paths(files: &[egui::DroppedFileHandle]) -> Vec<PathBuf> {
    files.iter().map(|file| file.path().to_path_buf()).collect()
}

impl SceneContext<'_> {
    /// Open into this exact scene. Dirty or open Edit sessions enter the
    /// scene-keyed guard; a guard already owned by the other scene keeps its
    /// modal and this request waits in the workspace queue.
    pub(super) fn replace_paths(&mut self, paths: &[PathBuf], source: &'static str) {
        if paths.is_empty() {
            return;
        }
        *self.preserve_on_transfer_undo = true;
        let requested_at = Instant::now();
        if self
            .ui
            .pending_replace_open
            .as_ref()
            .is_some_and(|pending| pending.scene_key == self.scene_key)
        {
            self.loader.supersede_scene(self.scene_key);
            self.ui.pending_replace_open = Some(PendingReplaceOpen {
                scene_key: self.scene_key,
                paths: paths.to_vec(),
                source,
                requested_at,
            });
            self.scene_ui.status_message = None;
            return;
        }

        if self.ui.pending_replace_open.is_none()
            && !self.ui.close_guard_open
            && !self.ui.modal_dialog_open()
            && self.replace_open_needs_guard()
        {
            self.loader.supersede_scene(self.scene_key);
            self.ui.pending_replace_open = Some(PendingReplaceOpen {
                scene_key: self.scene_key,
                paths: paths.to_vec(),
                source,
                requested_at,
            });
            self.scene_ui.status_message = None;
            return;
        }

        let defer_guard = self.ui.modal_dialog_open()
            || self.ui.pending_replace_open.is_some()
            || self.ui.close_guard_open;
        self.load_paths_with_mode(
            paths,
            source,
            LoadAuthorization {
                mode: SceneLoadMode::Replace,
                requested_at,
                dirty_at_request: !defer_guard && self.replace_open_needs_guard(),
            },
        );
    }

    /// Start the Replace explicitly authorized by the scene-keyed guard.
    pub(super) fn replace_paths_confirmed(
        &mut self,
        paths: &[PathBuf],
        source: &'static str,
        requested_at: Instant,
    ) {
        if paths.is_empty() {
            return;
        }
        if self.document.edit_mode.is_busy() {
            self.ui.pending_replace_open = Some(PendingReplaceOpen {
                scene_key: self.scene_key,
                paths: paths.to_vec(),
                source,
                requested_at,
            });
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("edit-session-busy")),
            );
            return;
        }
        self.load_paths_with_mode(
            paths,
            source,
            LoadAuthorization {
                mode: SceneLoadMode::Replace,
                requested_at,
                dirty_at_request: self.replace_open_needs_guard(),
            },
        );
    }

    pub(super) fn append_paths(&mut self, paths: &[PathBuf], source: &'static str) {
        self.load_paths_with_mode(
            paths,
            source,
            LoadAuthorization {
                mode: SceneLoadMode::Append,
                requested_at: Instant::now(),
                dirty_at_request: false,
            },
        );
    }

    /// Files dropped into a pane always add to that pane. `scene_key` comes
    /// from the drop-target decision, never from whichever pane is focused
    /// when decoding later finishes.
    pub(super) fn enqueue_dropped_paths(
        &mut self,
        scene_key: crate::app::workspace::id::SceneKey,
        paths: &[PathBuf],
    ) {
        if scene_key != self.scene_key || paths.is_empty() {
            return;
        }
        self.append_paths(paths, "drop");
    }

    fn load_paths_with_mode(
        &mut self,
        paths: &[PathBuf],
        source: &'static str,
        authorization: LoadAuthorization,
    ) {
        if paths.is_empty() {
            return;
        }
        *self.preserve_on_transfer_undo = true;
        let LoadAuthorization {
            mode,
            requested_at,
            dirty_at_request,
        } = authorization;
        let request = SceneLoadRequest {
            scene_key: self.scene_key,
            paths: paths.to_vec(),
            source,
            mode,
            content_revision_at_request: self.document.content_revision,
            dirty_at_request,
            requested_at,
        };
        if mode == SceneLoadMode::Replace {
            self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
            self.document.camera_modified_during_load = false;
        }
        let queued = self.loader.active.is_some()
            || self.loader.decoded.is_some()
            || !self.loader.queued.is_empty()
            || self.ui.modal_dialog_open()
            || (mode == SceneLoadMode::Append && self.append_load_is_blocked());
        self.loader.enqueue(request);
        if queued {
            self.scene_ui.status_message = Some(
                if mode == SceneLoadMode::Append && self.append_load_is_blocked() {
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("edit-session-busy"))
                } else {
                    self.ui.locale.tr_plural(
                        crate::i18n::message_id!("load-queued"),
                        &[],
                        &[("count", paths.len())],
                    )
                },
            );
        } else {
            self.scene_ui.status_message =
                Some(load_status_message(mode, paths.len(), &self.ui.locale));
        }
        self.start_next_queued_load();
    }

    fn append_load_is_blocked(&self) -> bool {
        self.document.edit_mode.has_active_session()
            || self.document.edit_mode.is_busy()
            || self.sculpt_has_live_work()
    }

    fn replace_open_needs_guard(&self) -> bool {
        self.document.edit_mode.has_active_session()
            || self.document.edit_mode.is_dirty()
            || self.document.has_unsaved_mesh_edits()
    }

    /// The global decoder is polled only by its target scene. Retired targets
    /// are reaped by the root coordinator; a scene switch never takes another
    /// scene's result.
    pub(super) fn process_scene_loads(&mut self, ctx: &egui::Context) {
        if self.process_parked_result(ctx) {
            return;
        }

        if self.loader.active_for(self.scene_key) {
            let Some(active) = self.loader.active.as_ref() else {
                return;
            };
            let polled = active.receiver.try_recv();
            match polled {
                Err(TryRecvError::Empty) => return,
                Ok(result) => {
                    let Some(pending) = self.loader.take_active_for(self.scene_key) else {
                        return;
                    };
                    if pending.superseded {
                        self.start_next_queued_load();
                        return;
                    }
                    let decoded = DecodedSceneLoad { pending, result };
                    if self.must_park_decoded(&decoded) {
                        self.park_load(decoded);
                        return;
                    }
                    self.apply_decoded_load(decoded, ctx);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    let Some(pending) = self.loader.take_active_for(self.scene_key) else {
                        return;
                    };
                    if pending.superseded {
                        self.start_next_queued_load();
                        return;
                    }
                    let decoded = DecodedSceneLoad {
                        pending,
                        result: Err(anyhow::anyhow!(
                            "scene loader stopped before returning a result"
                        )),
                    };
                    if self.must_park_decoded(&decoded) {
                        self.park_load(decoded);
                    } else {
                        self.apply_decoded_load(decoded, ctx);
                    }
                    return;
                }
            }
        }

        self.start_next_queued_load();
    }

    /// Returns true when a parked result belongs to this target. It either
    /// stays parked, is converted into the Replace confirmation, or commits.
    fn process_parked_result(&mut self, ctx: &egui::Context) -> bool {
        if !self.loader.decoded_for(self.scene_key) {
            return false;
        }
        let replace_needs_guard = self
            .loader
            .decoded
            .as_ref()
            .is_some_and(|decoded| self.loaded_replace_needs_guard(&decoded.pending));
        if self.ui.pending_replace_open.as_ref().is_some_and(|open| {
            open.scene_key == self.scene_key
                && self
                    .loader
                    .decoded
                    .as_ref()
                    .is_some_and(|decoded| open.requested_at > decoded.pending.requested_at)
        }) {
            let _ = self.loader.take_decoded_for(self.scene_key);
            return true;
        }
        if self.ui.modal_dialog_open()
            || (self
                .loader
                .decoded
                .as_ref()
                .is_some_and(|decoded| decoded.pending.mode == SceneLoadMode::Append)
                && self.append_load_is_blocked())
        {
            if self.append_load_is_blocked()
                && self
                    .loader
                    .decoded
                    .as_ref()
                    .is_some_and(|decoded| decoded.pending.mode == SceneLoadMode::Append)
            {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("edit-session-busy")),
                );
            }
            return true;
        }
        if replace_needs_guard {
            if let Some(decoded) = self.loader.take_decoded_for(self.scene_key) {
                self.park_replace_open(decoded.pending);
            }
            return true;
        }
        if let Some(decoded) = self.loader.take_decoded_for(self.scene_key) {
            self.apply_decoded_load(decoded, ctx);
        }
        true
    }

    fn must_park_decoded(&self, decoded: &DecodedSceneLoad) -> bool {
        self.ui.modal_dialog_open()
            || (decoded.pending.mode == SceneLoadMode::Append && self.append_load_is_blocked())
    }

    fn park_load(&mut self, decoded: DecodedSceneLoad) {
        if decoded.pending.mode == SceneLoadMode::Append && self.append_load_is_blocked() {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("edit-session-busy")),
            );
        }
        self.loader.park_decoded(decoded);
    }

    fn apply_decoded_load(&mut self, decoded: DecodedSceneLoad, ctx: &egui::Context) {
        let source = decoded.pending.source;
        let load_settled = self.loader.queued_len() == 0;
        let raise_after_handoff = source == "single-instance" && load_settled;
        let raise_after_startup = source == "startup" && load_settled;
        self.apply_scene_load_result(decoded.pending, decoded.result, ctx);
        if raise_after_handoff {
            let token = self.platform.pending_raise_token.clone();
            self.raise_window_for_incoming_open(ctx);
            single_instance::complete_startup_notification(token.as_deref());
            self.platform.pending_raise_token = None;
        } else if raise_after_startup {
            self.raise_window_for_startup_open(ctx);
        }
        self.start_next_queued_load();
    }

    fn start_next_queued_load(&mut self) {
        if self.loader.active.is_some()
            || self.loader.decoded.is_some()
            || self.ui.modal_dialog_open()
        {
            return;
        }
        let mut append_blocked_scenes = self.append_blocked_scene_keys.clone();
        append_blocked_scenes.retain(|key| *key != self.scene_key);
        if self.append_load_is_blocked() {
            append_blocked_scenes.push(self.scene_key);
        }
        let Some(request) = self
            .loader
            .take_next_for(self.scene_key, &append_blocked_scenes)
        else {
            return;
        };
        if request.mode == SceneLoadMode::Replace
            && ((!request.dirty_at_request && self.document.edit_mode.has_active_session())
                || replace_result_requires_guard(
                    request.content_revision_at_request,
                    self.document.content_revision,
                    request.dirty_at_request,
                    self.replace_open_needs_guard(),
                    self.document.edit_mode.is_busy(),
                ))
        {
            self.ui.pending_replace_open = Some(PendingReplaceOpen {
                scene_key: self.scene_key,
                paths: request.paths,
                source: request.source,
                requested_at: request.requested_at,
            });
            self.scene_ui.status_message = None;
            return;
        }
        self.start_scene_load(request);
    }

    fn start_scene_load(&mut self, request: SceneLoadRequest) {
        let SceneLoadRequest {
            scene_key,
            paths,
            source,
            mode,
            content_revision_at_request,
            dirty_at_request,
            requested_at,
        } = request;
        let load_paths = paths.clone();
        let retained_scene_bytes = u64::try_from(self.retained_scene_bytes).unwrap_or(u64::MAX);
        let started_at = Instant::now();
        let (sender, receiver) = mpsc::channel();
        let repaint_ctx = self.ui.repaint_ctx.clone();
        let spawn_result = std::thread::Builder::new()
            .name("scene-load".to_owned())
            .spawn(move || {
                let result = load_scene(&load_paths, retained_scene_bytes);
                let _ = sender.send(result);
                repaint_ctx.request_repaint();
            });
        if let Err(error) = spawn_result {
            let append = mode == SceneLoadMode::Append;
            self.scene_ui.status_message = Some(self.ui.locale.tr(if append {
                crate::i18n::message_id!("load-add-failed-start")
            } else {
                crate::i18n::message_id!("load-open-failed-start")
            }));
            self.ui.app_error = Some(AppErrorDialog {
                title: self.ui.locale.tr(if append {
                    crate::i18n::message_id!("error-add-title")
                } else {
                    crate::i18n::message_id!("error-open-title")
                }),
                summary: self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("load-loader-failed-summary")),
                details: format!("Loader thread start failed\n\n{error:#}"),
                action: AppErrorAction::None,
            });
            tracing::error!(?error, source, "scene loader thread spawn failed");
            return;
        }
        self.scene_ui.status_message =
            Some(load_status_message(mode, paths.len(), &self.ui.locale));
        self.loader.install_active(PendingSceneLoad {
            scene_key,
            paths,
            source,
            mode,
            started_at,
            receiver,
            superseded: false,
            content_revision_at_request,
            dirty_at_request,
            requested_at,
        });
    }

    /// Replace authorization is checked at acceptance and again at commit.
    fn loaded_replace_needs_guard(&self, pending: &PendingSceneLoad) -> bool {
        pending.mode == SceneLoadMode::Replace
            && ((!pending.dirty_at_request && self.document.edit_mode.has_active_session())
                || replace_result_requires_guard(
                    pending.content_revision_at_request,
                    self.document.content_revision,
                    pending.dirty_at_request,
                    self.replace_open_needs_guard(),
                    self.document.edit_mode.is_busy(),
                ))
    }

    fn park_replace_open(&mut self, pending: PendingSceneLoad) {
        let newer_same_scene_is_parked =
            self.ui.pending_replace_open.as_ref().is_some_and(|open| {
                open.scene_key == pending.scene_key && open.requested_at > pending.requested_at
            });
        if newer_same_scene_is_parked {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("load-superseded-parked-open")),
            );
            return;
        }
        if self.ui.pending_replace_open.is_some() || self.ui.close_guard_open {
            self.loader.requeue_front(SceneLoadRequest {
                scene_key: pending.scene_key,
                paths: pending.paths,
                source: pending.source,
                mode: SceneLoadMode::Replace,
                content_revision_at_request: pending.content_revision_at_request,
                dirty_at_request: pending.dirty_at_request,
                requested_at: pending.requested_at,
            });
            return;
        }
        self.ui.pending_replace_open = Some(PendingReplaceOpen {
            scene_key: pending.scene_key,
            paths: pending.paths,
            source: pending.source,
            requested_at: pending.requested_at,
        });
        self.scene_ui.status_message = None;
    }

    fn forget_replaced_scene_state(&mut self) {
        self.document.edit_mode.clear();
        self.document.discard_edit_metadata();
        self.discard_align_drag();
        self.document.clear_unsaved_mesh_edits();
        self.document.hidden_layer_stack.clear();
        self.document.translucent_layer_restore.clear();
        self.document.focused_layer_id = None;
    }

    pub(super) fn apply_scene_load_result(
        &mut self,
        pending: PendingSceneLoad,
        result: Result<Scene>,
        ctx: &egui::Context,
    ) {
        let append = pending.mode == SceneLoadMode::Append;
        if append && self.append_load_is_blocked() {
            self.park_load(DecodedSceneLoad { pending, result });
            return;
        }
        match result {
            Ok(scene) => {
                if self.loaded_replace_needs_guard(&pending) {
                    self.park_replace_open(pending);
                    return;
                }
                self.commit_loaded_scene(pending, scene, ctx);
            }
            Err(error) => self.report_scene_load_failure(&pending, error),
        }
    }

    // Keep the ordered document, camera, epoch, queue and persistence updates
    // together: this is one scene-install transaction, with no intermediate
    // state exposed to another pane.
    #[allow(clippy::too_many_lines)]
    fn commit_loaded_scene(
        &mut self,
        pending: PendingSceneLoad,
        scene: Scene,
        ctx: &egui::Context,
    ) {
        let append = pending.mode == SceneLoadMode::Append;
        let replaced_scene_bytes = self
            .document
            .scene
            .as_deref()
            .map_or(0, Scene::estimated_memory_bytes);
        let retained_scene_bytes = u64::try_from(self.retained_scene_bytes).unwrap_or(u64::MAX);
        if let Some(memory_error) = resulting_workspace_memory_error(
            retained_scene_bytes,
            replaced_scene_bytes,
            scene.estimated_memory_bytes(),
            pending.mode,
        ) {
            let error = match pending.paths.first() {
                Some(path) => anyhow::Error::new(memory_error)
                    .context(format!("{}: format reader failed", path.display())),
                None => anyhow::Error::new(memory_error),
            };
            self.report_scene_load_failure(&pending, error);
            return;
        }
        let next_epoch = if append {
            None
        } else {
            match self.ids.allocate_scene_epoch() {
                Ok(epoch) => Some(epoch),
                Err(error) => {
                    self.ui.app_error = Some(AppErrorDialog {
                        title: self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("error-open-title")),
                        summary: self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("load-loader-failed-summary")),
                        details: error.to_string(),
                        action: AppErrorAction::None,
                    });
                    return;
                }
            }
        };
        let scene_ready_ms = pending.started_at.elapsed().as_millis();
        let old_key = self.scene_key;
        let (scene, current_paths) = if append {
            combine_loaded_scene(
                self.document.scene.as_deref(),
                &self.document.current_paths,
                scene,
                &pending.paths,
            )
        } else {
            (scene, pending.paths.clone())
        };
        let recent_paths = current_paths.clone();
        let queued_after_current =
            append
                && self.loader.queued.iter().any(|queued| {
                    queued.scene_key == old_key && queued.mode == SceneLoadMode::Append
                });
        if !append {
            self.forget_replaced_scene_state();
        }
        let reset_camera = if append {
            let reset = self.document.load_queue_camera_reset
                == LoadQueueCameraReset::WhenQueueDrains
                && !queued_after_current
                && !self.document.camera_modified_during_load;
            if reset {
                self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
            }
            reset
        } else if queued_after_current {
            self.document.load_queue_camera_reset = LoadQueueCameraReset::WhenQueueDrains;
            self.render.camera.is_none()
        } else {
            self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
            self.persistence.settings.frame_scene_on_open
        };
        self.document.current_paths = current_paths;
        self.set_scene(scene, reset_camera);
        if self.document.load_queue_camera_reset == LoadQueueCameraReset::WhenQueueDrains
            && queued_after_current
        {
            self.render.invalidation.suppress_redraw();
            self.render.rendered = None;
            self.clear_live_viewport();
        }
        if let Some(epoch) = next_epoch {
            *self.lifetime_epoch = epoch;
            let new_key = crate::app::workspace::id::SceneKey::new(old_key.id, epoch);
            self.loader.advance_scene_lifetime_after_replace(
                old_key,
                new_key,
                pending.requested_at,
            );
            self.scene_key = new_key;
            self.document.edit_mode.rebind_scene_scope(new_key);
            super::state::restore_sculpt_preferences(
                ctx,
                self.scene_key,
                &mut self.persistence.settings,
            );
            self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
            self.document.camera_modified_during_load = false;
        }
        self.persistence.push_recent_scene(&recent_paths);
        self.persistence.save_recent_files();
        self.scene_ui.status_message = self.ambiguous_units_notice();
        tracing::info!(
            source = pending.source,
            append,
            path_count = pending.paths.len(),
            scene_ready_ms,
            scene_id = old_key.id.get(),
            "scene load completed"
        );
        ctx.request_repaint();
    }

    fn report_scene_load_failure(&mut self, pending: &PendingSceneLoad, error: anyhow::Error) {
        let append = pending.mode == SceneLoadMode::Append;
        let action = if append { "Add" } else { "Open" };
        if !append {
            self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
        } else if self.document.load_queue_camera_reset == LoadQueueCameraReset::WhenQueueDrains
            && !self.loader.has_queued_for(self.scene_key)
        {
            self.document.load_queue_camera_reset = LoadQueueCameraReset::Idle;
            if self.document.scene.is_some() {
                self.reset_camera_to_home();
            }
        }
        self.scene_ui.status_message = Some(load_failure_summary(&self.ui.locale, action, &error));
        self.ui.app_error = Some(load_error_dialog(
            &self.ui.locale,
            action,
            &error,
            &pending.paths,
        ));
        tracing::error!(
            error = %failure_without_paths(&error, &pending.paths),
            path_count = pending.paths.len(),
            formats = ?crate::file_extensions(&pending.paths),
            source = pending.source,
            load_ms = pending.started_at.elapsed().as_millis(),
            scene_id = pending.scene_key.id.get(),
            "scene load failed"
        );
    }

    fn ambiguous_units_notice(&self) -> Option<String> {
        let scene = self.document.scene.as_ref()?;
        let ambiguous = scene.meshes().iter().any(|entry| {
            entry.import_units().confidence == occluview_core::UnitConfidence::Ambiguous
        });
        if !ambiguous {
            return None;
        }
        let extent = scene.bbox().size().max_element();
        let suggestion = match occluview_formats::units::recommend_glb_scale(extent) {
            occluview_formats::units::GlbScaleRecommendation::MetersToMillimeters => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("load-units-suggest-meters")),
            occluview_formats::units::GlbScaleRecommendation::KeepAsMillimeters => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("load-units-suggest-millimeters")),
            occluview_formats::units::GlbScaleRecommendation::Unclear => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("load-units-unclear")),
        };
        Some(self.ui.locale.tr_with(
            crate::i18n::message_id!("load-units-ambiguous"),
            &[("suggestion", &suggestion)],
        ))
    }

    pub(super) fn open_paths_from_external_source(
        &mut self,
        paths: &[PathBuf],
        source: &'static str,
    ) {
        if self.document.scene.is_some() || self.loader.has_work_for(self.scene_key) {
            self.append_paths(paths, source);
        } else {
            self.replace_paths(paths, source);
        }
    }

    pub(super) fn handle_open_requests(&mut self, ctx: &egui::Context) {
        let mut handled_request = false;
        for request in self.platform.take_open_requests() {
            handled_request = true;
            self.platform.remember_raise_token(request.activation_token);
            self.open_paths_from_external_source(&request.paths, "single-instance");
        }
        if handled_request {
            self.raise_window_for_incoming_open(ctx);
        }
    }

    #[cfg(not(windows))]
    pub(super) fn schedule_linux_open_request_repaint(ctx: &egui::Context) {
        ctx.request_repaint_after(super::LINUX_OPEN_REQUEST_REPAINT_INTERVAL);
    }

    #[cfg(windows)]
    pub(super) fn schedule_linux_open_request_repaint(_ctx: &egui::Context) {}

    pub(super) fn raise_window_for_startup_open(&mut self, ctx: &egui::Context) {
        let token = self.platform.take_raise_token();
        let activated = self.platform.raise_target.try_activate(token.as_deref());
        single_instance::complete_startup_notification(token.as_deref());
        if activated {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(super) fn raise_window_for_incoming_open(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        let token = self.platform.pending_raise_token.clone();
        if self.platform.raise_target.try_activate(token.as_deref()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::viewport::WindowLevel::AlwaysOnTop,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));
        self.ui.foreground_pulse_until = Some(Instant::now() + FOREGROUND_PULSE_DURATION);
        ctx.request_repaint_after(FOREGROUND_PULSE_DURATION);
    }

    pub(super) fn finish_foreground_pulse_if_due(&mut self, ctx: &egui::Context) {
        let Some(until) = self.ui.foreground_pulse_until else {
            return;
        };
        if Instant::now() < until {
            ctx.request_repaint_after(until.saturating_duration_since(Instant::now()));
            return;
        }
        self.ui.foreground_pulse_until = None;
        self.platform.pending_raise_token = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::viewport::WindowLevel::Normal,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Reset,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{failure_without_paths, native_drop_paths, resulting_workspace_memory_error};
    use crate::scene_loading::SceneLoadMode;
    use eframe::egui;
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
    };

    #[derive(Debug)]
    struct NativeDroppedFile {
        path: PathBuf,
    }

    impl egui::DroppedFile for NativeDroppedFile {
        fn path(&self) -> &Path {
            &self.path
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("native path routing must not read dropped-file bytes".to_owned())
        }
    }

    fn native_drop(path: PathBuf) -> egui::DroppedFileHandle {
        Arc::new(NativeDroppedFile { path })
    }

    #[test]
    fn native_drop_paths_preserve_backend_order() {
        let first = PathBuf::from("/cases/first.stl");
        let second = PathBuf::from("/cases/second.obj");
        let dropped = vec![native_drop(first.clone()), native_drop(second.clone())];

        assert_eq!(native_drop_paths(&dropped), vec![first, second]);
    }

    #[test]
    fn commit_budget_uses_current_workspace_bytes_and_respects_replace_semantics() {
        let limit = occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES;
        let replaced_scene = 96;
        let imported_scene = 160;

        let retained_when_load_started = limit - 128;
        assert!(resulting_workspace_memory_error(
            retained_when_load_started,
            replaced_scene,
            imported_scene,
            SceneLoadMode::Replace,
        )
        .is_none());
        assert!(matches!(
            resulting_workspace_memory_error(
                limit - 128,
                replaced_scene,
                imported_scene,
                SceneLoadMode::Append,
            ),
            Some(occluview_formats::FormatError::MemoryBudgetExceeded { .. })
        ));

        // A neighboring scene or shared history can grow after decoding starts.
        // The freshly measured workspace total must reject the commit while the
        // earlier total would still have fit.
        assert!(matches!(
            resulting_workspace_memory_error(
                limit - 8,
                replaced_scene,
                imported_scene,
                SceneLoadMode::Replace,
            ),
            Some(occluview_formats::FormatError::MemoryBudgetExceeded {
                estimated_bytes,
                limit: error_limit,
            }) if estimated_bytes == limit + 56 && error_limit == limit
        ));
    }

    #[test]
    fn a_failed_load_reaches_the_log_without_the_path_it_failed_on() {
        // This text lands in the crash log ring, and the ring is written to a
        // file operators are asked to attach to public issues.
        let path = PathBuf::from("/mnt/cases/Ivanov 2026-08-23/upper.stl");
        let error = anyhow::anyhow!("{}: {}", path.display(), "unexpected end of file");

        let logged = failure_without_paths(&error, std::slice::from_ref(&path));

        assert!(
            !logged.contains("Ivanov"),
            "the case name must not survive into the log: {logged}"
        );
        assert!(
            !logged.contains("/mnt/cases"),
            "no part of the path may survive: {logged}"
        );
        assert!(
            logged.contains("unexpected end of file"),
            "the line must keep the failure reason: {logged}"
        );
        assert!(
            logged.contains("<stl>"),
            "the format is what a reader needs instead of the name: {logged}"
        );
    }

    #[test]
    fn a_path_that_prefixes_another_does_not_leave_its_tail_behind() {
        // The prefix case from `failure_without_paths`: replace in request
        // order and the basename of the longer path survives, which is the
        // half that names the case.
        let folder = PathBuf::from("/mnt/cases/Ivanov 2026");
        let scan = folder.join("upper.stl");
        let error = anyhow::anyhow!("{}: {}", scan.display(), "unexpected end of file");

        let logged = failure_without_paths(&error, &[folder, scan]);

        assert!(!logged.contains("Ivanov"), "{logged}");
        assert!(!logged.contains("upper"), "{logged}");
        assert!(logged.contains("unexpected end of file"), "{logged}");
    }

    #[test]
    fn an_extension_that_is_really_part_of_the_name_is_not_echoed_back() {
        let path = PathBuf::from("/mnt/cases/scan.Ivanov");
        let error = anyhow::anyhow!("{}: {}", path.display(), "unsupported format");

        let logged = failure_without_paths(&error, std::slice::from_ref(&path));

        assert!(!logged.to_lowercase().contains("ivanov"), "{logged}");
        assert!(logged.contains("<file>"), "{logged}");
    }

    #[test]
    fn a_file_with_no_extension_is_still_redacted() {
        let path = PathBuf::from("/mnt/cases/Ivanov/scan");
        let error = anyhow::anyhow!("{}: {}", path.display(), "unsupported format");

        let logged = failure_without_paths(&error, std::slice::from_ref(&path));

        assert!(!logged.contains("Ivanov"), "{logged}");
        assert!(logged.contains("<file>"), "{logged}");
    }

    #[test]
    fn incoming_open_state_prefers_append_when_scene_or_load_exists() {
        assert!(
            !crate::should_append_incoming_open_state(false, false, 0),
            "empty app state should replace the scene on external open"
        );
        assert!(
            crate::should_append_incoming_open_state(true, false, 0),
            "an existing scene should append new external opens"
        );
        assert!(
            crate::should_append_incoming_open_state(false, true, 0),
            "an active background load should append new external opens"
        );
        assert!(
            crate::should_append_incoming_open_state(false, false, 2),
            "queued loads should append new external opens"
        );
    }
}
