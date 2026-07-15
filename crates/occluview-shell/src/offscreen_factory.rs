use crate::ShellError;
use occluview_render::Offscreen;

/// Unit tests must use wgpu's fallback adapter: GitHub's Windows runner may
/// expose a nominal hardware adapter that accepts commands but produces an
/// empty headless render target. Installed shell code still prefers hardware
/// and falls back inside the renderer when one is unavailable.
pub(crate) const fn should_prefer_hardware_offscreen() -> bool {
    cfg!(all(windows, not(test)))
}

pub(crate) fn create_shell_offscreen() -> Result<Offscreen, ShellError> {
    if should_prefer_hardware_offscreen() {
        pollster::block_on(Offscreen::new_prefer_hardware()).map_err(Into::into)
    } else {
        pollster::block_on(Offscreen::new()).map_err(Into::into)
    }
}
