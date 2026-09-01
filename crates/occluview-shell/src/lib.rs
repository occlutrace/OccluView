//! `occluview-shell` — Windows COM shell extension for OccluView.
//!
//! Implements `IThumbnailProvider` as an out-of-process COM server hosted by
//! Windows in `dllhost.exe`.
//!
//! The thumbnail reuses [`occluview_render`] offscreen path — one shader, one
//! camera — so a given mesh rasterizes exactly as it does in the app. Large
//! files are the exception by design: past the fidelity cutoffs in
//! [`occluview_thumbnail`] the tile is drawn from a decimated preview mesh
//! through the same pipeline.
//!
#![cfg_attr(not(test), deny(unsafe_code))]
#![cfg_attr(test, allow(clippy::expect_used))]
// The COM class (`com.rs`) is `unsafe` by definition (FFI + raw pointers across
// the COM ABI). Its module-level `#![allow(unsafe_code)]` overrides this gate
// under `cfg(windows)` only; the platform-agnostic code stays panic-free and
// unsafe-free. We use `deny` rather than `forbid` precisely so the Windows COM
// module can relax it — `forbid` is unreleasable.

#[cfg(any(windows, test))]
mod deferred_source;
pub mod error;
#[cfg(test)]
mod installer_contract_tests;
mod offscreen_factory;
#[cfg(any(windows, test))]
mod preview_canvas;
#[cfg(any(windows, test))]
mod preview_menu;
mod preview_scene;
#[cfg(test)]
mod shell_adapter_contract_tests;
mod shell_contract;
#[cfg(test)]
mod shell_contract_tests;
#[cfg(any(test, all(windows, feature = "diagnostic-logs")))]
mod shell_diagnostics;
#[cfg(test)]
mod shell_preview_render_tests;
#[cfg(test)]
mod shell_preview_tests;
#[cfg(any(windows, test))]
mod stream_read {
    #[allow(unused_imports)]
    pub(crate) use occluview_thumbnail::stream_read::{
        read_capped_stream_until, StreamRead, StreamReadBounds,
    };
}
#[cfg(test)]
mod test_support;

#[cfg(windows)]
pub mod com;

#[cfg(windows)]
pub mod registration;

pub use error::ShellError;
pub use shell_contract::{
    owns_extension, APP_EXE_NAME, DEDICATED_FILE_ICON_EXTENSIONS, OFFERED_ONLY_EXTENSIONS,
    PREVIEW_HANDLER_CATEGORY, SUPPORTED_EXTENSIONS, THUMBNAIL_PROVIDER_CATEGORY,
};

#[cfg(windows)]
pub use registration::notify_shell_associations_changed;

#[cfg(test)]
pub(crate) use test_support::acquire_render_test_guard;

/// No-op shell refresh stub on non-Windows hosts.
#[cfg(not(windows))]
pub fn notify_shell_associations_changed() {}
