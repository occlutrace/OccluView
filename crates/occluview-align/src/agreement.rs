//! SYMMETRY: the direction the deviation map does not cover, and the two-way
//! summary an operator should read instead of a one-sided one.
//!
//! A one-sided nearest-point map answers "how far is every point of the moving
//! scan from the fixed surface". It cannot answer "is any of the fixed surface
//! missing from the moving scan": a moving mesh with a hole in it reports a
//! *perfect* fit across that hole, because the vertices that would have
//! measured it do not exist. On a dome fixture with its middle third removed
//! the one-sided mean is 0.00000 mm while the reverse direction reads 0.187 mm
//! mean and 1.54 mm at the 95th percentile.
//!
//! Measuring both directions is the standard answer, and the reason Hausdorff
//! distance is defined as the larger of the two directed distances rather than
//! either one of them. This module measures the reverse direction and pools the
//! two into the symmetric statistics: mean (the average symmetric surface
//! distance), RMS, the 95th-percentile symmetric Hausdorff distance, and the
//! plain symmetric Hausdorff maximum.
//!
//! What symmetry does **not** buy is any defence against tangential blindness.
//! Slide two surfaces past each other along a direction the geometry is smooth
//! in and both directions collapse together: on a cylinder offset 0.30 mm along
//! its own axis, forward reads 0.0075 mm and reverse reads 0.0075 mm. The
//! surfaces genuinely do coincide as point sets. That is what
//! [`crate::observability`] is for, and it must be reported next to these
//! numbers, not instead of them.
//!
//! # Why these statistics and not others
//!
//! * **Symmetric, because one-sided understates.** Aspert, Santa-Cruz and
//!   Ebrahimi, "MESH: Measuring Errors between Surfaces using the Hausdorff
//!   Distance", ICME 2002: "the computation of a 'one-sided' error can lead to
//!   significantly underestimated distance values… a small one-sided distance
//!   does not imply a small distortion."
//! * **The 95th percentile and not the maximum.** Taha and Hanbury, "Metrics
//!   for evaluating 3D medical image segmentation", BMC Med Imaging 15:29
//!   (2015): "The HD is generally sensitive to outliers… it is not recommended
//!   to use the HD directly", with the quantile substitution as the fix. The
//!   maximum is still reported, as a bound rather than a summary.
//! * **A balanced mean as well as a pooled one.** Maier-Hein, Reinke et al.,
//!   "Metrics reloaded", Nature Methods 21:195 (2024), DG7.2: pooling means
//!   "if one boundary is much larger than the other, this boundary will impact
//!   the mean much more", and they "generally recommend MASD" for that reason.
//! * **Magnitudes and not signed values.** A pure rotation of an arch reads
//!   negative at one end and positive at the other and averages to nothing. The
//!   signed field is kept for the map; the summary is unsigned.
//!
//! One limitation to state rather than hide: these statistics weight each
//! *vertex* equally, not each square millimetre. A finely triangulated region
//! therefore votes more than a coarse one of the same area.

use crate::deviation::{deviation, deviation_stats, DeviationMap, DeviationSettings};
use crate::icp::Orientation;
use crate::{CancelFlag, DeviationStats, DeviationSummary, Rigid, Soup, SurfaceIndex, Validity};

/// A two-way surface comparison: both directed maps and the pooled statistics.
///
/// The pooled values weight the two directions by how many vertices each
/// actually measured, so a dense scan compared against a coarse one is not
/// summarized as though the coarse one carried half the evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceAgreement {
    /// Moving vertices measured against the fixed surface — the direction the
    /// deviation map paints.
    pub moving_to_fixed: DeviationStats,
    /// Fixed vertices measured against the moving surface — the direction that
    /// sees material the moving scan is missing.
    pub fixed_to_moving: DeviationStats,
    /// Mean absolute distance pooled over both directions, in millimetres: the
    /// average symmetric surface distance (ASSD).
    ///
    /// Pooled, so the denser of the two meshes carries proportionally more of
    /// it. Prefer [`Self::balanced_mean_abs`] when the two are not comparable
    /// in size.
    pub mean_abs: f64,
    /// Root-mean-square distance pooled over both directions, in millimetres.
    pub rms: f64,
    /// Larger of the two directed 95th percentiles, in millimetres: the robust
    /// symmetric Hausdorff distance, which one bad vertex cannot set.
    pub hausdorff_p95: f64,
    /// Larger of the two directed maxima, in millimetres: the symmetric
    /// Hausdorff distance. A single spike sets it, so it is a bound and not a
    /// summary.
    pub hausdorff: f64,
    /// Share of the pooled measurements within the tolerance band, 0 to 1.
    pub within_tolerance: f64,
    /// Measurements pooled over both directions.
    pub measured: u32,
    /// Vertices, either side, that carried no measurement.
    pub skipped: u32,
}

impl SurfaceAgreement {
    /// Mean of the two directed means, in millimetres: the mean average
    /// surface distance (MASD).
    ///
    /// Each surface contributes equally regardless of how finely it is
    /// triangulated, which is what [`Self::mean_abs`] cannot promise. A scan
    /// with three times the vertex count of the one it is compared against
    /// otherwise carries three quarters of the pooled figure on its own.
    ///
    /// Absent when either direction measured too little to characterise.
    #[must_use]
    pub fn balanced_mean_abs(&self) -> Option<f64> {
        let forward = self.moving_to_fixed.summary?;
        let backward = self.fixed_to_moving.summary?;
        Some(f64::midpoint(forward.mean_abs, backward.mean_abs))
    }

    /// How much larger one direction's mean is than the other's, in
    /// millimetres.
    ///
    /// Zero on two scans of the same surface. Large when one scan holds
    /// material the other does not — the signature a one-sided map cannot
    /// produce at all.
    ///
    /// Absent when either direction measured too little to characterise.
    #[must_use]
    pub fn asymmetry_mm(&self) -> Option<f64> {
        let forward = self.moving_to_fixed.summary?;
        let backward = self.fixed_to_moving.summary?;
        Some((backward.mean_abs - forward.mean_abs).abs())
    }
}

/// The larger of two directions' figures, ignoring a direction that measured
/// too little to have one.
fn worst(
    forward: &DeviationStats,
    backward: &DeviationStats,
    pick: impl Fn(&DeviationSummary) -> f64,
) -> f64 {
    [forward.summary.as_ref(), backward.summary.as_ref()]
        .into_iter()
        .flatten()
        .map(pick)
        .fold(0.0, f64::max)
}

/// Measure the fixed surface against the moving one: the half a
/// [`deviation`] map leaves out.
///
/// `moving_index` indexes the moving mesh in its **own local frame**, the frame
/// `pose` maps out of. The query point is carried into that frame by the
/// inverse pose rather than the mesh being re-indexed at every pose: a rigid
/// map preserves distance, so the answer is identical and an index build is
/// saved.
///
/// The sign convention matches the forward map — positive still means the
/// moving surface lies outside the fixed one — which is why the orientation is
/// flipped on the way in: the hit normal here belongs to the moving surface,
/// and it points the other way.
#[must_use]
pub fn reverse_deviation(
    fixed: Soup<'_>,
    moving_index: &SurfaceIndex,
    pose: Rigid,
    settings: &DeviationSettings,
    cancel: &CancelFlag,
) -> DeviationMap {
    let reversed = DeviationSettings {
        influence_radius_mm: settings.influence_radius_mm,
        orientation: match settings.orientation {
            Orientation::Match => Orientation::Inverted,
            Orientation::Inverted => Orientation::Match,
            Orientation::Ignored => Orientation::Ignored,
        },
    };
    deviation(fixed, moving_index, pose.inverse(), &reversed, cancel)
}

/// Pool two directed maps into the symmetric statistics.
///
/// Pass the map [`deviation`] produced and the one [`reverse_deviation`]
/// produced for the same pose and settings. Passing two maps from different
/// poses is not detectable here and yields a meaningless summary.
#[must_use]
pub fn surface_agreement(
    moving_to_fixed: &DeviationMap,
    fixed_to_moving: &DeviationMap,
    tolerance_mm: f64,
) -> SurfaceAgreement {
    let forward = deviation_stats(moving_to_fixed, tolerance_mm);
    let backward = deviation_stats(fixed_to_moving, tolerance_mm);

    let mut sum = 0.0;
    let mut squares = 0.0;
    let mut inside = 0usize;
    let mut count = 0usize;
    // Folded in map order over one direction then the other, never as a
    // parallel reduction: the same pair of maps must summarize to the same bits
    // whatever the machine ran them.
    for map in [moving_to_fixed, fixed_to_moving] {
        for (value, state) in map.signed_mm.iter().zip(&map.validity) {
            if *state != Validity::Measured {
                continue;
            }
            let magnitude = f64::from(*value).abs();
            sum += magnitude;
            squares += magnitude * magnitude;
            if magnitude <= tolerance_mm {
                inside += 1;
            }
            count += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let total = count as f64;
    let (mean_abs, rms, within_tolerance) = if count == 0 {
        (0.0, 0.0, 0.0)
    } else {
        #[allow(clippy::cast_precision_loss)]
        let share = inside as f64 / total;
        (sum / total, (squares / total).sqrt(), share)
    };

    SurfaceAgreement {
        moving_to_fixed: forward,
        fixed_to_moving: backward,
        mean_abs,
        rms,
        // A direction that measured too little contributes no distance rather
        // than a zero, which would silently pull the worst case down.
        hausdorff_p95: worst(&forward, &backward, |summary| summary.p95),
        hausdorff: worst(&forward, &backward, |summary| summary.max_abs),
        within_tolerance,
        measured: forward.measured.saturating_add(backward.measured),
        skipped: forward.skipped.saturating_add(backward.skipped),
    }
}
