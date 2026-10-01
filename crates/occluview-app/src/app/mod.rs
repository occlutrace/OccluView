use super::app_files::{recent_scene_hover, recent_scene_label};
use super::cut::cut_tool::CutTool;
use super::edit_mode::{EditModeCommand, EditModeController, ScreenPolygonSelectionRequest};
use super::layer_actions::{self, LayerContextAction, LayerContextApply, LayerContextRequest};
use super::layers_overlay::{self, LayerOverlayChanges};
use super::live_viewport;
use super::mesh_editor::mesh_editor_overlay::{self, MeshEditorAction};
use super::scene_loading::{
    combine_loaded_scene, load_status_message, LoadQueueCameraReset, SceneLoadMode,
};
use super::viewer::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, desired_render_extent_px,
    home_camera_for_scene, orbit_delta_from_drag, paint_axis_gizmo, pick_scene_hit,
    pick_scene_point, render_extent_change_requires_rerender, viewport_orbit_drag_active,
    viewport_pan_drag_active, zoom_factor_from_scroll, AxisGizmoInput,
};
use super::{Context, PathBuf, Result};
#[cfg(test)]
use crate::scene_loading::{PendingSceneLoad, SceneLoadRequest};
use crate::ui::app_chrome::{load_app_logo_color_image, status_overlay_rect, viewer_visuals};
use crate::ui::scale_bar::ScaleBar;
use anyhow::Error;
use eframe::egui;
use glam::Mat4;
use occluview_core::{Camera, Scene, SceneMesh};
use occluview_render::{
    GpuCamera, GpuMeshUniform, Offscreen, PreparedSceneSource, ThumbnailSpec, ViewportSpec,
};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const OPEN_DIALOG_EXTENSIONS: &[&str] = occluview_formats::V1_OPEN_EXTENSIONS;
const FOREGROUND_PULSE_DURATION: Duration = Duration::from_millis(250);
#[cfg(not(windows))]
const LINUX_OPEN_REQUEST_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) mod align;
#[cfg(test)]
mod app_accessibility_tests;
mod app_bridge_split;
mod app_cut;
mod app_dialogs;
mod app_empty_state;
mod app_guard_dialog;
mod app_help;
mod app_input;
#[cfg(test)]
mod app_input_ownership_tests;
pub(crate) mod app_layer_edits;
mod app_layer_interaction;
mod app_load_errors;
mod app_loading;
#[cfg(test)]
mod app_loading_tests;
mod app_measure;
#[cfg(test)]
mod app_provenance_tests;
mod app_recent_popup;
mod app_ruler_options;
mod app_scale_bar;
#[cfg(test)]
mod app_test_support;
mod app_third_party;
mod app_viewport;
mod app_workspace_commands;
#[cfg(test)]
mod app_workspace_ui_tests;
mod contact;
mod disc_frame;
mod information_dialog;
mod mesh_edit;
mod open_dialogs;
mod render;
mod scene;
mod sculpt;
mod selection_overlay;
mod settings;
mod state;
mod state_document;
mod state_persistence;
mod state_platform;
mod state_render;
mod state_tool;
mod state_ui;
pub(crate) mod workspace;

use app_layer_edits::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
    apply_layer_context_action_with_status,
    apply_visible_selected_face_mesh_edit_action_with_limit,
};
use app_load_errors::{load_error_dialog, load_failure_summary};
use app_scale_bar::paint_scale_bar;
pub(crate) use state::{OccluViewApp, SceneContext};
use state_document::MeshSelectionDrag;
pub(crate) use state_platform::StartupHandles;
use state_render::RenderedFrame;
use state_ui::{AppErrorAction, AppErrorDialog, PendingReplaceOpen};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_dialog_accepts_hps_and_legacy_alias() {
        assert!(OPEN_DIALOG_EXTENSIONS.contains(&occluview_formats::LEGACY_HPS_EXTENSION));
        assert!(OPEN_DIALOG_EXTENSIONS.contains(&"hps"));
    }
}
