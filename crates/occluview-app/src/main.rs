//! `occluview-app` - the desktop viewer binary.
//!
//! Native desktop app for Windows and Linux.
//!
//! ## Shape of the binary
//!
//! Opens one or more files from the CLI args via `occluview-formats` and draws
//! the scene through the shared `occluview-render` wgpu pipeline. The main
//! viewport uses a live eframe/wgpu callback when available, with the offscreen
//! path kept for thumbnails, cut-view previews, and fallback.
//!
//! Everything above that sits in the `mod` list below: `app` and `viewer` hold
//! the application state and the viewport, `scene_loading` brings files in,
//! `align_*` registers one scan onto another, `edit_mode` and `sculpt_*` change
//! geometry, `cut_*` and `section_view` slice it, `measure_*` and
//! `probe_section` measure it, `layer_*` and `mesh_editor_*` drive the panels,
//! and `single_instance`, `jump_list` and `update_notice` handle the desktop
//! integration around all of it.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use occluview_formats::{hps::RuntimeHpsKeyProvider, read_files_with_key_provider};
use std::path::PathBuf;

mod align_brush;
mod align_drag;
mod align_geometry;
mod align_markings;
mod align_overlay;
mod align_panel;
mod align_panel_brush;
mod align_panel_map;
mod align_panel_roles;
mod align_state;
mod align_tool;
#[cfg(test)]
mod align_tool_tests;
mod align_worker;
pub(crate) mod app;
mod app_bootstrap;
mod app_chrome;
mod app_files;
mod app_paths;
mod app_settings;
mod bridge_split;
mod bridge_split_overlay;
mod cut_geometry;
mod cut_manipulator;
mod cut_overlay;
mod cut_ruler;
mod cut_tool;
mod edit_mode;
mod icons;
mod interaction_hints;
#[cfg(windows)]
mod jump_list;
mod jump_list_model;
mod layer_actions;
mod layers_overlay;
mod live_viewport;
mod measure_draw;
mod measure_overlay;
mod measure_tool;
mod mesh_editor_icons;
mod mesh_editor_overlay;
mod modal_surface;
mod probe_section;
mod recent_files;
mod repair_report;
mod scale_bar;
mod scene_loading;
mod sculpt_tool;
mod sculpt_worker;
mod section_view;
#[cfg(windows)]
mod shell_refresh;
mod single_instance;
mod ui_theme;
mod update_notice;
mod viewer;

#[cfg(windows)]
pub(crate) const APP_USER_MODEL_ID: &str = "OccluTrace.OccluView";
#[cfg(target_os = "linux")]
const LINUX_DESKTOP_APP_ID: &str = "ai.occlutrace.OccluView";
pub(crate) const LIVE_VIEWPORT_SAMPLE_COUNT: u16 = 4;

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
