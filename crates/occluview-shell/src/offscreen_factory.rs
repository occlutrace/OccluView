use crate::ShellError;
use occluview_render::{AdapterPolicy, Offscreen, RenderDeadline};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const PREVIEW_RENDERER_SETUP_TIMEOUT: Duration = Duration::from_secs(8);

const fn shell_adapter_policy() -> AdapterPolicy {
    AdapterPolicy::HardwareThenFallback
}

const fn preview_adapter_policy() -> AdapterPolicy {
    if cfg!(test) {
        AdapterPolicy::FallbackOnly
    } else {
        shell_adapter_policy()
    }
}

pub(crate) fn create_shell_offscreen() -> Result<Offscreen, ShellError> {
    pollster::block_on(Offscreen::new_with_adapter_policy(
        preview_adapter_policy(),
        RenderDeadline::after(PREVIEW_RENDERER_SETUP_TIMEOUT),
    ))
    .map_err(Into::into)
}

/// One offscreen renderer per host process, shared across preview loads.
/// Creation is serialized so concurrent callers reuse the same device.
static SHARED_SHELL_OFFSCREEN: OnceLock<Mutex<Option<Arc<Offscreen>>>> = OnceLock::new();

fn shared_shell_offscreen_slot() -> &'static Mutex<Option<Arc<Offscreen>>> {
    SHARED_SHELL_OFFSCREEN.get_or_init(|| Mutex::new(None))
}

pub(crate) fn shared_shell_offscreen() -> Result<Arc<Offscreen>, ShellError> {
    let existing = shared_shell_offscreen_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .cloned();
    if let Some(offscreen) = existing {
        return Ok(offscreen.clone());
    }

    let offscreen = Arc::new(create_shell_offscreen()?);
    let mut slot = shared_shell_offscreen_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(slot.get_or_insert(offscreen).clone())
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
