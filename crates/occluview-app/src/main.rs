//! `occluview-app` - the desktop viewer binary.
//!
//! Native desktop app for Windows and Linux.
//!
//! ## Status (this commit)
//!
//! Opens one or more files from the CLI args via `occluview-formats` and draws
//! the scene through the shared `occluview-render` wgpu pipeline. The main
//! viewport uses a live eframe/wgpu callback when available, with the offscreen
//! path kept for thumbnails, cut-view previews, and fallback.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use occluview_formats::{hps::RuntimeHpsKeyProvider, read_files_with_key_provider};
use std::path::PathBuf;

mod app;
mod app_bootstrap;
mod app_chrome;
mod app_files;
mod app_paths;
mod bridge_split;
mod bridge_split_overlay;
mod cut_geometry;
mod cut_manipulator;
mod cut_overlay;
mod cut_ruler;
mod cut_tool;
mod edit_mode;
#[cfg(windows)]
mod jump_list;
mod layer_actions;
mod layers_overlay;
mod live_viewport;
mod measure_draw;
mod measure_overlay;
mod measure_tool;
mod mesh_editor_icons;
mod mesh_editor_overlay;
mod probe_section;
mod repair_report;
mod scene_loading;
mod sculpt_tool;
mod section_view;
mod single_instance;
mod ui_theme;
mod update_notice;
mod viewer;

#[cfg(windows)]
pub(crate) const APP_USER_MODEL_ID: &str = "OccluTrace.OccluView";
#[cfg(target_os = "linux")]
const LINUX_DESKTOP_APP_ID: &str = "ai.occlutrace.OccluView";
pub(crate) const LIVE_VIEWPORT_SAMPLE_COUNT: u16 = 4;

#[cfg(test)]
fn primary_camera_action_labels() -> [&'static str; 0] {
    []
}

fn should_append_incoming_open_state(
    has_scene: bool,
    has_active_load: bool,
    queued_load_count: usize,
) -> bool {
    has_scene || has_active_load || queued_load_count != 0
}

fn main() {
    app_bootstrap::main_entry();
}

#[cfg(test)]
mod cut_manipulator_hostile_tests;
#[cfg(test)]
mod primary_ui_tests;
