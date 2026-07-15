use crate::ThumbnailError;
use occluview_render::Offscreen;

/// Unit tests use wgpu's fallback adapter; installed Windows builds prefer
/// hardware and let the renderer fall back when no suitable adapter exists.
pub(crate) const fn should_prefer_hardware_offscreen() -> bool {
    cfg!(all(windows, not(test)))
}

pub(crate) fn create_thumbnail_offscreen() -> Result<Offscreen, ThumbnailError> {
    if should_prefer_hardware_offscreen() {
        pollster::block_on(Offscreen::new_prefer_hardware()).map_err(Into::into)
    } else {
        pollster::block_on(Offscreen::new()).map_err(Into::into)
    }
}
