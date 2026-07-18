use super::app_chrome::{
    load_app_logo_color_image, paint_version_stamp, status_overlay_rect, viewer_visuals,
};
use super::app_files::{
    load_recent_files, recent_scene_hover, recent_scene_label, save_recent_files,
};
use super::cut_tool::CutTool;
use super::edit_mode::{EditModeCommand, EditModeController, ScreenPolygonSelectionRequest};
use super::layer_actions::{self, LayerContextAction, LayerContextApply, LayerContextRequest};
use super::layers_overlay::{self, LayerOverlayChanges};
use super::live_viewport::{self, SharedLiveViewport};
use super::mesh_editor_overlay::{self, MeshEditorAction};
use super::scene_loading::{
    combine_loaded_scene, load_status_message, LoadQueueCameraReset, PendingSceneLoad,
    SceneLoadMode, SceneLoadRequest,
};
use super::viewer::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, desired_render_extent_px,
    home_camera_for_scene, orbit_delta_from_drag, paint_axis_gizmo, pick_scene_hit,
    pick_scene_point, render_extent_change_requires_rerender, viewport_orbit_drag_active,
    viewport_pan_drag_active, zoom_factor_from_scroll, DEFAULT_RENDER_EXTENT_PX,
};
use super::{
    read_files_with_key_provider, single_instance, Context, PathBuf, Result, RuntimeHpsKeyProvider,
};
use anyhow::Error;
use eframe::egui;
use glam::Mat4;
use occluview_core::{Camera, RecentFiles, ScaleBar, Scene, SceneMesh};
use occluview_render::{
    GpuCamera, GpuMeshUniform, Offscreen, PreparedScene, PreparedSceneSource,
    PreparedSceneTopology, PreparedSceneUpdate, ThumbnailSpec, ViewportSpec,
};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const OPEN_DIALOG_EXTENSIONS: &[&str] = occluview_formats::V1_OPEN_EXTENSIONS;
const VIEWPORT_BACKGROUND_LINEAR: [f64; 4] = [0.80, 0.82, 0.84, 1.0];
const FOREGROUND_PULSE_DURATION: Duration = Duration::from_millis(250);
#[cfg(not(windows))]
const LINUX_OPEN_REQUEST_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

mod app_bridge_split;
mod app_cut_measure;
mod app_dialogs;
mod app_layer_edits;
mod app_layer_interaction;
mod app_load_errors;
mod app_loading;
mod app_mesh_editor;
mod app_mesh_export;
mod app_render;
mod app_scale_bar;
mod app_scene_commit;
mod app_sculpt;
mod app_viewport;
mod selection_overlay;
mod state;

use app_layer_edits::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
    apply_layer_context_action_with_status,
    apply_visible_selected_face_mesh_edit_action_with_limit,
};
use app_load_errors::load_error_dialog;
use app_scale_bar::paint_scale_bar;
pub(crate) use state::{parse_args, OccluViewApp, StartupHandles};
use state::{
    AboutWindowState, AppErrorDialog, MeshSelectionDrag, PendingReplaceOpen, RenderedFrame,
    SceneStats,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_dialog_accepts_hps_and_legacy_alias() {
        assert!(OPEN_DIALOG_EXTENSIONS.contains(&occluview_formats::LEGACY_HPS_EXTENSION));
        assert!(OPEN_DIALOG_EXTENSIONS.contains(&"hps"));
    }
}
