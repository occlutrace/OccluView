//! Renderer error type.

use thiserror::Error;

/// Errors raised by the renderer.
#[derive(Debug, Error)]
pub enum RenderError {
    /// wgpu reported an error acquiring or presenting a surface.
    #[error("wgpu surface error: {0}")]
    Surface(String),

    /// No suitable GPU adapter was found, and the software fallback is
    /// unavailable.
    #[error("no GPU adapter available and software fallback unavailable")]
    NoAdapter,

    /// A shader failed to compile (WGSL parse/validation error).
    #[error("shader compilation failed: {0}")]
    Shader(String),

    /// The offscreen render exceeded its time budget.
    #[error("render timed out after {ms} ms")]
    Timeout {
        /// Elapsed milliseconds when the watchdog fired.
        ms: u32,
    },
}
