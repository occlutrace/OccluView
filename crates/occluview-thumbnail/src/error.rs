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

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn variant_name(error: &ThumbnailError) -> &'static str {
        match error {
            ThumbnailError::Format(_) => "Format",
            ThumbnailError::Render(_) => "Render",
        }
    }

    #[test]
    fn every_thumbnail_error_variant_renders_an_operator_message() {
        let variants = [
            ThumbnailError::Format(occluview_formats::FormatError::Unsupported {
                extension: "xyz".to_string(),
            }),
            ThumbnailError::Render(occluview_render::RenderError::NoAdapter),
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
