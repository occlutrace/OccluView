//! Getting files into the scene without losing work already in it.
//!
//! Loads run off the UI thread and report back over a channel; `queued_loads`
//! holds the ones that arrived while another was in flight, so a multi-file
//! drop or a burst of shell hand-offs is serialised rather than raced.
//!
//! A load is either REPLACE (menu Open, a recent file, a hand-off classified
//! as replace) or APPEND (a second scan added to the current scene). Every
//! replace goes through the guard here first: if the session has unsaved mesh
//! edits, the open is parked behind the confirmation dialog instead of
//! destroying them. Append needs no guard, and neither do the camera rules --
//! a replace re-frames the scene, an append keeps the operator's camera unless
//! they had not moved it.

use super::{
    combine_loaded_scene, egui, load_error_dialog, load_status_message, mpsc,
    read_files_with_key_provider, single_instance, AppErrorDialog, Instant, LoadQueueCameraReset,
    OccluViewApp, PathBuf, PendingReplaceOpen, PendingSceneLoad, Result, RuntimeHpsKeyProvider,
    Scene, SceneLoadMode, SceneLoadRequest, TryRecvError, FOREGROUND_PULSE_DURATION,
};

/// Load one or more mesh files into a scene.
pub(super) fn load_scene(paths: &[PathBuf]) -> Result<Scene> {
    read_files_with_key_provider(paths, &RuntimeHpsKeyProvider)
        .map_err(|(path, e)| anyhow::anyhow!("{}: {}", path.display(), e))
}

impl OccluViewApp {
    /// Guarded entry for every REPLACE open (menu Open, recent, drop/handoff
    /// classified as replace). If a live session is dirty or unsaved edits
    /// exist, the open is parked behind the edit-session guard dialog instead of
    /// silently destroying the session; otherwise it starts immediately.
    pub(super) fn replace_paths(&mut self, paths: &[PathBuf], source: &'static str) {
        if paths.is_empty() {
            return;
        }
        if self.replace_open_needs_guard() {
            // Newest replace supersedes an older parked one; the open is held,
            // never dropped, until the operator answers the dialog.
            self.pending_replace_open = Some(PendingReplaceOpen {
                paths: paths.to_vec(),
                source,
            });
            return;
        }
        self.load_paths_with_mode(paths, source, SceneLoadMode::Replace);
    }

    /// Start a replace open the operator confirmed at the guard dialog (or that
    /// never needed guarding). The live session is intentionally NOT torn down
    /// here: the load's success path replaces the scene (and clears the
    /// session); a load that fails leaves the session and its edits intact.
    pub(super) fn replace_paths_confirmed(&mut self, paths: &[PathBuf], source: &'static str) {
        if paths.is_empty() {
            return;
        }
        self.load_paths_with_mode(paths, source, SceneLoadMode::Replace);
    }

    /// Whether an incoming replace must be parked behind the edit-session guard:
    /// a live session carrying uncommitted edits, or any layer with edits not
    /// yet written to disk, would be lost by a blind scene replace.
    fn replace_open_needs_guard(&self) -> bool {
        self.edit_mode.is_dirty() || self.has_unsaved_mesh_edits()
    }

    pub(super) fn append_paths(&mut self, paths: &[PathBuf], source: &'static str) {
        self.load_paths_with_mode(paths, source, SceneLoadMode::Append);
    }

    fn load_paths_with_mode(
        &mut self,
        paths: &[PathBuf],
        source: &'static str,
        mode: SceneLoadMode,
    ) {
        if paths.is_empty() {
            return;
        }
        let request = SceneLoadRequest {
            paths: paths.to_vec(),
            source,
            mode,
        };
        if self.active_load.is_some() {
            if mode == SceneLoadMode::Replace {
                self.queued_loads.clear();
                self.active_load = None;
                self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
                self.camera_modified_during_load = false;
                self.start_scene_load(request);
            } else {
                self.queued_loads.push_back(request);
                self.status_message = Some(format!(
                    "Queued {} layer{}",
                    paths.len(),
                    if paths.len() == 1 { "" } else { "s" }
                ));
            }
            return;
        }
        self.start_scene_load(request);
    }

    fn start_scene_load(&mut self, request: SceneLoadRequest) {
        let SceneLoadRequest {
            paths,
            source,
            mode,
        } = request;
        let load_paths = paths.clone();
        let started_at = Instant::now();
        let (sender, receiver) = mpsc::channel();
        let repaint_ctx = self.repaint_ctx.clone();
        let spawn_result = std::thread::Builder::new()
            .name("scene-load".to_string())
            .spawn(move || {
                let result = load_scene(&load_paths);
                let _ = sender.send(result);
                repaint_ctx.request_repaint();
            });
        if let Err(error) = spawn_result {
            self.status_message = Some("Open failed: could not start loader".to_string());
            self.app_error = Some(AppErrorDialog {
                title: "Could not open file".to_string(),
                summary: "The background scene loader could not be started.".to_string(),
                details: format!("Loader thread start failed\n\n{error:#}"),
            });
            tracing::error!(?error, source, "scene loader thread spawn failed");
            return;
        }
        self.status_message = Some(load_status_message(mode, paths.len()));
        self.active_load = Some(PendingSceneLoad {
            paths,
            source,
            mode,
            started_at,
            receiver,
        });
    }

    pub(super) fn process_scene_loads(&mut self, ctx: &egui::Context) {
        let Some(active) = self.active_load.as_ref() else {
            self.start_next_queued_load();
            return;
        };
        let result = match active.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                let source = active.source;
                let startup_token = self.pending_raise_token.take();
                self.active_load = None;
                self.status_message = Some("Open failed: loader stopped".to_string());
                tracing::error!(source, "scene loader disconnected");
                if source == "startup" || source == "single-instance" {
                    single_instance::complete_startup_notification(startup_token.as_deref());
                }
                self.start_next_queued_load();
                return;
            }
        };

        let Some(active) = self.active_load.take() else {
            return;
        };
        let load_settled = self.queued_loads.is_empty();
        let raise_after_handoff = active.source == "single-instance" && load_settled;
        let raise_after_startup = active.source == "startup" && load_settled;
        self.apply_scene_load_result(active, result);
        if raise_after_handoff {
            // Second (definitive) raise now that the window has fresh content.
            let startup_token = self.pending_raise_token.clone();
            self.raise_window_for_incoming_open(ctx);
            single_instance::complete_startup_notification(startup_token.as_deref());
            // The handoff is complete; drop its provenance token.
            self.pending_raise_token = None;
        } else if raise_after_startup {
            self.raise_window_for_startup_open(ctx);
        }
        self.start_next_queued_load();
    }

    fn start_next_queued_load(&mut self) {
        if self.active_load.is_none() {
            if let Some(request) = self.queued_loads.pop_front() {
                self.start_scene_load(request);
            }
        }
    }

    fn apply_scene_load_result(&mut self, pending: PendingSceneLoad, result: Result<Scene>) {
        let append = pending.mode == SceneLoadMode::Append;
        match result {
            Ok(scene) => {
                let scene_ready_ms = pending.started_at.elapsed().as_millis();
                let (scene, current_paths) = if append {
                    combine_loaded_scene(
                        self.scene.as_deref(),
                        &self.current_paths,
                        scene,
                        &pending.paths,
                    )
                } else {
                    (scene, pending.paths.clone())
                };
                let recent_paths = current_paths.clone();
                let queued_after_current = !self.queued_loads.is_empty();
                if !append {
                    self.edit_mode.clear();
                    // The old scene (and any unsaved edits on it) is gone.
                    self.clear_unsaved_mesh_edits();
                    self.hidden_layer_stack.clear();
                    self.translucent_layer_restore.clear();
                }
                let reset_camera = if append {
                    let reset = self.load_queue_camera_reset
                        == LoadQueueCameraReset::WhenQueueDrains
                        && !queued_after_current
                        && !self.camera_modified_during_load;
                    if reset {
                        self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
                    }
                    reset
                } else if queued_after_current {
                    self.load_queue_camera_reset = LoadQueueCameraReset::WhenQueueDrains;
                    self.camera.is_none()
                } else {
                    self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
                    true
                };
                self.set_scene(scene, reset_camera);
                if self.load_queue_camera_reset == LoadQueueCameraReset::WhenQueueDrains
                    && queued_after_current
                {
                    self.needs_render = false;
                    self.rendered = None;
                    self.clear_live_viewport();
                }
                self.current_paths = current_paths;
                self.push_recent_scene(&recent_paths);
                self.save_recent_files();
                self.status_message = None;
                tracing::info!(
                    source = pending.source,
                    append,
                    path_count = pending.paths.len(),
                    scene_ready_ms,
                    "scene load completed"
                );
            }
            Err(e) => {
                let action = if append { "Add" } else { "Open" };
                if !append {
                    self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
                } else if self.load_queue_camera_reset == LoadQueueCameraReset::WhenQueueDrains
                    && self.queued_loads.is_empty()
                {
                    self.load_queue_camera_reset = LoadQueueCameraReset::Idle;
                    if self.scene.is_some() {
                        self.reset_camera_to_home();
                    }
                }
                self.status_message = Some(format!("{action} failed: {e:#}"));
                self.app_error = Some(load_error_dialog(action, &e, &pending.paths));
                tracing::error!(
                    error = ?e,
                    paths = ?pending.paths,
                    source = pending.source,
                    load_ms = pending.started_at.elapsed().as_millis(),
                    "scene load failed"
                );
            }
        }
    }

    fn should_append_incoming_open(&self) -> bool {
        crate::should_append_incoming_open_state(
            self.scene.is_some(),
            self.active_load.is_some(),
            self.queued_loads.len(),
        )
    }

    pub(super) fn open_paths_from_external_source(
        &mut self,
        paths: &[PathBuf],
        source: &'static str,
    ) {
        if self.should_append_incoming_open() {
            self.append_paths(paths, source);
        } else {
            self.replace_paths(paths, source);
        }
    }

    pub(super) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            let paths: Vec<PathBuf> = i
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect();
            if !paths.is_empty() {
                self.open_paths_from_external_source(&paths, "drop");
            }
        });
    }

    pub(super) fn handle_open_requests_impl(&mut self, ctx: &egui::Context) {
        let mut handled_request = false;
        for request in self.incoming_open_requests.take_requests() {
            handled_request = true;
            // Keep the most recent forwarded activation token; it is the
            // provenance the post-load raise uses. See activation.rs.
            if request.activation_token.is_some() {
                self.pending_raise_token = request.activation_token;
            }
            self.open_paths_from_external_source(&request.paths, "single-instance");
        }
        if handled_request {
            // First raise attempt, right after queueing the load. The load
            // finishing triggers a second attempt (`raise_after_load` in
            // `process_scene_loads`) once the window has fresh content to show.
            self.raise_window_for_incoming_open(ctx);
        }
    }

    /// Quiet startup raise: the first launch claims focus with the launcher's
    /// activation token through X11 or Wayland. Without a token, the normal
    /// compositor/window-manager fallback remains the only policy-compliant
    /// option for a launch that did not originate from a user action.
    pub(super) fn raise_window_for_startup_open(&mut self, ctx: &egui::Context) {
        let token = self.pending_raise_token.take();
        let activated = self.raise_target.try_activate(token.as_deref());
        single_instance::complete_startup_notification(token.as_deref());
        if activated {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        // Keep the first launch visible and unminimized even when the process
        // was started without a desktop activation token (for example from a
        // terminal). A compositor may still refuse focus by policy, but this
        // does not add a fake always-on-top or attention pulse.
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(super) fn raise_window_for_incoming_open_impl(&mut self, ctx: &egui::Context) {
        // Always make sure the window is mapped and un-minimized first; these
        // are not "activation" and are honored by every WM.
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));

        // Preferred path: send the compositor-specific activation request with
        // the forwarded user-interaction provenance. winit's own `Focus` is not
        // enough for a live Wayland surface and can be rejected as a focus
        // steal on X11.
        let token = self.pending_raise_token.clone();
        if self.raise_target.try_activate(token.as_deref()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }

        // Fallback (missing token/handle or a failed compositor request): make
        // the pending file visible and request attention without leaving the
        // viewer permanently topmost.
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::viewport::WindowLevel::AlwaysOnTop,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));
        self.foreground_pulse_until = Some(Instant::now() + FOREGROUND_PULSE_DURATION);
        ctx.request_repaint_after(FOREGROUND_PULSE_DURATION);
    }

    pub(super) fn finish_foreground_pulse_if_due_impl(&mut self, ctx: &egui::Context) {
        let Some(until) = self.foreground_pulse_until else {
            return;
        };
        if Instant::now() < until {
            ctx.request_repaint_after(until.saturating_duration_since(Instant::now()));
            return;
        }
        self.foreground_pulse_until = None;
        // The attention pulse (fallback path) has ended; drop any provenance
        // token still held from a load that failed before its post-load raise.
        self.pending_raise_token = None;
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
