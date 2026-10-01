//! Shell-extension error type.

use thiserror::Error;

/// Errors raised by the shell extension.
///
/// What Windows sees depends on the surface that raised one. The thumbnail
/// provider answers a transient failure with a failure `HRESULT` rather than a
/// bitmap, because Explorer's thumbcache stores any bitmap answered with `S_OK`
/// and a "busy right now" placeholder would freeze into the file's icon; a
/// deterministic verdict, such as an over-budget or undecodable file, is a
/// placeholder the shell may cache (`com::thumbnail_provider` documents both).
/// The preview pane draws a placeholder. Neither surface propagates a panic
/// across the COM boundary.
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
