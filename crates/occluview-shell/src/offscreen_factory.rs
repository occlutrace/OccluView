use crate::ShellError;
use occluview_render::Offscreen;
use std::sync::{Arc, Mutex, OnceLock};

/// Whether this build should ask wgpu for a hardware adapter. Tests use the
/// software fallback so they do not depend on the host GPU.
///
/// The thumbnail crate has the same per-crate definition because `cfg!(test)`
/// is evaluated independently for each crate.
pub(crate) const fn should_prefer_hardware_offscreen() -> bool {
    !cfg!(test)
}

pub(crate) fn create_shell_offscreen() -> Result<Offscreen, ShellError> {
    if let Some(offscreen) = hardware_offscreen_that_draws() {
        return Ok(offscreen);
    }
    pollster::block_on(Offscreen::new()).map_err(Into::into)
}

/// A hardware renderer, but only if it can put a triangle on the screen.
///
/// A device that accepts commands and renders nothing would otherwise answer
/// every scan with a blank preview pane.
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

/// One offscreen renderer per host process, shared across preview loads.
/// Creation is serialized so concurrent callers reuse the same device.
static SHARED_SHELL_OFFSCREEN: OnceLock<Mutex<Option<Arc<Offscreen>>>> = OnceLock::new();

fn shared_shell_offscreen_slot() -> &'static Mutex<Option<Arc<Offscreen>>> {
    SHARED_SHELL_OFFSCREEN.get_or_init(|| Mutex::new(None))
}

pub(crate) fn shared_shell_offscreen() -> Result<Arc<Offscreen>, ShellError> {
    let mut slot = shared_shell_offscreen_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(offscreen) = slot.as_ref() {
        return Ok(offscreen.clone());
    }
    let offscreen = Arc::new(create_shell_offscreen()?);
    *slot = Some(offscreen.clone());
    Ok(offscreen)
}

/// Warm the shared preview renderer ahead of the first `DoPreview`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn prewarm_shared_shell_offscreen() {
    let _ = shared_shell_offscreen();
}

/// Retire the shared renderer after a render on it failed. A replacement
/// created concurrently is preserved.
pub(crate) fn discard_shared_shell_offscreen(sick: &Arc<Offscreen>) {
    let mut slot = shared_shell_offscreen_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, sick))
    {
        *slot = None;
    }
}
