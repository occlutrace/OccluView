//! Render-owned state: GPU mirrors, caches, and typed invalidation cursors.
//!
//! Owned invariants:
//!
//! - `camera` describes the loaded scene; it is `None` exactly when no scene
//!   is available to frame.
//! - `prepared_scene` / `prepared_selection_overlay` mirror GPU buffers and
//!   are rebuilt only when their invalidation cursor says so, never per frame.
//! - `section_cache` is content-keyed: camera motion never recomputes it,
//!   only geometry/transform/visibility or plane changes do.
//! - `rendered` is the last presented frame; `render_extent_px` sizes it.
//!
//! Permitted mutation entry points: [`RenderState::new`] for bootstrap, the
//! render pipeline in `app_render.rs` operating on `&mut RenderState`, and
//! root orchestration naming a semantic invalidation cause. Worker inputs
//! arrive as sculpt shadow uploads through the tool→render flush; the
//! prepared scenes are the cross-domain output the viewport consumes.

use super::egui;
use super::Instant;
use crate::invalidation::RenderInvalidation;
use crate::live_viewport::SharedLiveViewport;
use crate::viewer::DEFAULT_RENDER_EXTENT_PX;
use occluview_core::Camera;
use occluview_render::{Offscreen, PreparedScene};

pub(super) struct RenderedFrame {
    pub(super) texture: egui::TextureHandle,
    pub(super) pixels: Vec<u8>,
    pub(super) size_px: [u16; 2],
}

pub(super) struct RenderState {
    pub(super) camera: Option<Camera>,
    pub(super) live_viewport: Option<SharedLiveViewport>,
    pub(super) offscreen: Option<Offscreen>,
    /// A terminal offscreen GPU failure must not be retried on every egui
    /// repaint. The live path has its own fault latch; this one covers the
    /// fallback path and cut-view readbacks.
    pub(super) offscreen_failed: bool,
    /// When the last retryable offscreen failure happened.
    ///
    /// A readback deadline is not a device verdict, so the fallback path gets
    /// another attempt — but not on every repaint, which is the storm the
    /// terminal latch was added to stop. The wait is short enough that an
    /// operator who repositions the cut plane does not notice it and long
    /// enough that a machine under load is not asked to fail on a loop.
    pub(super) offscreen_retry_after: Option<Instant>,
    pub(super) prepared_scene: Option<PreparedScene>,
    pub(super) prepared_selection_overlay: Option<PreparedScene>,
    pub(super) render_extent_px: [u16; 2],
    pub(super) rendered: Option<RenderedFrame>,
    /// Typed redraw and cache-staleness state; see [`RenderInvalidation`].
    /// Render paths consume their own cursors.
    pub(super) invalidation: RenderInvalidation,
    /// Content-keyed cache of the section contour for the active cut plane.
    pub(super) section_cache: occluview_core::scene::SectionCache,
}

impl RenderState {
    pub(super) fn new(live_viewport: Option<SharedLiveViewport>) -> Self {
        Self {
            camera: None,
            live_viewport,
            offscreen: None,
            offscreen_failed: false,
            offscreen_retry_after: None,
            prepared_scene: None,
            prepared_selection_overlay: None,
            render_extent_px: DEFAULT_RENDER_EXTENT_PX,
            rendered: None,
            invalidation: RenderInvalidation::new(),
            section_cache: occluview_core::scene::SectionCache::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_state_starts_with_clean_caches() {
        let state = RenderState::new(None);

        assert!(state.camera.is_none());
        assert!(state.prepared_scene.is_none());
        assert!(state.prepared_selection_overlay.is_none());
        assert!(state.rendered.is_none());
        assert!(!state.offscreen_failed);
        assert!(!state.invalidation.redraw_pending());
        assert!(!state.invalidation.live_scene_stale());
        assert!(!state.invalidation.offscreen_scene_stale());
        assert!(!state.invalidation.live_overlay_stale());
        assert!(!state.invalidation.offscreen_overlay_stale());
    }
}
