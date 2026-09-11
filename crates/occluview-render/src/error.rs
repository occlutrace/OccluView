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
    ///
    /// Doc note: this is the only variant that does not imply the graphics
    /// stack is unusable. A deadline can pass on a loaded machine with a
    /// perfectly healthy device, which is why the application retries it
    /// instead of latching its offscreen path off for the rest of the session.
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
