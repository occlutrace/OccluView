//! `occluview-app` — the desktop viewer application behind a library boundary.
//!
//! The `occluview` binary entry point (`main.rs`) holds only platform
//! attributes and delegates to [`main_entry`]. The real module graph,
//! bootstrap/composition, startup ordering, panic boundary, update behavior,
//! single-instance handoff, and platform cfgs live here, so composition is
//! testable without parsing the binary source.
//!
//! Native desktop app for Windows and Linux.
//!
//! ## Shape of the application
//!
//! Opens one or more files from the CLI args via `occluview-formats` and draws
//! the scene through the shared `occluview-render` wgpu pipeline. The main
//! viewport uses a live eframe/wgpu callback when available, with the offscreen
//! path kept for thumbnails, cut-view previews, and fallback.
//!
//! Everything above that sits in the `mod` list below: `app` and `viewer` hold
//! the application state and the viewport, `scene_loading` brings files in,
//! `align` registers one scan onto another, `edit_mode` and `sculpt_*` change
//! geometry, `cut_*` and `section_view` slice it, `measure_*` and
//! `probe_section` measure it, `layer_*` and `mesh_editor_*` drive the panels,
//! `ui` holds the shared theme, icons, chrome and accessibility layer, and
//! `desktop` handles the single-instance handoff, shell surfaces, recent files
//! and update notice around all of it.

// Test setup failures must fail the test instead of passing through an early return.
#![cfg_attr(test, allow(clippy::panic))]
// The platform FFI modules (`app_bootstrap`, `desktop::single_instance`,
// `desktop::jump_list`, `desktop::shell_refresh`) opt in with their own
// `#![allow(unsafe_code)]`; everything else in the crate must stay
// unsafe-free.
#![deny(unsafe_code)]

pub mod invalidation;
mod startup;

pub use startup::{
    file_extensions, parse_args, parse_args_from, should_append_incoming_open_state, StartupArgs,
};

use anyhow::{Context, Result};
use std::path::PathBuf;

mod align;
pub(crate) mod app;
mod app_bootstrap;
mod app_files;
mod app_settings;
mod bridge_split;
mod bridge_split_overlay;
mod contact;
#[cfg(test)]
mod contact_render_tests;
mod contact_worker;
mod cut_geometry;
mod cut_manipulator;
mod cut_overlay;
mod cut_ruler;
mod cut_tool;
mod desktop;
mod edit_mode;
pub(crate) mod i18n;
mod layer_actions;
mod layers_overlay;
mod live_viewport;
mod measure_draw;
mod measure_overlay;
mod measure_ruler;
mod measure_tool;
mod mesh_editor_icons;
mod mesh_editor_overlay;
mod probe_section;
mod repair_report;
mod scene_loading;
mod sculpt_kernel;
mod sculpt_tool;
mod sculpt_worker;
mod section_view;
mod ui;
mod viewer;

pub use app_bootstrap::main_entry;

#[cfg(windows)]
pub(crate) const APP_USER_MODEL_ID: &str = "OccluTrace.OccluView";
#[cfg(target_os = "linux")]
const LINUX_DESKTOP_APP_ID: &str = "ai.occlutrace.OccluView";

#[cfg(test)]
mod cut_manipulator_hostile_tests;
#[cfg(test)]
mod panel_shots;
#[cfg(test)]
mod perf_harness;
#[cfg(test)]
mod primary_ui_tests;
