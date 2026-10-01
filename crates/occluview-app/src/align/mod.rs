//! Registration of one scan onto another.
//!
//! The point-pair tool and its click model, the mark-out brush and its mask
//! commands, the manual drag and its constraints, the background fit worker
//! and its settings, the deviation overlay and legend, and the window that
//! drives them. `align_state` holds the per-scene state the rest read.

pub(crate) mod align_brush;
pub(crate) mod align_drag;
pub(crate) mod align_geometry;
pub(crate) mod align_markings;
pub(crate) mod align_overlay;
pub(crate) mod align_panel;
pub(crate) mod align_panel_brush;
pub(crate) mod align_panel_map;
pub(crate) mod align_panel_roles;
pub(crate) mod align_panel_settings;
pub(crate) mod align_state;
pub(crate) mod align_tool;
#[cfg(test)]
mod align_tool_tests;
pub(crate) mod align_worker;
