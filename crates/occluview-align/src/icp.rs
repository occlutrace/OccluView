//! Trimmed point-to-plane ICP against the fixed surface.
//!
//! Refinement runs at coarse and dense sample resolutions.
//!
//! Correspondence search is parallel, while normal equations are accumulated
//! serially in sample order to keep floating-point results deterministic.

use glam::DVec3;

use crate::pairs::FitRejection;
use crate::sample::{bounds_of, sample_vertices, vertex_at, vertex_normals};
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};
use occluview_surface_query::SurfaceSample;

#[path = "feature_seed.rs"]
mod feature_seed;

#[path = "icp_overlap.rs"]
mod icp_overlap;
#[path = "icp_step.rs"]
mod icp_step;
#[path = "icp_verify.rs"]
mod icp_verify;
use icp_verify::{verification_holds, verify, Verification};
#[path = "icp_unique.rs"]
mod icp_unique;
use icp_unique::{rivalry, RivalContext, Rivalry};

#[path = "icp_search.rs"]
mod icp_search;
use icp_search::{choose_start_pose, StartPose};
#[cfg(test)]
use icp_search::{coarse_candidate_is_better, coarse_candidates_are_ambiguous, CoarseCandidate};
#[path = "icp_solve.rs"]
mod icp_solve;
use icp_solve::run_level;
#[cfg(test)]
use icp_solve::weak_axes_from_normal_matrix;
#[cfg(test)]
use icp_step::correspondences_at_radius;

#[cfg(test)]
#[path = "icp_internal_tests.rs"]
mod icp_internal_tests;

/// Samples used by the coarse level.
const COARSE_BUDGET: usize = 8_000;
/// Samples used by the dense level.
const DENSE_BUDGET: usize = 40_000;

/// Bounded representatives used for the fixed-to-moving half of the overlap
/// check. This is much smaller than the dense ICP level: it is a
/// guard against a wrong patch, not a second dense registration pass.
const RECIPROCAL_BUDGET: usize = 2_048;

/// Correspondences below this leave the fit undetermined.
const MIN_CORRESPONDENCES: usize = 6;

/// A large scan must not be declared registered because six vertices happened
/// to land on a neighbouring patch. Partial scans remain allowed; this is a
/// small one-percent floor on the moving surface.
const MIN_FORWARD_COVERAGE_FRACTION: f64 = 0.01;

/// Seated-fraction difference that counts as a real advantage rather than
/// noise. Two poses a few micrometres apart seat the same surface.
const COARSE_TIE_SEATED: f64 = 0.01;

/// A committed refinement must explain a meaningful portion of the moving
/// surface. The looser one-percent floor above is still useful while searching
/// for a local correspondence set, but it is not enough to authorize a pose.
const MIN_REFINEMENT_COVERAGE_FRACTION: f64 = 0.05;

/// A stationary local patch must still sit close to the fixed surface before a
/// pose can authorize a heatmap. The limit is derived from the operator's
/// correspondence radius in [`IcpReport::is_trustworthy_refinement_for`], so
/// it follows the physical search setting rather than a mesh-size guess.
const MAX_REFINEMENT_GEOMETRIC_RMS_FRACTION: f64 = 0.5;

/// How far the typical matched vertex may sit from the surface it matched.
///
/// A fraction of the operator's correspondence radius, like the ceiling above,
/// but this one is what tells an alignment from an accident. Two different
/// jaws can be brought close enough that a third of one surface finds points on
/// the other within the search radius; measured, that seating reports a
/// geometric RMS of 0.58 mm and a coverage of 35 %, so it satisfies every
/// threshold above while being a confidently wrong pose that this gate
/// refuses.
///
/// The median separates them cleanly. On the same pair: the two-jaw seating has
/// a median of 0.42 mm, while an arch seated against a displaced copy of itself
/// has a median at the surface's own discretisation error. The threshold sits
/// between those, scaled by the radius so it follows the operator's setting
/// rather than a mesh-size guess.
const MAX_REFINEMENT_MEDIAN_FRACTION: f64 = 0.10;

/// The floor under that limit, in millimetres.
///
/// The search radius is adjustable down to 0.2 mm, where a tenth of it is
/// 0.02 mm — below the noise of the scanners this tool reads. A limit that
/// tight would refuse a correct seating. The floor sits above scanner noise
/// and well below the separation that matters.
const MIN_REFINEMENT_MEDIAN_MM: f64 = 0.08;

/// The ceiling over that limit, in millimetres.
///
/// The radius is also adjustable up to 10 mm, where a tenth of it is a
/// millimetre — wide enough to authorize two different jaws, which is the pose
/// this gate refuses. Measured on the real fixture pair that seating has a
/// median of 0.42 mm, so the ceiling is set under it and above any seating
/// that is actually an alignment. Without this end the gate weakens when the
/// operator widens the search.
const MAX_REFINEMENT_MEDIAN_MM: f64 = 0.30;

/// The proximity band that decides how much of the surface counts as seated.
///
/// It decides two things the operator cannot see: whether the global feature
/// seed runs at all (at 0.9), and how much of the trim ratio is actually used
/// (the fraction is multiplied by 0.8 and clamped to 0.1..0.8, replacing the
/// ratio slider). The band is derived from the operator's correspondence
/// radius and clamped, so it tracks a setting rather than a mesh-size guess.
fn seated_band_mm(influence_radius_mm: f64) -> f64 {
    (influence_radius_mm.abs() * SEATED_BAND_RADIUS_FRACTION)
        .clamp(MIN_SEATED_BAND_MM, MAX_SEATED_BAND_MM)
}

/// Fraction of the correspondence radius the seated band spans.
const SEATED_BAND_RADIUS_FRACTION: f64 = 0.1;

/// Floor and ceiling for that band, in millimetres.
///
/// The floor keeps the band above a scanner's own discretisation error at the
/// tightest slider setting; the ceiling keeps it from growing into "anything on
/// the other arch counts as seated" at the widest.
const MIN_SEATED_BAND_MM: f64 = 0.2;
const MAX_SEATED_BAND_MM: f64 = 1.0;

/// Maximum point-to-plane p95 residual, in millimetres. Correct cases in the
/// selected real and crop corpus reach 0.03571 mm; the lowest wrong accepted
/// case reaches 0.04463 mm. This scan-agreement limit is independent of search
/// reach, which describes where to look rather than how closely surfaces fit.
const MAX_REFINEMENT_P95_MM: f64 = 0.04;

/// Huber cut as a multiple of the median absolute residual — the usual 95%
/// efficiency constant for a normal error model.
const HUBER_FACTOR: f64 = 1.345;

/// How close a sample must sit to count as *seated*, in millimetres.
///
/// This is the one number in the refinement that does not come from the
/// trimmed residual, and it exists because the trimmed residual cannot tell a
/// seating from a slide. On a prepared model only a part of the surface is
/// truly rigid; the operated region sits within a couple of millimetres of the
/// original but is not congruent to it. A trimmed least-squares objective is
/// minimised by spreading that deformation over everything — the reported
/// residual goes down while the scan goes sideways — so that objective alone
/// selects the pose that best hides the deformation, not the one that seats the
/// surface that did not change.
///
/// The band is the distance within which two acquisitions of the same surface
/// agree, and it is much smaller than any correspondence radius. Measured on a
/// real prepared arch pair, the true seating puts 0.203 of the sampled surface
/// inside 0.05 mm while a trimmed-residual wrong pose manages 0.072, and
/// trimmed-residual refinement from the true pose moves 1.98 mm away from it:
/// the objective, not the search, picks the wrong basin.
const SEATED_BAND_MM: f64 = 0.05;

/// Rotation step below this (radians) counts as converged.
const CONVERGED_ROTATION: f64 = 1e-7;
/// Translation step below this (millimetres) counts as converged.
const CONVERGED_TRANSLATION: f64 = 1e-7;

/// Starting Levenberg damping, as a fraction of each diagonal entry.
const INITIAL_DAMPING: f64 = 1e-6;
/// Damping growth per rejected step.
const DAMPING_GROWTH: f64 = 10.0;
/// Damping retries before a level gives up on the current iteration.
const MAX_DAMPING_RETRIES: usize = 3;

/// Trial sizes for a solved step. The normal equations are a local
/// linearisation; a full step can cross a nearest-surface boundary and turn a
/// good registration sideways. A bounded line search keeps the rigid solver
/// local while refusing that untested jump.
const BACKTRACK_SCALES: [f64; 4] = [1.0, 0.5, 0.25, 0.125];

/// Do not accept a trial that loses most of the surface supporting its fit.
const MIN_TRIAL_COVERAGE_FRACTION: f64 = 0.85;

/// An iteration counts as an improvement only if it cuts the residual by more
/// than this factor. A tenth of a percent is far below anything a scan can
/// resolve, so anything slower than that is wandering, not converging.
const STALL_IMPROVEMENT: f64 = 0.999;

/// A normal-equation diagonal below this fraction of the largest means that
/// degree of freedom is not determined by the geometry.
const WEAK_AXIS_FRACTION: f64 = 1e-6;

/// Which way the two surfaces face each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Accept a correspondence only where the surfaces face the same way.
    #[default]
    Match,
    /// Accept only where they face opposite ways — the escape hatch for a
    /// fixed mesh whose winding is inverted.
    Inverted,
    /// Accept either.
    Ignored,
}

/// Knobs the operator can see, in the operator's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefineSettings {
    /// Farthest a moving vertex may look for fixed surface, in millimetres.
    /// Maximum correspondence distance, in millimetres.
    pub influence_radius_mm: f64,
    /// Fraction of correspondences kept after trimming, 0 to 1.
    pub matching_ratio: f64,
    /// Surface orientation rule.
    pub orientation: Orientation,
    /// Iteration ceiling per level.
    pub max_iterations: u32,
    /// Keep refinement around the pose supplied by the operator's coarse fit.
    /// Global feature recovery is only for callers explicitly seeking it.
    pub local_only: bool,
}

impl Default for RefineSettings {
    fn default() -> Self {
        Self {
            influence_radius_mm: 2.0,
            matching_ratio: 0.8,
            orientation: Orientation::Match,
            max_iterations: 40,
            local_only: false,
        }
    }
}

/// What the refine actually did, in the terms the panel reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IcpReport {
    /// The refined pose.
    pub rigid: Rigid,
    /// Iterations run across both levels.
    pub iterations: u32,
    /// Whether the last level stopped because the step went to nothing.
    pub converged: bool,
    /// Correspondences kept by the final iteration.
    pub inliers: u32,
    /// Kept correspondences over sampled vertices.
    pub inlier_ratio: f64,
    /// Sampled vertices that found any fixed surface at all.
    pub coverage: f64,
    /// Root-mean-square point-to-plane residual, in millimetres.
    pub rms: f64,
    /// Root-mean-square Euclidean distance to the matched fixed surface, in
    /// millimetres. Unlike point-to-plane RMS this catches a tangent-slide
    /// local match that has a tiny normal error but is geometrically apart.
    pub geometric_rms: f64,
    /// Median absolute residual, in millimetres.
    pub median_abs: f64,
    /// 95th-percentile absolute residual, in millimetres.
    pub p95_abs: f64,
    /// Per world axis, whether rotation about it is undetermined.
    pub weak_rot_axes: [bool; 3],
    /// Per world axis, whether translation along it is undetermined.
    pub weak_trans_axes: [bool; 3],
    /// The trim ratio the fit actually ran at.
    ///
    /// Not the operator's slider value: the global-seed branch replaces it with
    /// `near_surface_fraction(seed) * 0.8` clamped to 0.1..0.8. Carried here so
    /// the panel can state which algorithm ran.
    pub effective_matching_ratio: f64,
    /// Fraction of the level's samples inside the seated band of the surface
    /// (see `SEATED_BAND_MM`).
    ///
    /// The statistic the solver ranks candidates by and keeps monotonic in
    /// `icp_step`. It is a diagnostic; the acceptance gate decides on the median
    /// residual.
    pub seated_fraction: f64,
    /// Share of the sampled moving surface with a counterpart at the final
    /// pose: nothing trimmed, the operator's reach, and vertices whose nearest
    /// fixed point is the fixed scan's open border left out.
    pub verified_coverage: f64,
    /// Median distance from those vertices to their counterpart, in
    /// millimetres. Independent of the solve's matching ratio; comparison of
    /// rival poses reads it alongside the solve median for verification because
    /// intentional edits can raise a correct fit's whole-surface median above
    /// scanner noise.
    pub verified_median_mm: f64,
    /// Smallest eigenvalue of the tightly seated part's normalized
    /// point-to-plane information matrix: near zero when that part alone
    /// could still slide or turn.
    pub verified_stability: f64,
}

impl IcpReport {
    /// Whether this report is strong enough to authorize a pose commit and the
    /// deviation map that follows it.
    ///
    /// `refine` intentionally returns its best report when an iteration budget
    /// stalls so callers can inspect diagnostics. That report is not, by
    /// itself, a successful registration: a stalled local patch, a
    /// rank-deficient plane, or a tiny overlap must not become a heatmap. The
    /// application uses this explicit gate instead of treating every
    /// `Ok(report)` as refined.
    #[must_use]
    pub fn is_trustworthy_refinement(&self) -> bool {
        self.is_trustworthy_refinement_with_limit(0.5)
    }

    /// Apply the same trust gate with the operator's correspondence radius.
    ///
    /// A small accepted step is only convergence of the optimizer, not proof
    /// that it found the intended surface. The geometric RMS floor rejects a
    /// stationary but still-apart local patch before the application can mark
    /// it as refined and paint a misleading map.
    #[must_use]
    pub fn is_trustworthy_refinement_for(&self, settings: &RefineSettings) -> bool {
        let radius = settings.influence_radius_mm.abs();
        let limit = radius * MAX_REFINEMENT_GEOMETRIC_RMS_FRACTION;
        let median_limit = (radius * MAX_REFINEMENT_MEDIAN_FRACTION)
            .clamp(MIN_REFINEMENT_MEDIAN_MM, MAX_REFINEMENT_MEDIAN_MM);
        // No seated-fraction floor distinguishes the measured partial fit
        // (0.05 seats over 0.08 coverage, with a 0.000 mm median) from the false
        // partial fit (0.072 seated): a minimum high enough to reject the false
        // pose also rejects the correct one. The two-jaw accident remains
        // refused by the median (0.42 mm against the 0.30 mm ceiling), so
        // `seated_fraction` ranks candidates without authorizing them.
        self.is_trustworthy_refinement_with_limit(limit)
            && self.median_abs.is_finite()
            && self.median_abs <= median_limit
    }

    fn is_trustworthy_refinement_with_limit(&self, geometric_rms_limit: f64) -> bool {
        self.converged
            && self.inliers >= u32::try_from(MIN_CORRESPONDENCES).unwrap_or(u32::MAX)
            && self.coverage.is_finite()
            && self.coverage >= MIN_REFINEMENT_COVERAGE_FRACTION
            && self.inlier_ratio.is_finite()
            && self.inlier_ratio > 0.0
            && self.rms.is_finite()
            && self.rms >= 0.0
            && self.geometric_rms.is_finite()
            && self.geometric_rms >= 0.0
            && geometric_rms_limit.is_finite()
            && geometric_rms_limit >= 0.0
            && self.geometric_rms <= geometric_rms_limit
            && self.median_abs.is_finite()
            && self.p95_abs.is_finite()
            && self.p95_abs >= 0.0
            && self.p95_abs <= MAX_REFINEMENT_P95_MM
            && !self.weak_rot_axes.into_iter().any(|weak| weak)
            && !self.weak_trans_axes.into_iter().any(|weak| weak)
            && verification_holds(
                &Verification {
                    coverage: self.verified_coverage,
                    median_mm: self.verified_median_mm,
                    stability: self.verified_stability,
                },
                self.median_abs,
            )
    }
}

/// Refine `start` so `moving` seats onto the surface behind `fixed`.
///
/// # Errors
///
/// Returns [`FitRejection::TooFewPairs`] when the moving mesh is empty or too
/// little of it reaches the fixed surface to determine a pose, and
/// [`FitRejection::Runaway`] when the result would move the mesh farther than
/// its own size plus the influence radius plus the coarse hypothesis this start
/// proved. Those are the two bounded stages the total move is made of; the
/// refinement's own travel is not allowed to grow beyond the first two terms.
/// A surface with enough pairs but no accepted improvement returns
/// [`FitRejection::NoImprovement`] instead of claiming a refined pose.
/// A solve that stops on an iteration budget returns its best report with
/// `converged == false`, which is not by itself a success: callers must consult
/// [`IcpReport::is_trustworthy_refinement`].
pub fn refine(
    moving: Soup<'_>,
    fixed: &SurfaceIndex,
    start: Rigid,
    settings: &RefineSettings,
    cancel: &CancelFlag,
) -> Result<IcpReport, FitRejection> {
    if moving.vertex_count() == 0 || moving.triangle_count() == 0 {
        return Err(FitRejection::TooFewPairs {
            have: 0,
            need: MIN_CORRESPONDENCES,
        });
    }
    if !start.is_finite() {
        return Err(FitRejection::NonFinite);
    }
    if cancel.is_cancelled() {
        return Ok(idle_report(start));
    }

    let normals = vertex_normals(moving);
    // The moving index is built from the same masked soup as the forward
    // correspondence path. It is optional because a soup can have vertices
    // and triangles but no usable non-degenerate triangle after masking.
    let moving_surface = SurfaceIndex::build(moving);
    let fixed_samples = fixed.representative_samples(RECIPROCAL_BUDGET);
    let (center, extent) = bounds_of(moving).unwrap_or((DVec3::ZERO, 0.0));
    let initial_samples = sample_vertices(moving, COARSE_BUDGET);
    let initial_level = Level {
        moving,
        normals: &normals,
        fixed,
        moving_surface: moving_surface.as_ref(),
        fixed_samples: &fixed_samples,
        samples: &initial_samples,
        settings,
        cancel,
        start,
    };
    let (initial_pose, adaptive_settings, feature_seed) =
        select_initial_pose(&initial_level, center)?;
    let state = refine_levels(&initial_level, &adaptive_settings, initial_pose.rigid)?;
    let allowed = extent.max(1.0) + initial_pose.coarse_shift + settings.influence_radius_mm.abs();
    let rivalry = RivalContext {
        moving,
        normals: &normals,
        fixed,
        moving_surface: moving_surface.as_ref(),
        fixed_samples: &fixed_samples,
        settings: &adaptive_settings,
        cancel,
    };
    finalize_refinement(
        FinalizeContext {
            rivalry,
            settings,
            start,
            center,
            allowed,
            feature_seed,
            matching_ratio: adaptive_settings.matching_ratio,
        },
        state,
    )
}

/// Run the coarse and dense levels with one selected starting pose.
fn refine_levels(
    base: &Level<'_>,
    settings: &RefineSettings,
    mut pose: Rigid,
) -> Result<LevelOutcome, FitRejection> {
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary = None;
    for budget in [COARSE_BUDGET, DENSE_BUDGET] {
        let samples = sample_vertices(base.moving, budget);
        if !level_samples_are_usable(summary, &samples)? {
            continue;
        }
        // A dense level is not optional evidence. Keeping the coarse summary
        // after a dense refusal would let a sparse/accidental coarse sample
        // authorize a refined pose and the heatmap that follows it.
        // Cancellation is returned as an untrusted report when it has evidence;
        // structural and refinement refusals remain refusals to the worker.
        let level = run_level(&Level {
            moving: base.moving,
            normals: base.normals,
            fixed: base.fixed,
            moving_surface: base.moving_surface,
            fixed_samples: base.fixed_samples,
            samples: &samples,
            settings,
            cancel: base.cancel,
            start: pose,
        })?;
        iterations += level.iterations;
        converged = level.converged;
        pose = level.pose;
        summary = Some(level.summary);
    }
    let Some(summary) = summary else {
        return Err(FitRejection::TooFewPairs {
            have: 0,
            need: MIN_CORRESPONDENCES,
        });
    };
    Ok(LevelOutcome {
        pose,
        iterations,
        converged,
        summary,
    })
}

/// Bounds and evidence used after the dense solve.
struct FinalizeContext<'a> {
    rivalry: RivalContext<'a>,
    settings: &'a RefineSettings,
    start: Rigid,
    center: DVec3,
    allowed: f64,
    feature_seed: Option<feature_seed::FeatureSeed>,
    matching_ratio: f64,
}

/// Verify the final pose, follow one better rival once, and assemble its report.
fn finalize_refinement(
    context: FinalizeContext<'_>,
    mut state: LevelOutcome,
) -> Result<IcpReport, FitRejection> {
    if let Some(seed) = context.feature_seed {
        if state
            .pose
            .apply(context.center)
            .distance(seed.rigid.apply(context.center))
            > 1.0
            || turn_between(state.pose, seed.rigid) > 0.1
        {
            return Err(FitRejection::NoImprovement);
        }
    }
    ensure_movement_bound(state.pose, context.center, context.start, context.allowed)?;
    let mut verification = verify(
        context.rivalry.moving,
        context.rivalry.normals,
        context.rivalry.fixed,
        state.pose,
        context.settings,
    );
    if verification_holds(&verification, state.summary.median_abs) {
        match rivalry(&context.rivalry, state.pose) {
            Rivalry::Unique => {}
            Rivalry::Ambiguous => return Err(FitRejection::Ambiguous),
            Rivalry::Better(better) => {
                state = refine_better_rival(&context.rivalry, better, state.iterations)?;
                ensure_movement_bound(state.pose, context.center, context.start, context.allowed)?;
                verification = verify(
                    context.rivalry.moving,
                    context.rivalry.normals,
                    context.rivalry.fixed,
                    state.pose,
                    context.settings,
                );
                if verification_holds(&verification, state.summary.median_abs)
                    && !matches!(rivalry(&context.rivalry, state.pose), Rivalry::Unique)
                {
                    return Err(FitRejection::Ambiguous);
                }
            }
        }
    }
    Ok(report_from_state(
        state,
        verification,
        context.matching_ratio,
    ))
}

/// Refine the one better basin discovered by the final competition pass.
fn refine_better_rival(
    context: &RivalContext<'_>,
    better: Rigid,
    previous_iterations: u32,
) -> Result<LevelOutcome, FitRejection> {
    let samples = sample_vertices(context.moving, DENSE_BUDGET);
    let level = run_level(&Level {
        moving: context.moving,
        normals: context.normals,
        fixed: context.fixed,
        moving_surface: context.moving_surface,
        fixed_samples: context.fixed_samples,
        samples: &samples,
        settings: context.settings,
        cancel: context.cancel,
        start: better,
    })?;
    Ok(LevelOutcome {
        pose: level.pose,
        iterations: previous_iterations + level.iterations,
        converged: level.converged,
        summary: level.summary,
    })
}

fn ensure_movement_bound(
    pose: Rigid,
    center: DVec3,
    start: Rigid,
    allowed: f64,
) -> Result<(), FitRejection> {
    let moved_by = (pose.apply(center) - start.apply(center)).length();
    if moved_by > allowed {
        return Err(FitRejection::Runaway { moved_by, allowed });
    }
    Ok(())
}

fn report_from_state(
    state: LevelOutcome,
    verification: Verification,
    matching_ratio: f64,
) -> IcpReport {
    IcpReport {
        rigid: state.pose,
        iterations: state.iterations,
        converged: state.converged,
        inliers: state.summary.inliers,
        inlier_ratio: state.summary.inlier_ratio,
        coverage: state.summary.coverage,
        rms: state.summary.rms,
        geometric_rms: state.summary.geometric_rms,
        median_abs: state.summary.median_abs,
        p95_abs: state.summary.p95_abs,
        weak_rot_axes: state.summary.weak_rot_axes,
        weak_trans_axes: state.summary.weak_trans_axes,
        effective_matching_ratio: matching_ratio,
        seated_fraction: state.summary.seated_fraction,
        verified_coverage: verification.coverage,
        verified_median_mm: verification.median_mm,
        verified_stability: verification.stability,
    }
}

/// Turn between two poses, in radians.
fn turn_between(left: Rigid, right: Rigid) -> f64 {
    (left.rotation * right.rotation.inverse())
        .to_scaled_axis()
        .length()
}

fn select_initial_pose(
    level: &Level<'_>,
    center: DVec3,
) -> Result<(StartPose, RefineSettings, Option<feature_seed::FeatureSeed>), FitRejection> {
    if level.settings.local_only {
        return Ok((
            StartPose {
                rigid: level.start,
                coarse_shift: 0.0,
            },
            *level.settings,
            None,
        ));
    }
    // An already seated scan needs no global search. Keep the cheap local
    // path when almost the entire surface is within scanner tolerance.
    let feature_seed = if near_surface_fraction(
        level.moving,
        level.samples,
        level.fixed,
        level.start,
        level.cancel,
        seated_band_mm(level.settings.influence_radius_mm),
    ) >= 0.9
    {
        None
    } else {
        feature_seed::find_feature_seed(level.moving_surface, level.fixed, level.cancel)
    };
    let initial_pose = if let Some(seed) = feature_seed {
        let coarse_shift = seed.rigid.apply(center).distance(level.start.apply(center));
        if !coarse_shift.is_finite() {
            return Err(FitRejection::NonFinite);
        }
        StartPose {
            rigid: seed.rigid,
            coarse_shift,
        }
    } else {
        choose_start_pose(level)?
    };
    let settings = RefineSettings {
        matching_ratio: feature_seed.map_or(level.settings.matching_ratio, |_| {
            level.settings.matching_ratio.min(
                (near_surface_fraction(
                    level.moving,
                    level.samples,
                    level.fixed,
                    initial_pose.rigid,
                    level.cancel,
                    seated_band_mm(level.settings.influence_radius_mm),
                ) * 0.8)
                    .clamp(0.1, 0.8),
            )
        }),
        ..*level.settings
    };
    Ok((initial_pose, settings, feature_seed))
}

#[allow(clippy::cast_precision_loss, clippy::too_many_arguments)]
fn near_surface_fraction(
    moving: Soup<'_>,
    samples: &[u32],
    fixed: &SurfaceIndex,
    pose: Rigid,
    cancel: &CancelFlag,
    band_mm: f64,
) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut near = 0usize;
    for (slot, &vertex) in samples.iter().enumerate() {
        if slot % 256 == 0 && cancel.is_cancelled() {
            return 0.0;
        }
        let Some(point) = vertex_at(moving.positions, vertex as usize) else {
            continue;
        };
        if fixed.nearest(pose.apply(point), band_mm).is_some() {
            near += 1;
        }
    }
    near as f64 / samples.len() as f64
}

/// Whether a sampling level produced usable evidence.
///
/// A missing coarse sample set is harmless — the dense pass can still be the
/// first usable level. Once a coarse report exists, an empty dense set is a
/// missing required evidence stage, not permission to keep the coarse report
/// and call the result refined.
///
/// The current sampler does not reach this refusal: `sample_vertices` returns
/// nothing only for a soup with no usable vertex, so emptiness does not depend
/// on the budget and no level can be empty while another has evidence. The
/// refine contract does not rely on that sampler property: a sparse coarse
/// sample never authorizes a refined pose on its own.
/// `an_empty_dense_level_cannot_reuse_coarse_evidence` covers the rule.
fn level_samples_are_usable(
    previous_summary: Option<Summary>,
    samples: &[u32],
) -> Result<bool, FitRejection> {
    if !samples.is_empty() {
        return Ok(true);
    }
    if previous_summary.is_some() {
        return Err(FitRejection::TooFewPairs {
            have: 0,
            need: MIN_CORRESPONDENCES,
        });
    }
    Ok(false)
}

/// The report for a run cancelled before its first measurement.
fn idle_report(start: Rigid) -> IcpReport {
    IcpReport {
        rigid: start,
        iterations: 0,
        converged: false,
        inliers: 0,
        inlier_ratio: 0.0,
        coverage: 0.0,
        rms: 0.0,
        geometric_rms: 0.0,
        median_abs: 0.0,
        p95_abs: 0.0,
        weak_rot_axes: [true; 3],
        weak_trans_axes: [true; 3],
        effective_matching_ratio: 0.0,
        seated_fraction: 0.0,
        verified_coverage: Verification::NONE.coverage,
        verified_median_mm: Verification::NONE.median_mm,
        verified_stability: Verification::NONE.stability,
    }
}

/// Everything one resolution level needs, bundled so the level function keeps
/// a readable signature.
struct Level<'a> {
    moving: Soup<'a>,
    normals: &'a [DVec3],
    fixed: &'a SurfaceIndex,
    moving_surface: Option<&'a SurfaceIndex>,
    fixed_samples: &'a [SurfaceSample],
    samples: &'a [u32],
    settings: &'a RefineSettings,
    cancel: &'a CancelFlag,
    start: Rigid,
}

/// Search radii from the operator's own setting outwards.
///
/// The operator's number is where the search starts, not the top of a ladder:
/// a fit configured at 2.0 mm searches 2.0 mm first, so a pair a few
/// millimetres apart is found at the reach the operator set.
///
/// Widening past it is bounded and only happens when the coverage floor is
/// still unmet at that radius, which is the case the operator cannot fix by
/// moving the scans. A broad influence distance can connect two neighbouring
/// teeth, so the wider rungs are a fallback reach rather than the first answer.
fn influence_radius_ladder(maximum: f64) -> Vec<f64> {
    if !maximum.is_finite() || maximum <= 0.0 {
        return Vec::new();
    }
    [1.0, 2.0, 4.0]
        .into_iter()
        .map(|fraction| (maximum * fraction).max(f64::EPSILON))
        .collect()
}

/// Whether the forward correspondence set covers enough of the moving sample
/// population to say anything about the whole registration.
#[allow(clippy::cast_precision_loss)]
fn forward_coverage_is_sufficient(matched: usize, sampled: usize) -> bool {
    matched >= MIN_CORRESPONDENCES
        && sampled > 0
        && matched as f64 / sampled as f64 >= MIN_FORWARD_COVERAGE_FRACTION
}

/// The count shown in a `TooFewPairs` rejection when coverage, rather than the
/// mathematical minimum, is what made the fit untrustworthy.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn minimum_forward_matches(sampled: usize) -> usize {
    MIN_CORRESPONDENCES.max((sampled as f64 * MIN_FORWARD_COVERAGE_FRACTION).ceil() as usize)
}

/// Statistics carried out of a level.
#[derive(Clone, Copy)]
struct Summary {
    inliers: u32,
    inlier_ratio: f64,
    coverage: f64,
    rms: f64,
    /// RMS Euclidean point-to-surface distance that gates a trial pose.
    geometric_rms: f64,
    /// Fraction of the level's samples sitting inside `SEATED_BAND_MM` of the
    /// fixed surface. This is the term that distinguishes a seating from a
    /// slide; every other statistic here is a trimmed residual and improves
    /// when a deformation is spread out.
    seated_fraction: f64,
    median_abs: f64,
    p95_abs: f64,
    weak_rot_axes: [bool; 3],
    weak_trans_axes: [bool; 3],
}

/// A level's outcome.
struct LevelOutcome {
    pose: Rigid,
    iterations: u32,
    converged: bool,
    summary: Summary,
}
