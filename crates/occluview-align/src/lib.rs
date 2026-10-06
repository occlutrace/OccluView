//! Scan-to-scan registration and deviation metrology for dental meshes.
//!
//! [`search_alignment`] looks for the rigid motions that bring a moving scan
//! onto a fixed one and returns them as candidates for the operator's review,
//! each with a confidence class and the evidence behind it. Scans that are
//! only partly alike are the ordinary case: the search assumes no common
//! centre, extent or outline. A result never edits a scene; accepting a
//! candidate is a separate act of the application. [`fit_pairs`] fits clicked
//! point pairs and names every reason it refuses.
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
//! The kernels depend only on product-neutral surface queries and numeric
//! libraries: plain slices in, plain values out. Every stage does a fixed
//! amount of work in a fixed order and sums in a fixed order, so completed
//! work is reproducible; a wall deadline or a cancellation can end a search
//! early, and the result then says so.
//!
//! Units are millimetres. Every correction is rigid; finite authored scale
//! and shear stay in the immutable input view and are never fitted away.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::float_cmp))]

mod confidence;
mod deviation;
mod deviation_display;
mod mask;
mod observability;
#[cfg(test)]
mod observability_tests;
mod pairs;
#[cfg(test)]
mod pairs_tests;
mod registration;
mod rigid;
mod sample;
mod search;
mod search_control;
mod search_result;

pub use deviation::{
    deviation, deviation_colors, deviation_stats, ramp_color, suggested_scale_mm, DeviationMap,
    DeviationSettings, DeviationStats, DeviationSummary, Orientation, RampMode, RampSettings,
    Unmeasured, Validity, MIN_MEASURED, NO_DATA_COLOR,
};
pub use deviation_display::{display_map, DISPLAY_PASSES};
pub use mask::{apply_brush, eligible_region, invert, set_all, MaskEdit, EXCLUDED, INCLUDED};
pub use observability::{observability, Observability};
pub use occluview_geometry::surface::{CancelFlag, Soup, SurfaceHit, SurfaceIndex};
pub use pairs::{fit_pairs, FitBounds, FitRejection, PairFit};
pub use rigid::Rigid;
pub use sample::bounds_of;
pub use search::search_alignment;
pub use search_control::SearchControl;
pub use search_result::*;
