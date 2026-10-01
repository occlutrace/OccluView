//! Shared triangle-surface queries for geometry kernels.
//!
//! The crate owns the borrowed mesh view, deterministic nearest-surface index,
//! and cooperative cancellation flag used by alignment and contact measurement.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

mod cancel;
mod soup;
mod surface;

pub use cancel::CancelFlag;
pub use soup::Soup;
pub use surface::{SurfaceHit, SurfaceIndex};

/// Alignment sampling support shared with the surface index implementation.
#[doc(hidden)]
pub use surface::{feature_voxel_key, FeaturePoint, SurfaceSample, FEATURE_VOXEL_MM};

#[cfg(test)]
mod workspace_layers_tests;
