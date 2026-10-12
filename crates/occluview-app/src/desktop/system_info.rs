//! Graphics adapter information shown in the About dialog.

use eframe::egui_wgpu::wgpu;
use std::sync::OnceLock;

static GRAPHICS: OnceLock<String> = OnceLock::new();

/// Remember the adapter the viewer drew its first frame on.
pub(crate) fn note_graphics(info: &wgpu::AdapterInfo) {
    let _ = GRAPHICS.set(format!(
        "{} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    ));
}

/// The adapter the viewer runs on, once it has one.
pub(crate) fn graphics() -> Option<&'static str> {
    GRAPHICS.get().map(String::as_str)
}
