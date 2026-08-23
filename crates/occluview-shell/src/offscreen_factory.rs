use crate::ShellError;
use occluview_render::Offscreen;
use std::sync::{Arc, Mutex, OnceLock};

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
/// The same function exists in `occluview-thumbnail`, and has to: `cfg!(test)` is
/// evaluated per crate, so a shared definition reports "not under test" while
/// this crate's own tests run -- the case the fallback exists for. The two
/// bodies must stay identical, and a contract test says so.
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
///
/// `prevhost.exe` keeps the preview handler process alive between file
/// selections, but every click builds a fresh `PreviewSceneState`. Creating a
/// wgpu instance + adapter + device and compiling both WGSL shader modules per
/// click was the dominant fixed cost of first paint; one shared renderer pays
/// it once per process. The slot mutex is held across creation so a concurrent
/// caller waits for the winner instead of racing a second device into
/// existence.
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
///
/// Called from a background thread when the preview class is first activated,
/// so device + shader-module creation overlaps prevhost's Initialize instead
/// of adding to the first paint. Errors are swallowed: the first real preview
/// load repeats the attempt and owns the error path.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn prewarm_shared_shell_offscreen() {
    let _ = shared_shell_offscreen();
}

/// Retire the shared renderer after a render on it failed.
///
/// A lost device (driver reset, TDR) stays lost; keeping it cached would turn
/// one GPU hiccup into a permanently broken preview pane. Only the exact
/// renderer that failed is dropped, so a healthy replacement created by a
/// concurrent load is never discarded by a stale error report.
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
