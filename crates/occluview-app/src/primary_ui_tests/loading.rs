use super::*;

#[test]
fn startup_paths_are_loaded_after_window_creation() {
    let bootstrap = app_bootstrap_source();
    let app_source = app_module_source();

    assert!(
        bootstrap.contains("args.files.clone(),\n                live_viewport"),
        "startup file paths should be passed into the app, not preloaded before run_native"
    );
    assert!(
        app_source.contains("app.replace_paths(&startup_paths, \"startup\")"),
        "app startup should schedule a background load"
    );
    assert!(
        !bootstrap.contains("load_scene(&args.files).context(\"loading startup scene\")"),
        "startup loading must not block window creation"
    );
}

#[test]
fn app_starts_idle_until_scene_data_arrives() {
    let new_fn = function_source(app_module_source(), "pub(crate) fn new(");

    assert!(
        new_fn.contains("needs_render: false,"),
        "empty startup should not spend time rendering a nonexistent scene"
    );
}

#[test]
fn load_and_first_frame_paths_log_timing_boundaries() {
    let loading_source = app_loading_source();
    let app_render = app_render_source();
    let scene_loading_source = include_str!("../scene_loading.rs");
    let live_viewport = include_str!("../live_viewport.rs");

    assert!(
        scene_loading_source.contains("started_at: Instant"),
        "pending scene loads should track their wall-clock start"
    );
    assert!(
        loading_source.contains("scene_ready_ms = pending.started_at.elapsed().as_millis()"),
        "scene parse/decrypt completion should log its duration"
    );
    assert!(
        app_render.contains("render_ms = render_started_at.elapsed().as_millis()"),
        "first viewport frame timing should be observable in logs"
    );
    assert!(
        app_render.contains("\"offscreen viewport scene prepared\""),
        "offscreen fallback should log scene upload timing separately from frame render timing"
    );
    assert!(
        live_viewport.contains("\"live viewport scene prepared\""),
        "live viewport should log scene upload timing separately from scene parse timing"
    );
}

#[test]
fn primary_startup_only_refreshes_shell_associations_on_explicit_request() {
    let source = app_bootstrap_source();
    let real_main_start = source.rfind("fn real_main() -> Result<()> {");
    let install_hook_start = source.rfind("fn install_panic_hook()");
    assert!(real_main_start.is_some(), "missing real_main");
    assert!(
        install_hook_start.is_some(),
        "missing install_panic_hook after real_main"
    );
    let Some(real_main_start) = real_main_start else {
        return;
    };
    let Some(install_hook_start) = install_hook_start else {
        return;
    };
    let real_main = &source[real_main_start..install_hook_start];
    let shell_refresh_branch = real_main.find("if args.shell_refresh {");
    assert!(
        shell_refresh_branch.is_some(),
        "missing explicit --shell-refresh branch"
    );
    let Some(shell_refresh_branch) = shell_refresh_branch else {
        return;
    };

    assert!(
        real_main[shell_refresh_branch..]
            .contains("occluview_shell::notify_shell_associations_changed();"),
        "--shell-refresh should still notify Explorer"
    );
    assert_eq!(
        real_main
            .matches("occluview_shell::notify_shell_associations_changed();")
            .count(),
        1,
        "normal app startup should not pay a shell-association refresh tax on every launch"
    );
}

#[test]
fn incoming_files_append_while_scene_or_load_is_active() {
    let source = app_loading_source();
    let shared_logic = main_source();

    assert!(
        source.contains("fn should_append_incoming_open(&self) -> bool"),
        "incoming open requests need a shared append decision"
    );
    assert!(
        source.contains("crate::should_append_incoming_open_state("),
        "incoming open requests should reuse the shared append helper"
    );
    assert!(
        shared_logic.contains("has_scene || has_active_load || queued_load_count != 0"),
        "incoming files should append when a scene load is still pending"
    );
}

#[test]
fn incoming_files_raise_existing_window_temporarily() {
    let loading_source = app_loading_source();

    assert!(
        loading_source.contains("fn raise_window_for_incoming_open_impl"),
        "single-instance handoff should explicitly raise the existing viewer window"
    );
    assert!(
        loading_source.contains("egui::ViewportCommand::Minimized(false)")
            && loading_source.contains("egui::ViewportCommand::Visible(true)")
            && loading_source.contains("egui::ViewportCommand::Focus"),
        "incoming file opens should restore and focus the existing window"
    );
    assert!(
        loading_source.contains("egui::viewport::WindowLevel::AlwaysOnTop")
            && loading_source.contains("egui::viewport::WindowLevel::Normal"),
        "foreground pulse should not leave the viewer permanently topmost"
    );
    assert!(
        loading_source.contains("egui::UserAttentionType::Informational")
            && loading_source.contains("egui::UserAttentionType::Reset"),
        "taskbar attention should be requested for a background handoff and reset after the pulse"
    );
    assert!(
        loading_source.contains("self.raise_target.try_activate("),
        "the handoff should first attempt a real WM activation (X11) before the \
         focus-stealing-prevention-limited fallback"
    );
}

#[test]
fn incoming_files_use_event_driven_ipc_instead_of_250ms_polling() {
    let app_source = app_module_source();
    let single_instance_windows = include_str!("../single_instance/windows.rs");
    let single_instance_fallback = include_str!("../single_instance/fallback.rs");

    assert!(
        app_source.contains("single_instance::OpenRequestListener::spawn"),
        "primary instance should start a dedicated open-request listener instead of passive polling"
    );
    assert!(
        app_source.contains("incoming_open_requests: single_instance::OpenRequestListener"),
        "app state should own an explicit incoming-open listener"
    );
    assert!(
        !app_source.contains("ctx.request_repaint_after(Duration::from_millis(250));"),
        "single-instance file opens should not wait on a 250ms repaint heartbeat"
    );
    assert!(
        single_instance_windows.contains("PIPE_NAME"),
        "single-instance handoff should have a dedicated IPC endpoint"
    );
    assert!(
        single_instance_windows.contains("CreateNamedPipeW")
            && single_instance_windows.contains("ConnectNamedPipe")
            && single_instance_windows.contains("WaitNamedPipeW"),
        "single-instance handoff should use named-pipe IPC for low-latency delivery"
    );
    assert!(
        single_instance_windows.contains("send_pipe_open_request")
            && single_instance_fallback.contains("write_disk_open_request"),
        "pipe handoff should keep a disk fallback for robustness"
    );
}

#[test]
fn linux_file_handoff_has_repaint_fallback_independent_of_focus() {
    let app_source = app_module_source();
    let app_state = include_str!("../app/state.rs");
    let single_instance = include_str!("../single_instance/mod.rs");
    let single_instance_unix = include_str!("../single_instance/unix.rs");
    let single_instance_fallback = include_str!("../single_instance/fallback.rs");

    assert!(
        app_source.contains("LINUX_OPEN_REQUEST_REPAINT_INTERVAL"),
        "Linux needs a lightweight UI-thread wake fallback because some file-manager handoffs do not wake winit until focus changes"
    );
    assert!(
        app_state.contains("Self::schedule_linux_open_request_repaint(ctx);"),
        "Linux open-request processing should not depend only on background-thread request_repaint"
    );
    assert!(
        app_state
            .contains("ctx.request_repaint_after(super::LINUX_OPEN_REQUEST_REPAINT_INTERVAL);"),
        "Linux fallback must keep the UI loop checking handoff requests while the viewer is open"
    );
    assert!(
        single_instance.contains("unix::send_socket_open_request(request).is_ok()")
            && single_instance.contains("fallback::write_disk_open_request(request)")
            && single_instance_unix.contains("fn send_socket_open_request")
            && single_instance_fallback.contains("fn write_disk_open_request"),
        "Linux handoff should have both Unix socket delivery and disk fallback"
    );
}

#[test]
fn linux_socket_handoff_repaints_until_background_window_consumes_request() {
    let single_instance = include_str!("../single_instance/mod.rs");
    let single_instance_unix = include_str!("../single_instance/unix.rs");
    let single_instance_fallback = include_str!("../single_instance/fallback.rs");

    assert!(
        single_instance.contains("LINUX_OPEN_REQUEST_WAKE_BURST_STEPS")
            && single_instance.contains("fn request_open_handoff_repaint("),
        "Linux handoff wakeups should not rely on a single background-thread repaint request"
    );
    assert!(
        single_instance_unix.contains("request_open_handoff_repaint(&repaint_ctx);"),
        "Unix socket handoff should issue a short repaint burst after delivering paths"
    );
    assert!(
        single_instance_fallback.contains("request_open_handoff_repaint(&repaint_ctx);"),
        "disk fallback handoff should issue the same repaint burst after delivering paths"
    );
}

#[test]
fn single_instance_load_raises_window_after_scene_is_ready() {
    let loading_source = app_loading_source();
    let app_source = app_module_source();
    let update = function_source(
        app_source,
        "fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {",
    );

    assert!(
        loading_source.contains("pub(super) fn process_scene_loads(&mut self, ctx: &egui::Context)"),
        "scene load completion needs egui context so a background single-instance open can wake the existing window after data is ready"
    );
    assert!(
        update.contains("self.process_scene_loads(ctx);"),
        "the update loop should pass context into scene-load completion"
    );
    assert!(
        loading_source.contains("let load_settled = result.is_ok() && self.queued_loads.is_empty();")
            && loading_source.contains("active.source == \"single-instance\" && load_settled")
            && loading_source.contains("if raise_after_handoff {")
            && loading_source.contains("self.raise_window_for_incoming_open(ctx);"),
        "single-instance opens should repeat the foreground/attention request after the appended scene is visible"
    );
    assert!(
        loading_source.contains("active.source == \"startup\" && load_settled")
            && loading_source.contains("self.raise_window_for_startup_open(ctx);")
            && function_source(
                loading_source,
                "pub(super) fn raise_window_for_startup_open("
            )
            .contains("try_activate")
            && !function_source(
                loading_source,
                "pub(super) fn raise_window_for_startup_open("
            )
            .contains("RequestUserAttention"),
        "first launch claims focus quietly with the launcher token; no attention-pulse \
         fallback that would recreate the 'window is ready' noise"
    );
}

#[test]
fn replace_open_is_guarded_when_a_session_is_dirty_or_unsaved() {
    let loading = app_loading_source();

    assert!(
        loading.contains("fn replace_open_needs_guard(&self) -> bool")
            && loading.contains("self.edit_mode.is_dirty() || self.has_unsaved_mesh_edits"),
        "a replace open must be gated on a live dirty session OR unsaved edits, \
         not proceed straight to a scene-destroying load"
    );
    assert!(
        loading.contains("self.pending_replace_open = Some(PendingReplaceOpen {"),
        "a guarded replace open must be parked, not silently dropped or applied"
    );
    assert!(
        loading.contains("fn replace_paths_confirmed(")
            && loading.contains("SceneLoadMode::Replace"),
        "the confirmed path must be able to start the replace once the operator answers"
    );
}

#[test]
fn replace_guard_dialog_offers_save_discard_and_cancel() {
    let dialogs = app_dialogs_source();

    assert!(
        dialogs.contains("fn guard_pending_replace_open(&mut self, ctx: &egui::Context)"),
        "the parked replace open needs a guard dialog"
    );
    assert!(
        dialogs.contains("An edit session is active on {layer}."),
        "the guard must name the layer whose session is at stake"
    );
    assert!(
        dialogs.contains("\"Save…\"")
            && dialogs.contains("\"Discard and open\"")
            && dialogs.contains("\"Cancel\""),
        "the guard must offer Save, Discard-and-open, and Cancel"
    );
    assert!(
        dialogs.contains("self.replace_paths_confirmed(&pending.paths, pending.source)"),
        "answering Save/Discard must apply the exact parked open"
    );
    assert!(
        dialogs.contains("SaveEditedLayersOutcome::Aborted => {}"),
        "a cancelled export or failed write must keep the open parked, never open on top of \
         edits the operator believes are saved"
    );
}

#[test]
fn replace_guard_suppresses_edit_shortcuts_and_runs_each_frame() {
    let app_source = app_module_source();

    assert!(
        app_source.contains("|| self.pending_replace_open.is_some()"),
        "edit hotkeys must not act behind the open-guard dialog"
    );
    assert!(
        app_source.contains("self.guard_pending_replace_open(ctx);"),
        "the update loop must drive the parked-open guard dialog"
    );
}

#[test]
fn append_scene_load_preserves_existing_camera() {
    let app_source = app_loading_source();

    assert!(
        app_source.contains("self.set_scene(scene, reset_camera)"),
        "scene load completion should make the camera reset decision explicitly"
    );
    assert!(
        app_source.contains("if append {")
            && app_source.contains("LoadQueueCameraReset::WhenQueueDrains")
            && app_source.contains("&& !queued_after_current")
            && app_source.contains("!self.camera_modified_during_load"),
        "queued append loads must not re-home after the user has already moved the camera"
    );
    assert!(
        !app_source
            .contains("self.set_scene(scene, true);\n                    self.current_paths"),
        "scene load completion should not always reset the camera"
    );
    assert!(
        !app_source.contains("self.set_scene(scene, !append)"),
        "append camera behavior should account for queued single-instance bursts, not only append mode"
    );
}

#[test]
fn queued_open_burst_frames_final_combined_scene_once() {
    let app_source = app_loading_source();

    assert!(
        app_source.contains("load_queue_camera_reset"),
        "multi-file single-instance bursts need an explicit deferred camera reset flag"
    );
    assert!(
        app_source.contains("camera_modified_during_load"),
        "multi-file bursts need to distinguish automatic framing from user camera movement"
    );
    assert!(
        app_source.contains("let queued_after_current = !self.queued_loads.is_empty();"),
        "camera reset should be deferred while more files from the open burst are queued"
    );
    assert!(
        app_source.contains("self.camera.is_none()"),
        "the first loaded layer should still get a camera even when more queued files are pending"
    );
    assert!(
        app_source.contains("self.needs_render = false;")
            && app_source.contains("self.rendered = None;")
            && app_source.contains("self.clear_live_viewport();"),
        "intermediate burst loads should not render or publish a half-framed scene"
    );
    assert!(
        app_source.contains("} else if self.load_queue_camera_reset")
            && app_source.contains("&& self.queued_loads.is_empty()"),
        "a final append failure should still frame the successfully loaded partial scene"
    );
}
