//! `OccluViewApp` itself: the root coordinator over owned state domains.
//!
//! Extracted domains live in their own modules with their invariants (see
//! `state_render`, `state_document`, `state_persistence`); the root only
//! orchestrates transitions across domains.
//! The remaining flat fields are mapped to their intended owners below and
//! move slice by slice, never mechanically all at once.
//!
//! State ownership by domain:
//!
//! - Document: extracted into [`DocumentState`].
//! - Render: extracted into [`RenderState`]. Call sites name a semantic
//!   invalidation cause; each render path consumes its own cursor, so
//!   camera-only redraws never touch uploaded geometry.
//! - Tools (`cut_view`, `bridge_split*`, `measure`, `sculpt`, `align`,
//!   `edit_mode`, `editor_tab`): each tool owns its workflow; cross-tool
//!   arbitration lives in the overlay orchestration, not in the tools.
//! - UI (`status_message*`, `app_error`, dialogs, `open_dialogs`,
//!   `information_dialog`, panel transient flags): presentation only, renders
//!   worker/domain results into user-facing copy at the boundary.
//! - Platform (`_single_instance`, `incoming_open_requests`, `raise_target`,
//!   `pending_raise_token`, window handles): native handoff and activation.
//! - Persistence: extracted into [`PersistenceState`].
//!
use super::app_settings_panel::settings_popup_id;
use super::information_dialog::InformationDialog;
use super::open_dialogs::OpenDialogs;
use super::state_document::DocumentState;
use super::state_persistence::PersistenceState;
use super::state_render::RenderState;
use super::{egui, home_camera_for_scene, single_instance, CutTool, Duration, Instant, PathBuf};
use crate::live_viewport::SharedLiveViewport;

/// Global egui zoom changes the geometry of every widget. Keep it stable while
/// a pointer gesture is active so the UI Scale slider cannot move under the
/// pointer; the next frame after release applies the selected value.
fn ui_scale_zoom_is_allowed(ctx: &egui::Context) -> bool {
    !ctx.input(|input| input.pointer.any_down())
}

/// Everything the bootstrap hands the app about how this process was started:
/// the single-instance guard, the window raise handle, and the launcher's
/// activation token (focus provenance for the first load).
pub(crate) struct StartupHandles {
    pub(crate) single_instance: single_instance::SingleInstance,
    pub(crate) raise_target: single_instance::RaiseTarget,
    pub(crate) activation_token: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
pub(crate) struct OccluViewApp {
    pub(super) repaint_ctx: egui::Context,
    /// Renderer mirrors, caches, and invalidation cursors; see `state_render`.
    pub(super) render: RenderState,
    /// Scene content, selection, undo, and the load pipeline; see `state_document`.
    pub(super) document: DocumentState,
    /// Settings, paths, save/export coordination; see `state_persistence`.
    pub(super) persistence: PersistenceState,
    pub(super) status_message: Option<String>,
    pub(super) status_message_since: Option<Instant>,
    pub(super) status_message_snapshot: Option<String>,
    pub(super) app_error: Option<AppErrorDialog>,
    pub(super) cut_view: CutTool,
    /// Bridge-separator controller and its world-fixed placement disc. Kept
    /// separate from Cut View: one previews a structural mesh operation, the
    /// other only changes viewport clipping.
    pub(super) bridge_split: crate::bridge_split::BridgeSplitController,
    pub(super) bridge_split_disc: crate::cut_manipulator::CutManipulator,
    /// Passive Cut View panel driven by the Bridge Split disc. It owns no
    /// placement interaction, so the bridge tool remains the single pose owner.
    pub(super) bridge_split_section: crate::section_view::SectionView,
    /// Viewport measurement tools (ruler + wall-thickness probe). Mutually
    /// exclusive with `cut_view`; anchors are world-space and re-project every
    /// frame.
    pub(super) measure: crate::measure_tool::MeasureTool,
    pub(super) incoming_open_requests: single_instance::OpenRequestListener,
    pub(super) _single_instance: single_instance::SingleInstance,
    /// Raises the window on an open-file handoff through the native compositor
    /// activation protocol. See activation.rs.
    pub(super) raise_target: single_instance::RaiseTarget,
    /// Latest window-activation token forwarded by a second instance, used as
    /// provenance for the raise. Cleared once the raise's attention pulse ends.
    pub(super) pending_raise_token: Option<String>,
    pub(super) information_dialog: InformationDialog,
    /// Persistent post-repair report card, populated by the Repair executor and
    /// drawn in `ui()`; shows what a repair changed (or that nothing did).
    pub(super) repair_report: crate::repair_report::RepairReportDialog,
    pub(super) app_logo: Option<egui::TextureHandle>,
    pub(super) foreground_pulse_until: Option<Instant>,
    pub(super) viewport_orbit_cursor_grabbed: bool,
    /// Suppresses the stationary RMB context menu when the same press already
    /// moved the camera, including motion below egui's click/drag threshold.
    pub(super) viewport_secondary_gesture_moved_since_press: bool,
    /// Interactive sculpt-brush tool and active stroke state.
    pub(super) sculpt: crate::sculpt_tool::SculptTool,
    /// The Align Scans tool's whole state: tool, worker, settings, deviation
    /// display, markings, drag, brush and session poses. One struct so the app
    /// carries a single `align` field instead of eighteen loose ones.
    pub(super) align: crate::align_state::AlignState,
    /// Which mesh-editor tab is showing (selection/repair vs sculpt).
    pub(super) editor_tab: crate::mesh_editor_overlay::EditorTab,
    /// Layer count the automatic window-growth hint last reacted to. The hint
    /// fires only when this count changes and only ever grows the window, so a
    /// manual user resize is never fought frame by frame.
    pub(super) layers_window_layer_count: Option<usize>,
    /// The empty-viewport card was clicked: open the native Open dialog once,
    /// after the panel pass (the toolbar dispatches its dialog the same way).
    pub(super) open_dialog_requested: bool,
    /// The close-guard dialog is on screen.
    pub(super) close_guard_open: bool,
    /// The operator explicitly chose to close without saving.
    pub(super) close_confirmed: bool,
    /// A replace-scene request waiting for the unsaved-edit guard.
    pub(super) pending_replace_open: Option<PendingReplaceOpen>,
}

/// A replace-scene open request parked behind the unsaved-edit guard dialog.
#[derive(Clone)]
pub(super) struct PendingReplaceOpen {
    pub(super) paths: Vec<PathBuf>,
    pub(super) source: &'static str,
}

#[derive(Clone)]
pub(super) struct AppErrorDialog {
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) details: String,
}

fn information_route_is_blocked(
    close_guard_open: bool,
    pending_replace_open: bool,
    app_error_open: bool,
) -> bool {
    close_guard_open || pending_replace_open || app_error_open
}

impl OccluViewApp {
    /// Expire transient status text after the shared display interval.
    fn expire_status_message(&mut self, ctx: &egui::Context) {
        const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(4);
        let now = Instant::now();
        if self.status_message != self.status_message_snapshot {
            self.status_message_snapshot = self.status_message.clone();
            self.status_message_since = self.status_message.as_ref().map(|_| now);
        }
        let Some(since) = self.status_message_since else {
            return;
        };
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= STATUS_MESSAGE_TTL {
            self.status_message = None;
            self.status_message_snapshot = None;
            self.status_message_since = None;
        } else {
            ctx.request_repaint_after(STATUS_MESSAGE_TTL.saturating_sub(elapsed));
        }
    }

    pub(crate) fn new(
        repaint_ctx: egui::Context,
        startup_paths: Vec<PathBuf>,
        live_viewport: Option<SharedLiveViewport>,
        startup: StartupHandles,
    ) -> Self {
        // UI scale is owned by settings, so egui's own keyboard zoom
        // (Cmd+=/Cmd+-) would fight the per-frame `set_zoom_factor` and blink.
        repaint_ctx.options_mut(|options| options.zoom_with_keyboard = false);
        let mut app = Self {
            repaint_ctx: repaint_ctx.clone(),
            render: RenderState::new(live_viewport),
            document: DocumentState::new(),
            persistence: PersistenceState::new(),
            information_dialog: InformationDialog::default(),
            status_message: None,
            status_message_since: None,
            status_message_snapshot: None,
            app_error: None,
            cut_view: CutTool::default(),
            bridge_split: crate::bridge_split::BridgeSplitController::default(),
            bridge_split_disc: crate::cut_manipulator::CutManipulator::default(),
            bridge_split_section: crate::section_view::SectionView::default(),
            measure: crate::measure_tool::MeasureTool::default(),
            incoming_open_requests: single_instance::OpenRequestListener::spawn(repaint_ctx),
            _single_instance: startup.single_instance,
            raise_target: startup.raise_target,
            pending_raise_token: startup.activation_token,
            repair_report: crate::repair_report::RepairReportDialog::default(),
            app_logo: None,
            foreground_pulse_until: None,
            viewport_orbit_cursor_grabbed: false,
            viewport_secondary_gesture_moved_since_press: false,
            sculpt: crate::sculpt_tool::SculptTool::default(),
            align: crate::align_state::AlignState::default(),
            editor_tab: crate::mesh_editor_overlay::EditorTab::default(),
            layers_window_layer_count: None,
            open_dialog_requested: false,
            close_guard_open: false,
            close_confirmed: false,
            pending_replace_open: None,
        };
        if app.persistence.settings.remember_sculpt_brush {
            crate::mesh_editor_overlay::set_sculpt_size(
                &app.repaint_ctx,
                app.persistence.settings.sculpt_size,
            );
            crate::mesh_editor_overlay::set_sculpt_intensity(
                &app.repaint_ctx,
                app.persistence.settings.sculpt_intensity,
            );
        }
        if !startup_paths.is_empty() {
            app.replace_paths(&startup_paths, "startup");
        }
        app
    }

    /// Whether a modal dialog owns the keyboard.
    ///
    /// Escape belongs to the dialog in front of the operator, never to a tool
    /// behind it. Decided inline, that list drifts: the cut and align tools
    /// missed the replace-open guard and nobody counted the third-party
    /// licences window, so with either up Escape tore the tool down behind the
    /// dialog -- and for align also ran `cancel_align_session`, putting every
    /// scan back where it started. One predicate, so the next dialog gets
    /// remembered once.
    pub(super) fn modal_dialog_open(&self) -> bool {
        OpenDialogs {
            close_guard: self.close_guard_open,
            pending_replace: self.pending_replace_open.is_some(),
            error: self.app_error.is_some(),
            settings_popup: egui::Popup::is_id_open(&self.repaint_ctx, settings_popup_id()),
            information_dialog: self.information_dialog.is_open(),
        }
        .any()
    }

    /// A decision dialog takes precedence over informational content. Keep an
    /// open information route in state so it can return after the decision is
    /// resolved, but never let its modal consume Escape or clicks underneath.
    fn foreground_dialog_open(&self) -> bool {
        information_route_is_blocked(
            self.close_guard_open,
            self.pending_replace_open.is_some(),
            self.app_error.is_some(),
        )
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
        if self.modal_dialog_open() || self.bridge_split_active() {
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
        if self.foreground_dialog_open() {
            return;
        }
        match self.information_dialog {
            InformationDialog::None => {}
            InformationDialog::About => self.show_about_dialog(ctx),
            InformationDialog::ThirdPartyNotices => self.show_third_party_window(ctx),
            InformationDialog::KeyboardMouse => self.show_help_dialog(ctx),
        }
    }
}

impl eframe::App for OccluViewApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.persistence.persist_settings_if_due(ctx);
        self.persistence.sync_sculpt_preferences(ctx);
        self.expire_status_message(ctx);
        Self::schedule_linux_open_request_repaint(ctx);
        self.process_scene_loads(ctx);
        self.poll_sculpt_preparation(ctx);
        self.poll_sculpt_worker(ctx);
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
        if self.open_dialog_requested {
            self.open_dialog_requested = false;
            self.open_files_dialog();
        }
        // Sync camera changes after viewport input so the live paint callback
        // uses the current frame's pose.
        self.render_pending_frame(&ctx);
        // Surface GPU faults before drawing the error dialog.
        self.poll_gpu_errors();
        self.show_error_dialog(&ctx);
        self.show_information_dialog(&ctx);
        self.repair_report.ui(&ctx);
        self.persistence.update_notice.show(&ctx);
        self.show_unsaved_close_guard(&ctx);
        self.guard_pending_replace_open(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::information_route_is_blocked;

    #[test]
    fn information_route_yields_to_each_foreground_decision_dialog() {
        assert!(!information_route_is_blocked(false, false, false));
        assert!(information_route_is_blocked(true, false, false));
        assert!(information_route_is_blocked(false, true, false));
        assert!(information_route_is_blocked(false, false, true));
    }
}
