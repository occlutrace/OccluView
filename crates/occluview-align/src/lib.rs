//! Scan-to-scan registration and deviation metrology for dental meshes.
//!
//! Search returns finite pose corrections and explicit missing evidence for
//! operator review through [`search_alignment`]. A result never authorizes a
//! scene edit. Pair fitting and legacy ICP remain numerical seed/refinement
//! primitives; their refusals become explanations at the search boundary.
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
//! numeric libraries: plain slices in, plain values out. Fixed random seeds,
//! iteration ceilings and ordered reductions make completed numeric work
//! reproducible. Cancellation and wall deadlines can return different finite
//! prefixes under different loads. The review boundary records unfinished
//! evidence; legacy indexing and nearest queries still need controlled work
//! accounting before their cancellation latency can be guaranteed.
//!
//! Units are millimetres. Every transform is rigid: dental scans are metric,
//! so a scale difference is *detected and reported*, never fitted away.
#![forbid(unsafe_code)]
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
mod search_control;
mod search_result;
mod search_transition;

pub use deviation::{
    deviation, deviation_colors, deviation_stats, ramp_color, suggested_scale_mm, DeviationMap,
    DeviationSettings, DeviationStats, DeviationSummary, RampMode, RampSettings, Unmeasured,
    Validity, MIN_MEASURED, NO_DATA_COLOR,
};
pub use icp::{refine, search_alignment, IcpReport, Orientation, RefineSettings};
pub use mask::{apply_brush, invert, set_all, MaskEdit, EXCLUDED, INCLUDED};
pub use observability::{observability, Observability};
pub use occluview_geometry::surface::{CancelFlag, Soup, SurfaceHit, SurfaceIndex};
pub use pairs::{fit_pairs, FitBounds, FitRejection, PairFit};
pub use rigid::Rigid;
pub use sample::bounds_of;

pub use search_control::SearchControl;
pub use search_result::*;
