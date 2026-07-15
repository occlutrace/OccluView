//! Shell-extension error type.

use thiserror::Error;

/// Errors raised by the shell extension. These always result in a **placeholder
/// thumbnail** being returned to Windows, never a propagated crash
/// shell host.
#[derive(Debug, Error)]
pub enum ShellError {
    /// The file did not match any supported format.
    #[error(transparent)]
    Format(#[from] occluview_formats::FormatError),

    /// The renderer failed (no adapter, shader error, watchdog timeout).
    #[error(transparent)]
    Render(#[from] occluview_render::RenderError),

    /// A Windows API call failed inside the COM layer.
    #[error("win32 error: {0}")]
    Win32(String),
}

impl From<occluview_thumbnail::ThumbnailError> for ShellError {
    fn from(error: occluview_thumbnail::ThumbnailError) -> Self {
        match error {
            occluview_thumbnail::ThumbnailError::Format(error) => Self::Format(error),
            occluview_thumbnail::ThumbnailError::Render(error) => Self::Render(error),
        }
    }
}
