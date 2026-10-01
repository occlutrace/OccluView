//! Surface sculpting kernel for dental scan meshes.
//!
//! The crate owns the geometry work behind an interactive brush: tip stamps
//! (ball, knife, cylinder), the clay/smooth/flatten dab solvers, live remesh
//! (split, collapse, flip and the isotropic cycle that drives them), local
//! densification, smoothing and fairing operators, sparse stroke history, and
//! the mutable [`sculpt_session::SculptSession`] that ties them together. It
//! has no UI, renderer or file-format dependency; callers hand it a triangle
//! mesh in millimetres and read back moved vertices, rewritten faces and a
//! topology journal they can reverse.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::panic))]
// The kernel computes in f64 millimetres over f32 vertex storage, so every
// remesh step crosses that boundary deliberately: the cast family is the
// representation itself, and the exact-zero float comparisons are degeneracy
// tests on coordinates that are stored at f32 precision. The session is one
// implementation split across files, so its submodules share the parent's item
// set rather than restating a long import list per file. Lints about hot-path
// naming and aggregate shape are the same statement. Each of these would
// otherwise need a local exception at most expressions of the kind.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::wildcard_imports,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::struct_excessive_bools,
    clippy::type_complexity
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic,))]

use glam::DVec3;

mod fairing;
mod flatten;
mod hash;
mod knot;
mod remesh_policy;
mod respace;
pub mod sculpt_session;
mod shape_preserve;
mod surface_topology;
mod tip_stamp;

pub use fairing::{
    fair_selection, fair_selection_preserving, fair_selection_with_constraint, smoothing_scale_mm,
    FairingContact, FairingContactStats, FairingScratch, FairingSurface,
};
pub use flatten::{flatten_displacement, FlattenSurface};

/// Standard triangle quality: 1 for equilateral, 0 for degenerate.
///
/// The 3D form is defined once here; a caller that already holds the double
/// area keeps its own inline form rather than recomputing it.
#[must_use]
pub fn triangle_quality_3d(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let ab = (b - a).length();
    let bc = (c - b).length();
    let ca = (a - c).length();
    let area = (b - a).cross(c - a).length() / 2.0;
    let denom = ab * ab + bc * bc + ca * ca;
    if denom <= 1e-18 {
        0.0
    } else {
        (4.0 * 3.0f64.sqrt() * area / denom).clamp(0.0, 1.0)
    }
}

/// Minimum triangle quality a split child may have (1 = equilateral, 0 =
/// degenerate).
pub const SLIVER_QUALITY_FLOOR: f64 = 0.25;
pub use knot::{clamp_dab_displacement, KnotSurface, KNOT_TANGENT_SHARE};
pub use occlu_geometry_math::{
    ball_weight, cylinder_weight, knife_weight, stamp_weight, TipStamp, CYLINDER_PLATEAU,
    KNIFE_CROSS_RADIUS_SHARE,
};
pub use remesh_policy::{RemeshPolicy, TopologyRevision};
pub use respace::{tangential_respace_target, RespaceSurface, RESPACE_GAIN};
pub use sculpt_session::*;
pub use shape_preserve::{preserve_alpha, DEFAULT_PRESERVE_RINGS};
pub use surface_topology::{SurfacePoint, SurfaceTopology};
pub use tip_stamp::segment_stamp_weight;
