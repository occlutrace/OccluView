use crate::ThumbnailError;
use occluview_render::Offscreen;

/// Whether this build should ask wgpu for a hardware adapter.
///
/// Installed code prefers hardware everywhere. It used to prefer it on Windows
/// alone, which left every other desktop rendering scans on the software
/// rasteriser: on a folder of 120 real scans that was 8.3 s serialised against
/// 5.9 s on the machine's own GPU, and a median file of 51 ms against 35 ms.
///
/// The reason for the caution was real, though -- an adapter can accept every
/// command and then produce an empty target, which is what a runner's nominal
/// GPU does -- so the preference is now checked rather than assumed: the
/// factory below draws one triangle and demotes a device that cannot.
///
/// Unit tests still take the fallback adapter, so a suite never depends on
/// whatever GPU the machine running it happens to have.
///
/// The same function exists in `occluview-shell`, and has to: `cfg!(test)` is
/// evaluated per crate, so a shared definition reports "not under test" while
/// this crate's own tests run -- the case the fallback exists for. The two
/// bodies must stay identical, and a contract test says so.
pub(crate) const fn should_prefer_hardware_offscreen() -> bool {
    !cfg!(test)
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
    if !should_prefer_hardware_offscreen() {
        return None;
    }
    let offscreen = pollster::block_on(Offscreen::new_prefer_hardware()).ok()?;
    if pollster::block_on(offscreen.can_draw()) {
        return Some(offscreen);
    }
    tracing::warn!("hardware adapter drew nothing; falling back to the software rasteriser");
    None
}
