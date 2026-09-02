//! Renderer error type.

use std::time::Duration;
use thiserror::Error;

/// Errors raised by the renderer.
#[derive(Debug, Error)]
pub enum RenderError {
    /// wgpu reported an error acquiring or presenting a surface.
    #[error("wgpu surface error: {0}")]
    Surface(String),

    /// A GPU readback did not complete before the liveness deadline.
    #[error("offscreen GPU readback timed out after {timeout:?}")]
    ReadbackTimeout {
        /// The finite wait that expired before the map callback arrived.
        timeout: Duration,
    },

    /// No suitable GPU adapter was found, and the software fallback is
    /// unavailable.
    #[error("no GPU adapter available and software fallback unavailable")]
    NoAdapter,
}
