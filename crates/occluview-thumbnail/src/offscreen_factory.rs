use crate::ThumbnailError;
use occluview_render::Offscreen;

/// Whether this build should ask wgpu for a hardware adapter.
///
/// Unit tests must take the fallback adapter: GitHub's Windows runner can
/// expose a nominal hardware adapter that accepts commands and then produces
/// an empty headless render target. Installed code prefers hardware and lets
/// the renderer fall back when no suitable adapter exists.
///
/// Deliberately a copy of the same function in `occluview-shell`. `cfg!(test)` is
/// evaluated per crate: a shared definition would report "not under test"
/// while this crate's own tests run, which is the case the fallback exists
/// for. The two bodies must stay identical, and a contract test says so.
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
