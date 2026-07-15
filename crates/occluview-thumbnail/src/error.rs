//! Errors produced by the platform-neutral thumbnail pipeline.

use thiserror::Error;

/// Errors raised while loading mesh data or rendering a thumbnail.
#[derive(Debug, Error)]
pub enum ThumbnailError {
    /// The input did not load through the shared format readers.
    #[error(transparent)]
    Format(#[from] occluview_formats::FormatError),

    /// The offscreen renderer failed.
    #[error(transparent)]
    Render(#[from] occluview_render::RenderError),
}
