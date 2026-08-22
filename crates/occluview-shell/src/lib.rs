//! `occluview-shell` — Windows COM shell extension for OccluView.
//!
//! Implements `IThumbnailProvider` as an **out-of-process** COM server: Windows
//! hosts this DLL in `dllhost.exe`, so a bug or a malicious file cannot crash
//! `explorer.exe`.
//!
//! The thumbnail reuses [`occluview_render`] offscreen path, so it is
//! pixel-identical to the in-app frame — one shader, one camera, one loader.
//!
//! ## Status
//!
//! `render_thumbnail` is the platform-agnostic render entry point, fully tested
//! on Linux. The COM class (`com::ThumbnailProvider`), class factory,
//! `DllGetClassObject`/`DllCanUnloadNow`, and `DllRegisterServer`/
//! `DllUnregisterServer` live in `com.rs` and the `registration` module, all `cfg(windows)`
//! — they require the windows toolchain to compile but the rest of the crate
//! builds on any host.

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
mod offscreen_factory;
#[cfg(any(windows, test))]
mod preview_menu;
mod preview_scene;
mod shell_contract;
#[cfg(test)]
mod shell_contract_tests;
#[cfg(test)]
mod shell_preview_tests;
#[cfg(any(windows, test))]
mod stream_read {
    #[allow(unused_imports)]
    pub(crate) use occluview_thumbnail::stream_read::{read_capped_stream, StreamRead};
}
#[cfg(test)]
mod test_support;

pub(crate) mod fast_thumb {
    pub(crate) use occluview_thumbnail::fast_thumb::*;
}

pub mod placeholder {
    //! Compatibility re-exports for deterministic thumbnail placeholders.
    pub use occluview_thumbnail::placeholder::*;
}

pub mod thumbnail_format {
    //! Compatibility re-exports for thumbnail format inference.
    pub use occluview_thumbnail::thumbnail_format::*;
}

pub mod thumbnail_timeout {
    //! Compatibility re-exports for thumbnail timeout helpers.
    pub use occluview_thumbnail::thumbnail_timeout::*;
}

/// Compatibility namespace for the pre-extraction thumbnail APIs.
pub mod render_thumb {
    use super::ShellError;
    use occluview_render::ThumbnailSpec;
    use std::path::Path;

    pub use occluview_thumbnail::render_thumb::{
        placeholder_for_oversize_input, prewarm_thumbnail_renderer,
        render_thumbnail_file_or_placeholder, render_thumbnail_file_or_placeholder_with_timeout,
        render_thumbnail_or_placeholder, render_thumbnail_or_placeholder_with_timeout,
        render_thumbnail_shared_or_placeholder_with_timeout, reserve_thumbnail_stream_job,
        try_render_thumbnail_file, try_render_thumbnail_shared,
        try_render_thumbnail_shared_with_reservation, ThumbnailAttempt, ThumbnailJobReservation,
        DEFAULT_THUMBNAIL_TIMEOUT, MAX_THUMBNAIL_FILE_BYTES, MAX_THUMBNAIL_INPUT_BYTES,
    };

    /// Render a thumbnail from bytes, preserving the shell error type.
    ///
    /// # Errors
    /// Returns [`ShellError`] when format loading or offscreen rendering fails.
    pub fn render_thumbnail(
        extension: &str,
        bytes: &[u8],
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, ShellError> {
        occluview_thumbnail::render_thumbnail(extension, bytes, spec).map_err(Into::into)
    }

    /// Render a thumbnail from bytes with an optional extension hint.
    ///
    /// # Errors
    /// Returns [`ShellError`] when format loading or offscreen rendering fails.
    pub fn render_thumbnail_bytes(
        extension: Option<&str>,
        bytes: &[u8],
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, ShellError> {
        occluview_thumbnail::render_thumbnail_bytes(extension, bytes, spec).map_err(Into::into)
    }

    /// Render a thumbnail from a local file, preserving the shell error type.
    ///
    /// # Errors
    /// Returns [`ShellError`] when file loading or offscreen rendering fails.
    pub fn render_thumbnail_file(path: &Path, spec: ThumbnailSpec) -> Result<Vec<u8>, ShellError> {
        occluview_thumbnail::render_thumbnail_file(path, spec).map_err(Into::into)
    }
}

#[cfg(windows)]
pub mod com;

#[cfg(windows)]
pub mod registration;

pub use error::ShellError;
pub use occluview_formats::{LEGACY_HPS_EXTENSION, V1_OPEN_EXTENSIONS};
pub use placeholder::{placeholder_thumbnail, placeholder_thumbnail_kind, PlaceholderKind};
pub use render_thumb::{render_thumbnail, render_thumbnail_bytes, render_thumbnail_or_placeholder};
pub use shell_contract::{
    APP_EXE_NAME, DEDICATED_FILE_ICON_EXTENSIONS, PREVIEW_HANDLER_CATEGORY, SUPPORTED_EXTENSIONS,
    THUMBNAIL_PROVIDER_CATEGORY,
};

#[cfg(windows)]
pub use registration::notify_shell_associations_changed;

#[cfg(test)]
pub(crate) use test_support::acquire_render_test_guard;

/// No-op shell refresh stub on non-Windows hosts.
#[cfg(not(windows))]
pub fn notify_shell_associations_changed() {}
