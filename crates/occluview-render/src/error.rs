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

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn variant_name(error: &RenderError) -> &'static str {
        match error {
            RenderError::Surface(_) => "Surface",
            RenderError::ReadbackTimeout { .. } => "ReadbackTimeout",
            RenderError::NoAdapter => "NoAdapter",
        }
    }

    #[test]
    fn every_render_error_variant_renders_an_operator_message() {
        let variants = [
            RenderError::Surface("the surface was lost".to_string()),
            RenderError::ReadbackTimeout {
                timeout: Duration::from_secs(5),
            },
            RenderError::NoAdapter,
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
