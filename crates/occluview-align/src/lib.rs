//! Scan-to-scan registration and deviation metrology for dental meshes.
//!
//! Three stages, mirroring how lab and metrology software does it:
//!   1. ALIGN  — a rigid fit from clicked surface point pairs;
//!   2. REFINE — trimmed point-to-plane ICP against the fixed surface;
//!   3. PROOF  — a signed deviation map (± mm along the fixed normal).
//!
//! The crate is a leaf: plain slices in, plain values out. It never allocates
//! unboundedly, never panics on hostile input, and is deterministic — no RNG,
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
mod pairs;
#[cfg(test)]
mod pairs_tests;
mod rigid;
mod sample;
mod surface;

pub use deviation::{
    deviation, deviation_colors, deviation_stats, ramp_color, suggested_scale_mm, DeviationMap,
    DeviationSettings, DeviationStats, RampMode, RampSettings, Validity, NO_DATA_COLOR,
};
pub use icp::{refine, IcpReport, Orientation, RefineSettings};
pub use mask::{apply_brush, invert, mark_around, set_all, MaskEdit, EXCLUDED, INCLUDED};
pub use pairs::{fit_pairs, FitRejection, PairFit};
pub use rigid::Rigid;
pub use surface::{SurfaceHit, SurfaceIndex};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cooperative cancellation shared with a long-running registration job.
///
/// Every stage checks it at a bounded interval and returns what it has so far
/// rather than abandoning the caller mid-computation.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A fresh, un-cancelled flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask every holder of this flag to stop at its next checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A borrowed triangle mesh: xyz triples, triangle indices, and an optional
/// per-vertex exclusion mask where a non-zero byte means "excluded from
/// matching".
#[derive(Clone, Copy, Debug)]
pub struct Soup<'a> {
    /// Vertex positions as consecutive xyz triples.
    pub positions: &'a [f32],
    /// Triangle indices into `positions`, three per triangle.
    pub indices: &'a [u32],
    /// Optional per-vertex exclusion mask; `None` means nothing is excluded.
    pub mask: Option<&'a [u8]>,
}

impl Soup<'_> {
    /// Number of whole vertices, ignoring any trailing partial triple.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of whole triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether `vertex` is excluded from matching by the mask.
    #[must_use]
    pub fn is_excluded(&self, vertex: usize) -> bool {
        self.mask
            .is_some_and(|mask| mask.get(vertex).copied().unwrap_or(0) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CancelFlag, Soup};

    #[test]
    fn a_fresh_cancel_flag_is_not_cancelled_and_a_clone_shares_it() {
        let flag = CancelFlag::new();
        let echo = flag.clone();
        assert!(!flag.is_cancelled());
        echo.cancel();
        assert!(flag.is_cancelled(), "cancellation must reach every clone");
    }

    #[test]
    fn a_soup_counts_whole_vertices_and_triangles_only() {
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 9.0];
        let indices = [0, 1, 2, 0, 1];
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        assert_eq!(
            soup.vertex_count(),
            3,
            "a trailing partial triple is not a vertex"
        );
        assert_eq!(
            soup.triangle_count(),
            1,
            "a trailing partial triangle is not a triangle"
        );
    }

    #[test]
    fn a_missing_or_short_mask_excludes_nothing() {
        let positions = [0.0; 9];
        let indices = [0, 1, 2];
        let short = [1u8];
        assert!(!Soup {
            positions: &positions,
            indices: &indices,
            mask: None
        }
        .is_excluded(0));
        let masked = Soup {
            positions: &positions,
            indices: &indices,
            mask: Some(&short),
        };
        assert!(masked.is_excluded(0));
        assert!(!masked.is_excluded(2), "past the mask end is not excluded");
    }
}
