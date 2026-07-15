//! Live preview-pane scene controller.
//!
//! The COM layer owns Win32 windowing. This subtree owns mesh loading, camera
//! interaction, and pixel rendering for Explorer Preview Pane.

#![cfg_attr(not(windows), allow(dead_code))]
#![cfg_attr(test, allow(clippy::cast_possible_truncation, clippy::expect_used))]

mod interaction;
mod load;
mod render;

#[cfg(test)]
mod test_support;

use occluview_core::{Camera, Scene};
use occluview_render::{Offscreen, PreparedScene};

#[cfg_attr(not(windows), allow(unused_imports))]
pub(crate) use interaction::win32_preview_orbit_delta;
#[cfg_attr(not(windows), allow(unused_imports))]
pub(crate) use interaction::PreviewViewPreset;

pub(crate) struct PreviewSceneState {
    pub(super) scene: Scene,
    pub(super) camera: Camera,
    pub(super) offscreen: Offscreen,
    pub(super) prepared_scene: PreparedScene,
}

#[cfg(test)]
mod tests {
    #[test]
    fn preview_scene_facade_stays_split_by_responsibility() {
        let facade = include_str!("mod.rs").replace("\r\n", "\n");
        let production_source = facade
            .split_once("\n#[cfg(test)]\nmod tests")
            .map_or(facade.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("mod interaction;")
                && production_source.contains("mod load;")
                && production_source.contains("mod render;"),
            "preview scene should stay split by loading, rendering, and interaction"
        );
        assert!(
            production_source.contains("pub(crate) struct PreviewSceneState")
                && production_source
                    .contains("pub(crate) use interaction::win32_preview_orbit_delta;"),
            "facade should preserve the COM-facing preview API"
        );
        assert!(
            !production_source.contains("fn load_preview_mesh_from_file(")
                && !production_source.contains("fn render_rgba_with_background(")
                && !production_source.contains("fn viewport_ray("),
            "facade should not absorb loading, rendering, or interaction implementation"
        );
    }
}
