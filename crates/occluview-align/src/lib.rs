//! Scan-to-scan registration and deviation metrology for dental meshes.
//!
//! The pipeline has three stages:
//! 1. Align clicked surface point pairs with a rigid fit.
//! 2. Refine the pose with trimmed point-to-plane ICP.
//! 3. Produce signed deviations along the fixed surface normal.
//!
//! # What the deviation number means
//!
//! A deviation map measures the distance from each moving vertex to the
//! **nearest point on the fixed surface**. This is not a material
//! correspondence:
//!
//! * It is **one-sided**. Fixed surface the moving scan never covered is not
//!   measured at all, so a scan with a hole in it can report a perfect fit. What
//!   the map could not reach is reported as such — see [`Unmeasured`] — rather
//!   than folded into the numbers.
//! * It is a **lower bound on displacement**. Tangential motion slides the
//!   nearest point along the surface instead of moving away from it. A 0.30 mm
//!   rigid offset of a real arch reads as 0.14 mm; on a cylinder slid along its
//!   own axis it reads 0.0075 mm. Symmetry does not fix this and nothing
//!   derived from surface distance can. [`observability()`] reports how much of a
//!   displacement this particular pair of surfaces converts into distance, and
//!   [`Observability::hidden_displacement_mm`] turns a reported RMS into the
//!   largest true displacement that could be hiding behind it.
//!
//! Report [`deviation_stats`] with [`observability()`] and include the
//! unmeasured counts. The distance alone is a lower bound.
//!
//! The alignment kernels depend only on product-neutral surface queries and
//! numeric libraries: plain slices in, plain values out. They never allocate
//! unboundedly, never panic on hostile input, and are deterministic — no RNG,
//! fixed iteration counts, ordered reductions — so the same input yields
//! bit-identical output across runs and thread counts.
//!
//! Units are millimetres. Every transform is rigid: dental scans are metric,
//! so a scale difference is *detected and reported*, never fitted away.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::float_cmp))]

mod deviation;
mod icp;
#[cfg(test)]
mod icp_tests;
mod mask;
mod observability;
#[cfg(test)]
mod observability_tests;
mod pairs;
#[cfg(test)]
mod pairs_tests;
mod rigid;
mod sample;

pub use deviation::{
    deviation, deviation_colors, deviation_stats, ramp_color, suggested_scale_mm, DeviationMap,
    DeviationSettings, DeviationStats, DeviationSummary, RampMode, RampSettings, Unmeasured,
    Validity, MIN_MEASURED, NO_DATA_COLOR,
};
pub use icp::{refine, IcpReport, Orientation, RefineSettings};
pub use mask::{apply_brush, invert, set_all, MaskEdit, EXCLUDED, INCLUDED};
pub use observability::{observability, Observability};
pub use occluview_surface_query::{CancelFlag, Soup, SurfaceHit, SurfaceIndex};
pub use pairs::{fit_pairs, FitBounds, FitRejection, PairFit};
pub use rigid::Rigid;
pub use sample::bounds_of;
