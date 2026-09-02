//! Cache-safe placeholder verdicts for thumbnail requests.

use crate::placeholder::{placeholder_thumbnail, PlaceholderKind};
use crate::ThumbnailError;
use occluview_formats::FormatError;
use occluview_render::ThumbnailSpec;

use super::cache::oversize_input_error;

/// Return the policy placeholder for an input that exceeds the size ceiling.
pub fn placeholder_for_oversize_input(spec: ThumbnailSpec, byte_len: usize) -> Vec<u8> {
    let error = oversize_input_error(byte_len);
    tracing::warn!(
        ?error,
        byte_len,
        "thumbnail input exceeded size policy; returning placeholder"
    );
    // Over-budget is a policy decision, not a broken file: quiet plain cube.
    placeholder_thumbnail(spec)
}

/// Pick the placeholder flavor for a thumbnail failure.
///
/// A *recognized* format that fails to decode gets the corrupt badge. Inputs
/// that are unsupported, deferred, over policy, I/O failures, or render
/// failures remain a quiet cube because retrying may still make them usable.
pub(super) fn placeholder_kind_for_error(error: &ThumbnailError) -> PlaceholderKind {
    match error {
        ThumbnailError::Format(format_error) => match format_error {
            FormatError::Malformed { format, .. } if format.starts_with("thumbnail") => {
                PlaceholderKind::Plain
            }
            FormatError::BadSignature { .. }
            | FormatError::Truncated { .. }
            | FormatError::Malformed { .. }
            | FormatError::Core(_) => PlaceholderKind::Corrupt,
            FormatError::Unsupported { .. }
            | FormatError::Deferred { .. }
            | FormatError::UnsafePath { .. }
            | FormatError::Io(_) => PlaceholderKind::Plain,
        },
        ThumbnailError::Render(_) => PlaceholderKind::Plain,
    }
}
