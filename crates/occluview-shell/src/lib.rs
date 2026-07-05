//! `occluview-shell` — Windows COM shell extension for OccluView.
//!
//! Implements `IThumbnailProvider` as an **out-of-process** COM server: Windows
//! hosts this DLL in `dllhost.exe`, so a bug or a malicious file cannot crash
//! `explorer.exe` (ADR-0005 + addendum, `docs/SHELL_INTEGRATION.md`).
//!
//! The thumbnail reuses [`occluview_render`] offscreen path, so it is
//! pixel-identical to the in-app frame — one shader, one camera, one loader.
//!
//! ## Status
//!
//! `render_thumbnail` is the platform-agnostic render entry point, fully tested
//! on Linux. The COM class (`com::ThumbnailProvider`), class factory,
//! `DllGetClassObject`/`DllCanUnloadNow`, and `DllRegisterServer`/
//! `DllUnregisterServer` live in `com.rs` and `registration.rs`, all `cfg(windows)`
//! — they require the windows toolchain to compile but the rest of the crate
//! builds on any host.

#![cfg_attr(not(test), deny(unsafe_code))]
// The COM class (`com.rs`) is `unsafe` by definition (FFI + raw pointers across
// the COM ABI). Its module-level `#![allow(unsafe_code)]` overrides this gate
// under `cfg(windows)` only; the platform-agnostic code stays panic-free and
// unsafe-free. We use `deny` rather than `forbid` precisely so the Windows COM
// module can relax it — `forbid` is unreleasable.

pub mod error;
pub mod render_thumb;
pub mod thumbnail_format;

#[cfg(windows)]
pub mod com;

#[cfg(windows)]
pub mod registration;

pub use error::ShellError;
pub use render_thumb::{render_thumbnail, render_thumbnail_bytes};

/// The CLSID string for the OccluView thumbnail provider.
///
/// Registered under
/// `HKCR\.<ext>\ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}` for each
/// supported extension. (The literal `{E357FCCD-A995-4576-B01F-234630154E96}`
/// is the shell's `IThumbnailProvider` category, not our own CLSID — our own
/// CLSID is generated when the COM class lands.)
pub const THUMBNAIL_PROVIDER_CATEGORY: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";

/// File extensions OccluView registers a thumbnail provider for.
///
/// JSON `.gltf` and `.3mf` are deliberately absent until their stream-safe
/// readers exist. Registering a shell handler for deferred formats makes
/// Explorer thumbnails look broken.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["stl", "ply", "obj", "glb"];

#[cfg(test)]
mod tests {
    use super::SUPPORTED_EXTENSIONS;

    #[test]
    fn thumbnail_registration_only_includes_implemented_stream_formats() {
        assert_eq!(SUPPORTED_EXTENSIONS, ["stl", "ply", "obj", "glb"]);
    }
}
