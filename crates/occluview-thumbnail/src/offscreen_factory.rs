use crate::ThumbnailError;
use occluview_render::{Offscreen, RenderDeadline};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Whether this build should ask wgpu for a hardware adapter. Tests use the
/// software fallback so they do not depend on the host GPU.
///
/// The shell crate has the same per-crate definition because `cfg!(test)` is
/// evaluated independently for each crate.
pub(crate) const fn should_prefer_hardware_offscreen() -> bool {
    !cfg!(test)
}

/// Set once by a process that renders a single thumbnail and exits.
static SOFTWARE_RENDERER_ONLY: AtomicBool = AtomicBool::new(false);

/// Keep a short-lived thumbnail process on the software rasteriser. Long-lived
/// shell and viewer processes may continue to use a hardware adapter.
pub fn use_software_renderer_only() {
    SOFTWARE_RENDERER_ONLY.store(true, Ordering::Relaxed);
}

pub(crate) fn create_thumbnail_offscreen() -> Result<Offscreen, ThumbnailError> {
    if let Some(offscreen) = hardware_offscreen_that_draws() {
        return Ok(offscreen);
    }
    pollster::block_on(Offscreen::new()).map_err(Into::into)
}

/// A hardware renderer, but only if it can put a triangle on the screen.
///
/// A device that accepts commands and renders nothing would otherwise answer
/// every scan with a blank tile -- which Explorer caches against the file's
/// timestamp and never asks about again.
fn hardware_offscreen_that_draws() -> Option<Offscreen> {
    if !should_prefer_hardware_offscreen() || SOFTWARE_RENDERER_ONLY.load(Ordering::Relaxed) {
        return None;
    }
    let offscreen = pollster::block_on(Offscreen::new_prefer_hardware()).ok()?;
    if pollster::block_on(
        offscreen.can_draw_with_deadline(RenderDeadline::after(Duration::from_secs(2))),
    ) {
        return Some(offscreen);
    }
    tracing::warn!("hardware adapter drew nothing; falling back to the software rasteriser");
    None
}
