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

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn variant_name(error: &ShellError) -> &'static str {
        match error {
            ShellError::Format(_) => "Format",
            ShellError::Render(_) => "Render",
            ShellError::Win32(_) => "Win32",
        }
    }

    #[test]
    fn every_shell_error_variant_renders_an_operator_message() {
        let variants = [
            ShellError::Format(occluview_formats::FormatError::Unsupported {
                extension: "xyz".to_string(),
            }),
            ShellError::Render(occluview_render::RenderError::NoAdapter),
            ShellError::Win32("the shell call failed".to_string()),
        ];
        for variant in &variants {
            assert!(!variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }
}
