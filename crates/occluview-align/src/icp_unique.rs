//! Whether a verified pose is the clear best of the basins around it.
//!
//! A small patch of a nearly symmetric region — two or three incisors, one
//! premolar — seats almost as well turned as it does straight. Measured on
//! real arches, a crop of lower incisors refined into a pose turned 60 to 180
//! degrees from the truth left a median of 0.022-0.035 mm against 0.020 mm for
//! the true pose, with the same coverage and stability: no threshold on how
//! well a pose fits can tell those apart, and the search does land in them.
//!
//! What does tell them apart is the other basin. From the found pose the patch
//! is turned about its own principal axes and refined locally from each turn;
//! every distinct pose those reach is verified the same way. A rival that fits
//! clearly better means the search stopped in the wrong basin, and the pose
//! moves there. A rival that fits about as well means the surface does not
//! decide between them, and the fit is refused as ambiguous rather than handed
//! over as a guess. This is the hypothesis-and-verify step of RANSAC-style
//! registration applied at the end of the solve instead of the start.

use glam::{DQuat, DVec3};

use crate::sample::{sample_vertices, vertex_at};
use crate::{Rigid, Soup, SurfaceIndex};

use super::icp_verify::{verify_with_budget, Verification, VerificationInput};
use super::{run_level, Level, RefineSettings, SurfaceSample, COARSE_BUDGET};
use crate::CancelFlag;

/// A rival within this factor of the found pose's verified median counts as
/// fitting about as well. In the measured crop corpus, the worst wrong basin
/// has a rival/base median ratio of 1.2617, while the closest distinct basin
/// for a correct fit is 1.5065; 1.27 separates those observed ranges.
pub(crate) const RIVAL_MARGIN: f64 = 1.27;

/// Two poses closer than this (largest displacement of the sampled patch, mm)
/// are the same basin. Wrong basins in the measured crop corpus are at least
/// 3.075 mm apart, so this excludes near-identical refinements with room below
/// the nearest observed wrong basin.
const DISTINCT_MM: f64 = 0.5;

/// A rival's local refine uses the full solver budget. A 15-iteration refine
/// leaves a lower-arch 60-degree crop at a wrong pose with 1.08 mm maximum
/// error; a 40-iteration competitor reaches a distinct basin 3.079 mm away
/// with a 1.2617 median ratio.
const RIVAL_ITERATIONS: u32 = 40;

/// A rival must cover at least this share of what the found pose covers: a
/// pose that seats a sliver tightly is not an alternative to one that seats
/// the patch. The lowest coverage share among the measured wrong rivals is
/// 0.8653; 0.8 retains those alternatives while rejecting smaller fragments.
const RIVAL_MIN_COVERAGE_SHARE: f64 = 0.8;

/// What the comparison found.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Rivalry {
    /// No distinct pose fits comparably: the found pose stands.
    Unique,
    /// A distinct pose fits clearly better.
    Better(Rigid),
    /// A distinct pose fits about as well; the surface does not decide.
    Ambiguous,
}

/// Everything a rival's local refine needs, borrowed from the main solve.
pub(crate) struct RivalContext<'a> {
    pub(crate) moving: Soup<'a>,
    pub(crate) normals: &'a [DVec3],
    pub(crate) fixed: &'a SurfaceIndex,
    pub(crate) moving_surface: Option<&'a SurfaceIndex>,
    pub(crate) fixed_samples: &'a [SurfaceSample],
    pub(crate) settings: &'a RefineSettings,
    pub(crate) cancel: &'a CancelFlag,
}

/// Compare `pose` with the poses reached from it turned about the patch's
/// principal axes.
pub(crate) fn rivalry(context: &RivalContext<'_>, pose: Rigid) -> Rivalry {
    let samples = sample_vertices(context.moving, COARSE_BUDGET);
    let posed: Vec<DVec3> = samples
        .iter()
        .filter_map(|&vertex| vertex_at(context.moving.positions, vertex as usize))
        .map(|point| pose.apply(point))
        .collect();
    let Some((centre, axes)) = principal_axes(&posed) else {
        return Rivalry::Unique;
    };
    let base = verify_with_budget(
        VerificationInput {
            moving: context.moving,
            normals: context.normals,
            fixed: context.fixed,
            pose,
            settings: context.settings,
        },
        COARSE_BUDGET,
    );
    if !base.median_mm.is_finite() {
        return Rivalry::Unique;
    }
    let local = RefineSettings {
        local_only: true,
        max_iterations: RIVAL_ITERATIONS,
        ..*context.settings
    };
    let mut best: Option<(Verification, Rigid)> = None;
    for (axis, degrees) in turns() {
        if context.cancel.is_cancelled() {
            return Rivalry::Unique;
        }
        let turn = DQuat::from_axis_angle(axes[axis], degrees.to_radians());
        let start = Rigid::new(turn, centre - turn * centre).compose(&pose);
        let Ok(outcome) = run_level(&Level {
            moving: context.moving,
            normals: context.normals,
            fixed: context.fixed,
            moving_surface: context.moving_surface,
            fixed_samples: context.fixed_samples,
            samples: &samples,
            settings: &local,
            cancel: context.cancel,
            start,
        }) else {
            continue;
        };
        let rival = outcome.pose;
        if separation(&samples, context.moving, pose, rival) < DISTINCT_MM {
            continue;
        }
        let checked = verify_with_budget(
            VerificationInput {
                moving: context.moving,
                normals: context.normals,
                fixed: context.fixed,
                pose: rival,
                settings: context.settings,
            },
            COARSE_BUDGET,
        );
        if !checked.median_mm.is_finite()
            || checked.coverage < base.coverage * RIVAL_MIN_COVERAGE_SHARE
        {
            continue;
        }
        if best.is_none_or(|(held, _)| checked.median_mm < held.median_mm) {
            best = Some((checked, rival));
        }
    }
    match best {
        Some((rival, pose)) if rival.median_mm * RIVAL_MARGIN < base.median_mm => {
            Rivalry::Better(pose)
        }
        Some((rival, _)) if rival.median_mm <= base.median_mm * RIVAL_MARGIN => Rivalry::Ambiguous,
        _ => Rivalry::Unique,
    }
}

/// The turns tried: a quarter, half and three-quarter turn about each
/// principal axis, and sixths about the axis of least spread (the occlusal
/// direction of a patch lying on an arch).
fn turns() -> impl Iterator<Item = (usize, f64)> {
    (0..3)
        .flat_map(|axis| [90.0, 180.0, 270.0].map(move |degrees| (axis, degrees)))
        .chain([60.0, 120.0, 240.0, 300.0].map(|degrees| (2, degrees)))
}

/// Largest displacement between two poses over the sampled patch.
fn separation(samples: &[u32], moving: Soup<'_>, pose: Rigid, rival: Rigid) -> f64 {
    samples
        .iter()
        .filter_map(|&vertex| vertex_at(moving.positions, vertex as usize))
        .map(|point| pose.apply(point).distance(rival.apply(point)))
        .fold(0.0, f64::max)
}

/// Centroid and unit principal axes (descending spread) of `points`.
fn principal_axes(points: &[DVec3]) -> Option<(DVec3, [DVec3; 3])> {
    if points.len() < 3 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let weight = 1.0 / points.len() as f64;
    let centre = points.iter().fold(DVec3::ZERO, |sum, point| sum + *point) * weight;
    let mut spread = [[0.0_f64; 3]; 3];
    for point in points {
        let offset = (*point - centre).to_array();
        for (row, line) in spread.iter_mut().enumerate() {
            for (column, cell) in line.iter_mut().enumerate() {
                *cell += offset[row] * offset[column] * weight;
            }
        }
    }
    let apply = |vector: DVec3| {
        DVec3::new(
            spread[0][0] * vector.x + spread[0][1] * vector.y + spread[0][2] * vector.z,
            spread[1][0] * vector.x + spread[1][1] * vector.y + spread[1][2] * vector.z,
            spread[2][0] * vector.x + spread[2][1] * vector.y + spread[2][2] * vector.z,
        )
    };
    // Fixed-count power iteration with deflation: deterministic, and the axes
    // only steer where the turns are tried, so their precision is not critical.
    let mut first = DVec3::new(1.0, 0.31, 0.17).normalize();
    for _ in 0..64 {
        first = apply(first).normalize_or_zero();
    }
    let mut second = DVec3::new(0.21, 1.0, 0.37).normalize();
    for _ in 0..64 {
        second = apply(second);
        second = (second - first * second.dot(first)).normalize_or_zero();
    }
    let third = first.cross(second).normalize_or_zero();
    (first.is_finite() && second.is_finite() && third.length_squared() > 0.5)
        .then_some((centre, [first, second, third]))
}
