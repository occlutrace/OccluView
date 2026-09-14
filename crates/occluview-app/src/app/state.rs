//! `OccluViewApp` itself: the root coordinator over owned state domains.
//!
//! Extracted domains live in their own modules with their invariants (see
//! `state_render`, `state_document`, `state_persistence`); the root only
//! orchestrates transitions across domains.
//!
//! State ownership by domain:
//!
//! - Document: extracted into [`DocumentState`].
//! - Render: extracted into [`RenderState`]. Call sites name a semantic
//!   invalidation cause; each render path consumes its own cursor, so
//!   camera-only redraws never touch uploaded geometry.
//! - Tools: extracted into [`ToolState`].
//! - UI: extracted into [`UiState`].
//! - Platform: extracted into [`PlatformState`].
//! - Persistence: extracted into [`PersistenceState`].
//!
use super::information_dialog::InformationDialog;
use super::state_document::DocumentState;
use super::state_persistence::PersistenceState;
use super::state_platform::{PlatformState, StartupHandles};
use super::state_render::RenderState;
use super::state_tool::ToolState;
use super::state_ui::UiState;
use super::{egui, home_camera_for_scene, CutTool, PathBuf};
use crate::live_viewport::SharedLiveViewport;

/// Global egui zoom changes the geometry of every widget. Keep it stable while
/// a pointer gesture is active so the UI Scale slider cannot move under the
/// pointer; the next frame after release applies the selected value.
fn ui_scale_zoom_is_allowed(ctx: &egui::Context) -> bool {
    !ctx.input(|input| input.pointer.any_down())
}

pub(crate) struct OccluViewApp {
    /// Dialogs, transient presentation, notifications; see `state_ui`.
    pub(super) ui: UiState,
    /// Renderer mirrors, caches, and invalidation cursors; see `state_render`.
    pub(super) render: RenderState,
    /// Scene content, selection, undo, and the load pipeline; see `state_document`.
    pub(super) document: DocumentState,
    /// Settings, paths, save/export coordination; see `state_persistence`.
    pub(super) persistence: PersistenceState,
    /// Tool controllers and cross-tool arbitration; see `state_tool`.
    pub(super) tools: ToolState,
    /// Native handles and single-instance handoff; see `state_platform`.
    pub(super) platform: PlatformState,
}

impl OccluViewApp {
    pub(crate) fn new(
        repaint_ctx: egui::Context,
        startup_paths: Vec<PathBuf>,
        live_viewport: Option<SharedLiveViewport>,
        startup: StartupHandles,
    ) -> Self {
        // UI scale is owned by settings, so egui's own keyboard zoom
        // (Cmd+=/Cmd+-) would fight the per-frame `set_zoom_factor` and blink.
        repaint_ctx.options_mut(|options| options.zoom_with_keyboard = false);
        // Single locale startup: sidecar preference → OS list → catalog.
        // Runtime lives in `UiState`, sidecar retry in `PersistenceState`.
        let state_dir = crate::app_paths::app_state_dir();
        let (locale, locale_snapshot) = crate::i18n::LocaleManager::startup(
            state_dir.as_deref(),
            &crate::i18n::os::SystemLocaleSource,
        );
        if let Some(diagnostic) = locale_snapshot.diagnostic {
            tracing::warn!(
                ?diagnostic,
                "language preference sidecar was unusable; using Auto/English"
            );
        }
        let mut app = Self {
            ui: UiState::new(repaint_ctx.clone(), locale),
            render: RenderState::new(live_viewport),
            document: DocumentState::new(),
            persistence: PersistenceState::new(),
            tools: ToolState::new(),
            platform: PlatformState::new(repaint_ctx.clone(), startup),
        };
        if app.persistence.settings.remember_sculpt_brush {
            crate::mesh_editor_overlay::set_sculpt_size(
                &app.ui.repaint_ctx,
                app.persistence.settings.sculpt_size,
            );
            crate::mesh_editor_overlay::set_sculpt_intensity(
                &app.ui.repaint_ctx,
                app.persistence.settings.sculpt_intensity,
            );
        }
        if !startup_paths.is_empty() {
            app.replace_paths(&startup_paths, "startup");
        }
        app
    }
    /// Edit hotkeys, refused while a dialog is up.
    ///
    /// The callee is named `_unguarded` rather than `_impl` because it is not
    /// the same thing: one plausible call from a neighbouring module deletes
    /// faces or replays an undo while the unsaved-changes prompt is open,
    /// quietly changing what "Save" then writes. A name nobody reaches for out
    /// of habit is the guard.
    pub(super) fn handle_edit_shortcuts(&mut self, ctx: &egui::Context) {
        // The bridge tool owns the scene while it is armed, which is not a
        // dialog and so is not part of the shared predicate.
        if self.ui.modal_dialog_open() || self.tools.bridge_split_active() {
            return;
        }
        self.handle_edit_shortcuts_unguarded(ctx);
    }

    pub(super) fn reset_camera_to_home(&mut self) {
        let Some(scene) = self.document.scene.as_ref() else {
            self.render.camera = None;
            return;
        };
        self.render.camera = Some(home_camera_for_scene(scene));
        self.render.invalidation.request_redraw();
    }

    pub(super) fn request_camera_repaint(&mut self, ctx: &egui::Context) {
        self.render.invalidation.request_redraw();
        self.document.mark_camera_modified();
        ctx.request_repaint();
    }

    /// A real app for a headless document-transition test.
    ///
    /// The production bootstrap acquires a process-wide single-instance claim,
    /// starts listener threads, and reads the operator's state directory. Tests
    /// that drive real transitions need the same types without those side
    /// effects; `app_test_support::test_app` sets the redirect and calls this.
    #[cfg(test)]
    pub(crate) fn new_for_tests(repaint_ctx: egui::Context) -> Self {
        Self {
            ui: UiState::new(repaint_ctx.clone(), crate::i18n::LocaleManager::for_tests()),
            render: RenderState::new(None),
            document: DocumentState::new(),
            persistence: PersistenceState::for_tests(),
            tools: ToolState::new(),
            platform: PlatformState::for_tests(repaint_ctx),
        }
    }

    pub(super) fn can_render_cut_view(&self) -> bool {
        self.document
            .scene
            .as_ref()
            .is_some_and(|scene| CutTool::can_render_bbox(scene.bbox()))
    }

    /// Whether any layer can take a measurement pick (visible triangles).
    pub(super) fn has_measurable_layer(&self) -> bool {
        self.document.scene.as_ref().is_some_and(|scene| {
            scene
                .meshes()
                .iter()
                .any(|entry| entry.visible && !entry.mesh.is_point_cloud())
        })
    }

    #[cfg(not(windows))]
    pub(super) fn schedule_linux_open_request_repaint(ctx: &egui::Context) {
        ctx.request_repaint_after(super::LINUX_OPEN_REQUEST_REPAINT_INTERVAL);
    }

    #[cfg(windows)]
    pub(super) fn schedule_linux_open_request_repaint(_ctx: &egui::Context) {}

    /// Render the information route that was active at the start of this UI
    /// pass. A selection inside About therefore replaces it on the following
    /// frame instead of briefly stacking two modal backdrops.
    pub(super) fn show_information_dialog(&mut self, ctx: &egui::Context) {
        if self.ui.foreground_dialog_open() {
            return;
        }
        match self.ui.information_dialog {
            InformationDialog::None => {}
            InformationDialog::About => self.show_about_dialog(ctx),
            InformationDialog::ThirdPartyNotices => self.show_third_party_window(ctx),
            InformationDialog::KeyboardMouse => self.show_help_dialog(ctx),
        }
    }
}

impl eframe::App for OccluViewApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui.sync_native_title(ctx);
        self.persistence.persist_settings_if_due(ctx);
        let preference = self.ui.locale.snapshot().preference.clone();
        self.persistence.persist_language_if_due(ctx, &preference);
        self.persistence.sync_sculpt_preferences(ctx);
        self.ui.expire_status_message(ctx);
        Self::schedule_linux_open_request_repaint(ctx);
        self.process_scene_loads(ctx);
        self.poll_sculpt_preparation(ctx);
        self.poll_sculpt_worker(ctx);
        // After the poll, and independently of it: a session can end without a
        // worker to poll, and the marker has to follow it either way.
        self.settle_sculpt_work_marker();
        self.handle_open_requests(ctx);
        self.finish_foreground_pulse_if_due(ctx);
        self.persistence.update_notice.poll(ctx);
        self.intercept_unsaved_close(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::ui_theme::set_active(self.persistence.settings.theme);
        ctx.set_visuals(super::viewer_visuals(self.persistence.settings.theme));
        // UI scale rides egui's zoom factor: a multiplier over the platform's
        // own pixel density, so HiDPI setups keep their native baseline.
        let target_ui_scale = self.persistence.settings.ui_scale();
        if ui_scale_zoom_is_allowed(&ctx) && (ctx.zoom_factor() - target_ui_scale).abs() > 1e-3 {
            ctx.set_zoom_factor(target_ui_scale);
        }
        self.handle_dropped_files(&ctx);
        self.release_viewport_orbit_cursor_if_inactive(&ctx);
        self.render_pending_frame(&ctx);
        self.handle_edit_shortcuts(&ctx);
        self.show_toolbar(ui);
        self.maybe_render_cut_view(&ctx);
        self.show_central_panel(ui);
        if self.ui.open_dialog_requested {
            self.ui.open_dialog_requested = false;
            self.open_files_dialog();
        }
        // Sync camera changes after viewport input so the live paint callback
        // uses the current frame's pose.
        self.render_pending_frame(&ctx);
        // Surface GPU faults before drawing the error dialog.
        if self.poll_gpu_errors() {
            // The fault was recorded by the previous submit. Request another
            // frame so the fail-closed callback and the operator-facing dialog
            // are both visible even when the normal repaint loop is idle.
            ctx.request_repaint();
        }
        self.show_error_dialog(&ctx);
        self.show_information_dialog(&ctx);
        self.ui.repair_report.ui(&ctx, &self.ui.locale);
        self.persistence.update_notice.show(&ctx, &self.ui.locale);
        self.show_unsaved_close_guard(&ctx);
        self.guard_pending_replace_open(&ctx);
    }
}
