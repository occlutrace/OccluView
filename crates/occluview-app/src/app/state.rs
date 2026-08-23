//! `OccluViewApp` itself: the fields the whole binary shares, and the small
//! set of methods that keep them consistent.
//!
//! Four booleans invalidate the render caches, and the rule for setting them
//! is the one thing worth knowing before touching this file:
//!
//! - `needs_render` — draw again this frame. A camera move sets this and
//!   nothing else, because the prepared geometry did not change.
//! - `live_viewport_scene_dirty` — the live eframe/wgpu path must rebuild its
//!   `PreparedScene`. Only ever set when `live_viewport` is `Some`.
//! - `offscreen_scene_dirty` — the offscreen path must rebuild its own.
//! - `selection_overlay_dirty` — the selection overlay mesh must be rebuilt.
//!
//! Geometry, materials or the scene itself: set all four. Camera only: set the
//! first. Miss one of the last three and that path keeps a stale cache on
//! screen, invisible from whichever path the author happened to be testing.

use super::open_dialogs::OpenDialogs;
use super::{
    egui, home_camera_for_scene, load_recent_files, save_recent_files, single_instance, Arc,
    Camera, CutTool, Duration, EditModeController, Instant, LoadQueueCameraReset, Offscreen,
    PathBuf, PendingSceneLoad, PreparedScene, RecentFiles, Scene, SceneLoadRequest,
    SharedLiveViewport, DEFAULT_RENDER_EXTENT_PX,
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
    pub version: bool,
    pub files: Vec<PathBuf>,
}

pub(crate) fn parse_args() -> Args {
    let mut shell_refresh = false;
    let mut version = false;
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--shell-refresh" => shell_refresh = true,
            "--version" | "-V" => version = true,
            _ => files.push(PathBuf::from(arg)),
        }
    }
    Args {
        shell_refresh,
        version,
        files,
    }
}

#[allow(clippy::struct_excessive_bools)]
pub(crate) struct OccluViewApp {
    pub(super) repaint_ctx: egui::Context,
    pub(super) scene: Option<Arc<Scene>>,
    pub(super) current_paths: Vec<PathBuf>,
    /// Where the last successful export of this session landed. Save dialogs
    /// fall back here for a layer with no file of its own before leaving the
    /// choice to the platform.
    pub(super) last_export_dir: Option<PathBuf>,
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
    /// Raises the window on an open-file handoff through the native compositor
    /// activation protocol. See activation.rs.
    pub(super) raise_target: single_instance::RaiseTarget,
    /// Latest window-activation token forwarded by a second instance, used as
    /// provenance for the raise. Cleared once the raise's attention pulse ends.
    pub(super) pending_raise_token: Option<String>,
    pub(super) about_window: AboutWindowState,
    /// The Third-party licenses window, opened from About.
    pub(super) third_party_window_open: bool,
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
    /// Interactive sculpt-brush tool (the dental CAD Freeforming workflow):
    /// the armed brush plus the live per-drag stroke session. Only
    /// meaningful while a mesh edit session is active.
    pub(super) sculpt: crate::sculpt_tool::SculptTool,
    pub(super) align: crate::align_tool::AlignTool,
    pub(super) align_worker: Option<crate::align_worker::AlignWorker>,
    pub(super) align_settings: crate::align_worker::AlignSettings,
    pub(super) align_status: Option<String>,
    pub(super) align_stats: Option<occluview_align::DeviationStats>,
    pub(super) align_rejected: Vec<u32>,
    /// Per-layer overlay colours currently on screen.
    pub(super) align_overlay_colors: Vec<(occluview_core::SceneMeshId, Arc<Vec<[u8; 4]>>)>,
    /// The flat arrays the align worker takes, kept between jobs so a settings
    /// change does not re-copy geometry that has not moved.
    pub(super) align_geometry: crate::align_geometry::AlignGeometry,
    /// The vertex buffer a deviation map is uploaded through, repainted across
    /// re-colours instead of rebuilt.
    pub(super) align_painted: crate::align_geometry::PaintedVertices,
    /// Set when new colours are attached and the GPU has not seen them yet.
    /// Consumed by the viewport sync, which is the one place that knows whether
    /// there is a prepared scene to write into.
    pub(super) deviation_push_pending: bool,
    /// What the operator marked out of the match, on both scans. Owns its own
    /// revision and coverage counts, so no caller can change a mask without the
    /// caches downstream hearing about it.
    pub(super) align_markings: crate::align_markings::AlignMarkings,
    pub(super) align_drag: Option<super::app_align_drag::AlignDrag>,
    pub(super) align_constraint: crate::align_drag::DragConstraint,
    pub(super) align_brush: crate::align_brush::AlignBrush,
    /// What the per-vertex colours on the moving scan currently mean.
    pub(super) align_overlay: super::app_align_display::AlignOverlay,
    pub(super) align_session_poses: Vec<(occluview_core::SceneMeshId, glam::Affine3A)>,
    pub(super) align_tab: crate::align_panel::AlignTab,
    pub(super) align_ghosted: Vec<(occluview_core::SceneMeshId, f32)>,
    /// Which mesh-editor tab is showing (selection/repair vs sculpt).
    pub(super) editor_tab: crate::mesh_editor_overlay::EditorTab,
    pub(super) edit_mode: EditModeController,
    pub(super) update_notice: crate::update_notice::UpdateNotice,
    /// Layers carrying unsaved edits: the in-scene mesh differs from what was
    /// loaded from disk. Written by every applied mesh-edit and its undo/redo,
    /// cleared per layer when that layer is written out, and entirely when the
    /// scene is replaced or closed. The close-without-saving guard and the save
    /// flow both read it, through [`Self::has_unsaved_mesh_edits`].
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
}

/// Edit a scene that has been taken out of `self.scene` for the duration.
///
/// `Option::take` moves the handle without touching the strong count, so the
/// rule [`OccluViewApp::live_scene_mut`] asserts still applies: a second live
/// handle turns `Arc::make_mut` into a copy of the whole case. The two sculpt
/// paths that take the scene out, because they put a rebuilt one back, assert
/// it here instead of reaching for `Arc::make_mut` where no guard looks.
/// Complain, once per run, that a scene edit found a second handle alive.
///
/// The `debug_assert` beside each call site fires in a test build and is gone
/// from a release one, so in the field the same mistake is silent and merely
/// slow: measured at 62 ms per edit on two welded arches and 253 ms on two
/// soups, against under a microsecond when the handle is unique. Once per run,
/// because these calls are per-frame and a line per frame would overwrite the
/// crash-report ring with this one message.
fn report_shared_scene_edit(handles: usize) {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    tracing::warn!(
        handles,
        "scene edited in place while another handle was alive; this copies the \
         whole case and will keep doing so until the handle is released"
    );
}

pub(super) fn taken_scene_mut(scene: &mut Arc<Scene>) -> &mut Scene {
    let handles = Arc::strong_count(scene);
    debug_assert_eq!(
        handles, 1,
        "in-place scene edit while another Arc<Scene> is alive: this \
         silently deep-copies every vertex, index and texture of the case"
    );
    if handles != 1 {
        report_shared_scene_edit(handles);
    }
    Arc::make_mut(scene)
}

impl OccluViewApp {
    /// Status text is a transient interaction hint, not a second permanent
    /// toolbar. Expiring it in one place keeps every caller on the same clock
    /// without a timer per tool.
    ///
    /// KNOWN BEHAVIOUR, not a bug to rediscover: the clock restarts on a change
    /// of TEXT, not on a call. Repeat an action at t = 3.9 s and the
    /// confirmation is byte-identical to the one already showing, the snapshot
    /// comparison sees nothing change, and the toast disappears a tenth of a
    /// second later as though it had never been shown.
    ///
    /// The shape that fixes it is one `Option<StatusMessage { text, since }>`
    /// behind a setter that restarts the clock on every call: 91 mechanical
    /// call-site edits, and a commit containing those and nothing else.
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
            ctx.request_repaint_after(STATUS_MESSAGE_TTL - elapsed);
        }
    }

    pub(crate) fn new(
        repaint_ctx: egui::Context,
        startup_paths: Vec<PathBuf>,
        live_viewport: Option<SharedLiveViewport>,
        startup: StartupHandles,
    ) -> Self {
        let mut app = Self {
            repaint_ctx: repaint_ctx.clone(),
            scene: None,
            current_paths: Vec::new(),
            last_export_dir: None,
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
            status_message_since: None,
            status_message_snapshot: None,
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
            third_party_window_open: false,
            repair_report: crate::repair_report::RepairReportDialog::default(),
            app_logo: None,
            foreground_pulse_until: None,
            viewport_orbit_cursor_grabbed: false,
            viewport_secondary_gesture_moved_since_press: false,
            mesh_selection_drag: None,
            sculpt: crate::sculpt_tool::SculptTool::default(),
            align: crate::align_tool::AlignTool::default(),
            align_worker: None,
            align_settings: crate::align_worker::AlignSettings::default(),
            align_status: None,
            align_stats: None,
            align_rejected: Vec::new(),
            align_overlay_colors: Vec::new(),
            align_geometry: crate::align_geometry::AlignGeometry::default(),
            align_painted: crate::align_geometry::PaintedVertices::default(),
            deviation_push_pending: false,
            align_markings: crate::align_markings::AlignMarkings::default(),
            align_drag: None,
            align_constraint: crate::align_drag::DragConstraint::default(),
            align_brush: crate::align_brush::AlignBrush::default(),
            align_overlay: super::app_align_display::AlignOverlay::default(),
            align_session_poses: Vec::new(),
            align_tab: crate::align_panel::AlignTab::default(),
            align_ghosted: Vec::new(),
            editor_tab: crate::mesh_editor_overlay::EditorTab::default(),
            edit_mode: EditModeController::default(),
            update_notice: crate::update_notice::UpdateNotice::begin_check(),
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
            about: self.about_window == AboutWindowState::Open,
            third_party: self.third_party_window_open,
        }
        .any()
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

    /// The live scene, mutable in place.
    ///
    /// `Arc::make_mut` copies the whole scene whenever a second handle exists,
    /// and the callers below all run per frame: a slider drag, a brush dab, a
    /// nudge of an aligned layer. On two 945k-vertex arches that is 40 ns as
    /// sole handle against 45.75 ms otherwise -- the entire case copied every
    /// frame to change a few numbers.
    ///
    /// Every in-place scene edit comes through here, so a caller that keeps a
    /// handle alive across the edit fails a test instead of costing frames.
    pub(super) fn live_scene_mut(&mut self) -> Option<&mut Scene> {
        let scene = self.scene.as_mut()?;
        let handles = Arc::strong_count(scene);
        debug_assert_eq!(
            handles, 1,
            "in-place scene edit while another Arc<Scene> is alive: this \
             silently deep-copies every vertex, index and texture of the case"
        );
        if handles != 1 {
            report_shared_scene_edit(handles);
        }
        Some(Arc::make_mut(scene))
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
        self.unsaved_edit_layer_ids.insert(layer_id);
    }

    /// Whether anything in the scene differs from what is on disk.
    ///
    /// Derived, not tracked. A parallel `bool` needs four assignments to stay
    /// in step with the set, and the export path skipped one; see
    /// [`Self::forget_unsaved_edits`] for what the disagreement cost.
    pub(super) fn has_unsaved_mesh_edits(&self) -> bool {
        !self.unsaved_edit_layer_ids.is_empty()
    }

    /// Forget the unsaved-edit tracking for the layers just written to disk.
    ///
    /// Scoped, because a save can skip a layer: the whole-scene and per-layer
    /// saves write only what is VISIBLE. Clearing everything after one of those
    /// told the close guard that a hidden layer's edits were on disk when they
    /// were only in memory, and the app then shut without asking.
    pub(super) fn forget_unsaved_edits(&mut self, layers: &[occluview_core::SceneMeshId]) {
        for layer in layers {
            self.unsaved_edit_layer_ids.remove(layer);
        }
    }

    /// Forget all unsaved-edit tracking (scene replaced, closed, or saved).
    pub(super) fn clear_unsaved_mesh_edits(&mut self) {
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
        self.expire_status_message(ctx);
        Self::schedule_linux_open_request_repaint(ctx);
        self.process_scene_loads(ctx);
        self.poll_sculpt_preparation(ctx);
        self.poll_sculpt_worker(ctx);
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
        self.show_third_party_window(ctx);
        self.repair_report.ui(ctx);
        self.update_notice.show(ctx);
        self.guard_unsaved_close(ctx);
        self.guard_pending_replace_open(ctx);
    }
}
