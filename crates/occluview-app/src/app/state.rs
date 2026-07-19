use super::{
    egui, home_camera_for_scene, load_recent_files, save_recent_files, single_instance, Arc,
    Camera, CutTool, EditModeController, Instant, LayerOverlayChanges, LoadQueueCameraReset,
    Offscreen, PathBuf, PendingSceneLoad, PreparedScene, RecentFiles, Result, Scene,
    SceneLoadRequest, SharedLiveViewport, ViewportSpec, DEFAULT_RENDER_EXTENT_PX,
};

/// Everything the bootstrap hands the app about how this process was started:
/// the single-instance guard, the window raise handle, and the launcher's
/// activation token (focus provenance for the first load).
pub(crate) struct StartupHandles {
    pub(crate) single_instance: single_instance::SingleInstance,
    pub(crate) raise_target: single_instance::RaiseTarget,
    pub(crate) activation_token: Option<String>,
}

pub(crate) struct Args {
    pub shell_refresh: bool,
    pub files: Vec<PathBuf>,
}

pub(crate) fn parse_args() -> Args {
    let mut shell_refresh = false;
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--shell-refresh" {
            shell_refresh = true;
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    Args {
        shell_refresh,
        files,
    }
}

#[allow(clippy::struct_excessive_bools)]
pub(crate) struct OccluViewApp {
    pub(super) repaint_ctx: egui::Context,
    pub(super) scene: Option<Arc<Scene>>,
    pub(super) scene_stats: Option<SceneStats>,
    pub(super) current_paths: Vec<PathBuf>,
    pub(super) recent_files: RecentFiles,
    pub(super) camera: Option<Camera>,
    pub(super) live_viewport: Option<SharedLiveViewport>,
    pub(super) offscreen: Option<Offscreen>,
    pub(super) prepared_scene: Option<PreparedScene>,
    pub(super) prepared_selection_overlay: Option<PreparedScene>,
    pub(super) render_extent_px: [u16; 2],
    pub(super) rendered: Option<RenderedFrame>,
    pub(super) needs_render: bool,
    pub(super) live_viewport_scene_dirty: bool,
    pub(super) offscreen_scene_dirty: bool,
    pub(super) selection_overlay_dirty: bool,
    pub(super) status_message: Option<String>,
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
    /// Content-keyed cache of the section contour for the active cut plane.
    /// Camera motion never recomputes it; only geometry/transform/visibility or
    /// plane changes do.
    pub(super) section_cache: occluview_core::scene::SectionCache,
    pub(super) active_load: Option<PendingSceneLoad>,
    pub(super) queued_loads: std::collections::VecDeque<SceneLoadRequest>,
    pub(super) load_queue_camera_reset: LoadQueueCameraReset,
    pub(super) camera_modified_during_load: bool,
    pub(super) incoming_open_requests: single_instance::OpenRequestListener,
    pub(super) _single_instance: single_instance::SingleInstance,
    /// Raises the window on an open-file handoff past WM focus-stealing
    /// prevention (X11 real raise; Wayland falls back). See activation.rs.
    pub(super) raise_target: single_instance::RaiseTarget,
    /// Latest window-activation token forwarded by a second instance, used as
    /// provenance for the raise. Cleared once the raise's attention pulse ends.
    pub(super) pending_raise_token: Option<String>,
    pub(super) about_window: AboutWindowState,
    /// Persistent post-repair report card, populated by the Repair executor and
    /// drawn in `update()`; shows what a repair changed (or that nothing did).
    pub(super) repair_report: crate::repair_report::RepairReportDialog,
    pub(super) app_logo: Option<egui::TextureHandle>,
    pub(super) foreground_pulse_until: Option<Instant>,
    pub(super) viewport_orbit_cursor_grabbed: bool,
    /// Suppresses the stationary RMB context menu when the same press already
    /// moved the camera, including motion below egui's click/drag threshold.
    pub(super) viewport_secondary_gesture_moved_since_press: bool,
    pub(super) mesh_selection_drag: Option<MeshSelectionDrag>,
    /// Interactive sculpt-brush tool (exocad Freeforming): the armed brush
    /// plus the live per-drag stroke session. Only meaningful while a mesh
    /// edit session is active.
    pub(super) sculpt: crate::sculpt_tool::SculptTool,
    /// Which mesh-editor tab is showing (selection/repair vs sculpt).
    pub(super) editor_tab: crate::mesh_editor_overlay::EditorTab,
    pub(super) edit_mode: EditModeController,
    pub(super) update_notice: crate::update_notice::UpdateNotice,
    /// Set by every applied mesh-edit (and its undo/redo): the in-scene meshes
    /// differ from what was loaded from disk. Cleared when the scene is
    /// replaced or closed. Drives the close-without-saving guard.
    pub(super) has_unsaved_mesh_edits: bool,
    /// Layers carrying unsaved edits, so the save flow knows exactly which
    /// meshes to offer for export. Kept in lockstep with
    /// `has_unsaved_mesh_edits`.
    pub(super) unsaved_edit_layer_ids: std::collections::BTreeSet<occluview_core::SceneMeshId>,
    /// Layers hidden via Ctrl+MiddleClick, in hide order. Shift+Ctrl+Middle
    /// restores the most recently hidden one (LIFO).
    pub(super) hidden_layer_stack: Vec<occluview_core::SceneMeshId>,
    /// Original opacity of layers made translucent via Shift+MiddleClick, so a
    /// second toggle restores exactly the previous value.
    pub(super) translucent_layer_restore:
        std::collections::HashMap<occluview_core::SceneMeshId, f32>,
    /// The close-guard dialog is on screen.
    pub(super) close_guard_open: bool,
    /// The operator explicitly chose to close without saving.
    pub(super) close_confirmed: bool,
    /// A REPLACE open (menu Open, recent, or a drop/handoff classified as
    /// replace) parked behind the edit-session guard because a live session is
    /// dirty or unsaved edits exist. Held until the operator chooses to open
    /// (save/discard) or cancels; a newer replace supersedes an older parked
    /// one so an open is never silently lost to the void.
    pub(super) pending_replace_open: Option<PendingReplaceOpen>,
}

/// A replace-scene open request parked behind the unsaved-edit guard dialog.
#[derive(Clone)]
pub(super) struct PendingReplaceOpen {
    pub(super) paths: Vec<PathBuf>,
    pub(super) source: &'static str,
}

/// In-progress mesh selection drag. Rectangle drags (default) track an origin
/// and current corner; an armed lasso collects the freehand outline points.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MeshSelectionDrag {
    Rect {
        origin: egui::Pos2,
        current: egui::Pos2,
    },
    Lasso {
        points: Vec<egui::Pos2>,
    },
}

impl MeshSelectionDrag {
    /// Axis-aligned extent of the drag (the rectangle for `Rect`, the bounding
    /// box of the collected outline for `Lasso`).
    pub(super) fn rect(&self) -> egui::Rect {
        match self {
            Self::Rect { origin, current } => egui::Rect::from_two_pos(*origin, *current),
            Self::Lasso { points } => {
                let mut bbox = egui::Rect::NOTHING;
                for &point in points {
                    bbox.extend_with(point);
                }
                bbox
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct AppErrorDialog {
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) details: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AboutWindowState {
    Closed,
    Open,
}

pub(super) struct RenderedFrame {
    pub(super) texture: egui::TextureHandle,
    pub(super) pixels: Vec<u8>,
    pub(super) size_px: [u16; 2],
    pub(super) stats: SceneStats,
}

#[derive(Clone, Copy)]
pub(super) struct SceneStats {
    pub(super) bbox_mm: [f32; 3],
}

impl OccluViewApp {
    pub(crate) fn new(
        repaint_ctx: egui::Context,
        startup_paths: Vec<PathBuf>,
        live_viewport: Option<SharedLiveViewport>,
        startup: StartupHandles,
    ) -> Self {
        let mut app = Self {
            repaint_ctx: repaint_ctx.clone(),
            scene: None,
            scene_stats: None,
            current_paths: Vec::new(),
            recent_files: load_recent_files(),
            camera: None,
            live_viewport,
            offscreen: None,
            prepared_scene: None,
            prepared_selection_overlay: None,
            render_extent_px: DEFAULT_RENDER_EXTENT_PX,
            rendered: None,
            needs_render: false,
            live_viewport_scene_dirty: false,
            offscreen_scene_dirty: false,
            selection_overlay_dirty: false,
            status_message: None,
            app_error: None,
            cut_view: CutTool::default(),
            bridge_split: crate::bridge_split::BridgeSplitController::default(),
            bridge_split_disc: crate::cut_manipulator::CutManipulator::default(),
            bridge_split_section: crate::section_view::SectionView::default(),
            measure: crate::measure_tool::MeasureTool::default(),
            section_cache: occluview_core::scene::SectionCache::new(),
            active_load: None,
            queued_loads: std::collections::VecDeque::new(),
            load_queue_camera_reset: LoadQueueCameraReset::Idle,
            camera_modified_during_load: false,
            incoming_open_requests: single_instance::OpenRequestListener::spawn(repaint_ctx),
            _single_instance: startup.single_instance,
            raise_target: startup.raise_target,
            pending_raise_token: startup.activation_token,
            about_window: AboutWindowState::Closed,
            repair_report: crate::repair_report::RepairReportDialog::default(),
            app_logo: None,
            foreground_pulse_until: None,
            viewport_orbit_cursor_grabbed: false,
            viewport_secondary_gesture_moved_since_press: false,
            mesh_selection_drag: None,
            sculpt: crate::sculpt_tool::SculptTool::default(),
            editor_tab: crate::mesh_editor_overlay::EditorTab::default(),
            edit_mode: EditModeController::default(),
            update_notice: crate::update_notice::UpdateNotice::begin_check(),
            has_unsaved_mesh_edits: false,
            unsaved_edit_layer_ids: std::collections::BTreeSet::new(),
            hidden_layer_stack: Vec::new(),
            translucent_layer_restore: std::collections::HashMap::new(),
            close_guard_open: false,
            close_confirmed: false,
            pending_replace_open: None,
        };
        if !startup_paths.is_empty() {
            app.replace_paths(&startup_paths, "startup");
        }
        app
    }

    pub(super) fn render_now(&mut self, ctx: &egui::Context) {
        self.render_now_impl(ctx);
    }

    pub(super) fn render_scene_pixels(&mut self) -> Result<(ViewportSpec, Vec<u8>, SceneStats)> {
        self.render_scene_pixels_impl()
    }

    pub(super) fn render_cut_now(&mut self, ctx: &egui::Context) {
        self.render_cut_now_impl(ctx);
    }

    pub(super) fn ensure_offscreen(&mut self) -> Result<()> {
        self.ensure_offscreen_impl()
    }

    pub(super) fn sync_live_viewport(&mut self) {
        self.sync_live_viewport_impl();
    }

    pub(super) fn clear_live_viewport(&self) {
        self.clear_live_viewport_impl();
    }

    pub(super) fn poll_gpu_errors(&mut self) {
        self.poll_gpu_errors_impl();
    }

    pub(super) fn set_scene(&mut self, scene: Scene, reset_camera: bool) {
        self.set_scene_impl(scene, reset_camera);
    }

    pub(super) fn update_scene_materials(&mut self, scene: Scene) {
        self.update_scene_materials_impl(scene);
    }

    pub(super) fn clear_scene(&mut self) {
        self.clear_scene_impl();
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn show_toolbar(&mut self, ctx: &egui::Context) {
        self.show_toolbar_impl(ctx);
    }

    pub(super) fn handle_edit_shortcuts(&mut self, ctx: &egui::Context) {
        // Edit hotkeys must never act "behind" an open dialog: undoing a mesh
        // edit while the unsaved-changes or open-guard prompt is up would
        // silently change what "Save" then exports (or which scene is at stake).
        if self.close_guard_open
            || self.pending_replace_open.is_some()
            || self.app_error.is_some()
            || self.about_window == AboutWindowState::Open
            || self.bridge_split_active()
        {
            return;
        }
        self.handle_edit_shortcuts_impl(ctx);
    }

    pub(super) fn show_layers_overlay(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        self.show_layers_overlay_impl(ui, viewport_rect, ctx);
    }

    pub(super) fn apply_layer_overlay_changes(
        &mut self,
        scene: &Scene,
        paths: &[PathBuf],
        changes: LayerOverlayChanges,
        ctx: &egui::Context,
    ) {
        self.apply_layer_overlay_changes_impl(scene, paths, changes, ctx);
    }

    pub(super) fn show_cut_tool_overlay(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) -> bool {
        self.show_cut_tool_overlay_impl(ui, viewport_rect, ctx)
    }

    pub(super) fn show_bridge_split_overlay(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        self.show_bridge_split_overlay_impl(ui, response, ctx)
    }

    pub(super) fn show_measure_tool_overlay(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        suppress_click: bool,
        ctx: &egui::Context,
    ) -> bool {
        self.show_measure_tool_overlay_impl(ui, response, suppress_click, ctx)
    }

    pub(super) fn reset_camera_to_home(&mut self) {
        let Some(scene) = self.scene.as_ref() else {
            self.camera = None;
            return;
        };
        self.camera = Some(home_camera_for_scene(scene));
        self.needs_render = true;
    }

    /// Record that `layer_id` now differs from what was loaded from disk.
    /// Every mesh-edit success path (including undo/redo) routes through here
    /// so the save flow knows exactly which layers to offer for export.
    pub(super) fn mark_mesh_edits_unsaved(&mut self, layer_id: occluview_core::SceneMeshId) {
        self.has_unsaved_mesh_edits = true;
        self.unsaved_edit_layer_ids.insert(layer_id);
    }

    /// Forget all unsaved-edit tracking (scene replaced, closed, or saved).
    pub(super) fn clear_unsaved_mesh_edits(&mut self) {
        self.has_unsaved_mesh_edits = false;
        self.unsaved_edit_layer_ids.clear();
    }

    pub(super) fn mark_camera_modified(&mut self) {
        if self.active_load.is_some() || !self.queued_loads.is_empty() {
            self.camera_modified_during_load = true;
        }
    }

    pub(super) fn request_camera_repaint(&mut self, ctx: &egui::Context) {
        self.needs_render = true;
        self.mark_camera_modified();
        ctx.request_repaint();
    }

    pub(super) fn handle_viewport_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        viewport_rect: egui::Rect,
    ) {
        self.handle_viewport_input_impl(ctx, response, viewport_rect);
    }

    pub(super) fn grab_viewport_orbit_cursor(&mut self, ctx: &egui::Context) {
        self.grab_viewport_orbit_cursor_impl(ctx);
    }

    pub(super) fn release_viewport_orbit_cursor(&mut self, ctx: &egui::Context) {
        self.release_viewport_orbit_cursor_impl(ctx);
    }

    pub(super) fn release_viewport_orbit_cursor_if_inactive(&mut self, ctx: &egui::Context) {
        self.release_viewport_orbit_cursor_if_inactive_impl(ctx);
    }

    pub(super) fn maybe_render_cut_view(&mut self, ctx: &egui::Context) {
        self.maybe_render_cut_view_impl(ctx);
    }

    pub(super) fn show_central_panel(&mut self, ctx: &egui::Context) {
        self.show_central_panel_impl(ctx);
    }

    pub(super) fn sync_render_extent(
        &mut self,
        viewport_points: egui::Vec2,
        pixels_per_point: f32,
    ) {
        self.sync_render_extent_impl(viewport_points, pixels_per_point);
    }

    pub(super) fn push_recent_scene(&mut self, paths: &[PathBuf]) {
        self.recent_files.push_paths(paths);
    }

    pub(super) fn can_render_cut_view(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|scene| CutTool::can_render_bbox(scene.bbox()))
    }

    /// Whether any layer can take a measurement pick (visible triangles).
    pub(super) fn has_measurable_layer(&self) -> bool {
        self.scene.as_ref().is_some_and(|scene| {
            scene
                .meshes()
                .iter()
                .any(|entry| entry.visible && !entry.mesh.is_point_cloud())
        })
    }

    pub(super) fn handle_open_requests(&mut self, ctx: &egui::Context) {
        self.handle_open_requests_impl(ctx);
    }

    pub(super) fn raise_window_for_incoming_open(&mut self, ctx: &egui::Context) {
        self.raise_window_for_incoming_open_impl(ctx);
    }

    pub(super) fn finish_foreground_pulse_if_due(&mut self, ctx: &egui::Context) {
        self.finish_foreground_pulse_if_due_impl(ctx);
    }

    pub(super) fn render_pending_frame(&mut self, ctx: &egui::Context) {
        self.render_pending_frame_impl(ctx);
    }

    #[cfg(not(windows))]
    pub(super) fn schedule_linux_open_request_repaint(ctx: &egui::Context) {
        ctx.request_repaint_after(super::LINUX_OPEN_REQUEST_REPAINT_INTERVAL);
    }

    #[cfg(windows)]
    pub(super) fn schedule_linux_open_request_repaint(_ctx: &egui::Context) {}

    pub(super) fn save_recent_files(&self) {
        save_recent_files(&self.recent_files);
    }
}

impl eframe::App for OccluViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(super::viewer_visuals());
        Self::schedule_linux_open_request_repaint(ctx);
        self.process_scene_loads(ctx);
        self.handle_open_requests(ctx);
        self.finish_foreground_pulse_if_due(ctx);
        self.handle_dropped_files(ctx);
        self.release_viewport_orbit_cursor_if_inactive(ctx);
        self.render_pending_frame(ctx);
        self.handle_edit_shortcuts(ctx);
        self.show_toolbar(ctx);
        self.maybe_render_cut_view(ctx);
        self.show_central_panel(ctx);
        // Second pending-frame pass AFTER viewport input: the live-viewport
        // paint callback reads shared GPU state at encode time (after this
        // update returns), so syncing the camera mutated by THIS frame's drag
        // here removes a full frame of input latency during orbit/pan/zoom.
        self.render_pending_frame(ctx);
        // Surface any GPU fault the wgpu error handler caught this frame before
        // drawing the error dialog, so it appears the same frame it happened.
        self.poll_gpu_errors();
        self.show_error_dialog(ctx);
        self.show_about_window(ctx);
        self.repair_report.ui(ctx);
        self.update_notice.show(ctx);
        self.guard_unsaved_close(ctx);
        self.guard_pending_replace_open(ctx);
    }
}
