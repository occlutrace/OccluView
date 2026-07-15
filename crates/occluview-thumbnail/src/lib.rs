//! Platform-neutral thumbnail loading, rendering, caching, and fallback APIs.

#![cfg_attr(not(test), deny(unsafe_code))]
#![cfg_attr(test, allow(clippy::expect_used))]

pub mod error;
pub mod fast_thumb;
mod offscreen_factory;
pub mod placeholder;
pub mod render_thumb;
/// Bounded helpers for copying shell-provided streams into memory.
pub mod stream_read;
pub mod thumbnail_format;
pub mod thumbnail_timeout;

pub use error::ThumbnailError;
pub use occluview_formats::V1_OPEN_EXTENSIONS as SUPPORTED_EXTENSIONS;
pub use occluview_render::ThumbnailSpec;
pub use placeholder::{placeholder_thumbnail, placeholder_thumbnail_kind, PlaceholderKind};
pub use render_thumb::{
    render_thumbnail, render_thumbnail_bytes, render_thumbnail_file,
    render_thumbnail_file_or_placeholder, render_thumbnail_file_or_placeholder_with_timeout,
    render_thumbnail_or_placeholder, render_thumbnail_or_placeholder_with_timeout,
};

#[cfg(test)]
mod test_support;

#[cfg(test)]
pub(crate) use test_support::acquire_render_test_guard;
