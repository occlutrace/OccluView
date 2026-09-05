//! `occluview` — the desktop viewer binary.
//!
//! Thin entry over the [`occluview_app`] library: platform attributes live
//! here, everything else behind the library boundary.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    occluview_app::main_entry();
}
