//! Shared triangle-surface queries for geometry kernels.
//!
//! This module owns the borrowed mesh view, deterministic nearest-surface index,
//! and cooperative cancellation flag used by alignment and contact measurement.

mod cancel;
mod index;
mod soup;

pub use cancel::{
    BuildOutcome, CancelFlag, GeometryControl, GeometryCounters, GeometryLimits, GeometryMemory,
    GeometryStop, QueryOutcome,
};
pub use index::{SurfaceHit, SurfaceIndex, SurfaceQueryHint, SurfaceQueryScratch};
pub use soup::Soup;

/// Alignment sampling support consumed by `occluview-align`, not part of the public API.
#[doc(hidden)]
pub use index::{feature_voxel_key, FeaturePoint, SurfaceSample, FEATURE_VOXEL_MM};
