//! What a bug report needs to know about the machine.
//!
//! The About dialog shows it and copies it, so a report can say what the
//! viewer ran on without the person writing it knowing where to look.

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

/// The plain-text block the About dialog copies.
///
/// Not translated, on purpose: it is read by whoever receives the report.
pub(crate) fn details(version: &str, graphics: Option<&str>) -> String {
    let mut lines = vec![
        format!("OccluView {version}"),
        format!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH),
    ];
    if let Some(graphics) = graphics {
        lines.push(format!("Graphics: {graphics}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::details;

    #[test]
    fn the_details_name_the_version_the_system_and_the_adapter() {
        let text = details("1.2.1", Some("llvmpipe (Vulkan, Cpu)"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "OccluView 1.2.1");
        assert!(lines[1].starts_with("OS: "));
        assert_eq!(lines[2], "Graphics: llvmpipe (Vulkan, Cpu)");
        assert_eq!(details("1.2.1", None).lines().count(), 2);
    }
}
