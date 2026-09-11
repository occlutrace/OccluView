//! Trimmed point-to-plane ICP against the fixed surface.
//!
//! Refinement runs at coarse and dense sample resolutions.
//!
//! Correspondence search is parallel, while normal equations are accumulated
//! serially in sample order to keep floating-point results deterministic.

use glam::{DMat3, DQuat, DVec3, EulerRot};
use rayon::prelude::*;

use crate::pairs::FitRejection;
use crate::sample::{bounds_of, sample_vertices, vertex_at, vertex_normals};
use crate::surface::SurfaceSample;
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

#[path = "icp_overlap.rs"]
mod icp_overlap;
use icp_overlap::{reciprocal_evidence, reciprocal_evidence_is_usable, ReciprocalSummary};
#[path = "icp_step.rs"]
mod icp_step;
use icp_step::{correspondences_at_radius, try_backtracked_step, TrialState};

#[cfg(test)]
#[path = "icp_internal_tests.rs"]
mod icp_internal_tests;

/// Samples used by the coarse level.
const COARSE_BUDGET: usize = 8_000;
/// Samples used by the dense level.
const DENSE_BUDGET: usize = 40_000;

/// Bounded representatives used for the fixed-to-moving half of the overlap
/// check. This is deliberately much smaller than the dense ICP level: it is a
/// guard against a wrong patch, not a second dense registration pass.
const RECIPROCAL_BUDGET: usize = 2_048;

/// Correspondences below this leave the fit undetermined.
const MIN_CORRESPONDENCES: usize = 6;

/// A large scan must not be declared registered because six vertices happened
/// to land on a neighbouring patch. Partial scans remain allowed; this is a
/// deliberately small one-percent floor on the moving surface.
const MIN_FORWARD_COVERAGE_FRACTION: f64 = 0.01;

/// A committed refinement must explain a meaningful portion of the moving
/// surface. The looser one-percent floor above is still useful while searching
/// for a local correspondence set, but it is not enough to authorize a pose.
const MIN_REFINEMENT_COVERAGE_FRACTION: f64 = 0.05;

/// Huber cut as a multiple of the median absolute residual — the usual 95%
/// efficiency constant for a normal error model.
const HUBER_FACTOR: f64 = 1.345;

/// Rotation step below this (radians) counts as converged.
const CONVERGED_ROTATION: f64 = 1e-7;
/// Translation step below this (millimetres) counts as converged.
const CONVERGED_TRANSLATION: f64 = 1e-7;

/// A rank-deficient zero-step fit is acceptable only when its measured surface
/// residual is already at numerical zero. A non-zero residual needs an
/// accepted rigid step before the UI may authorize a deviation map.
const CONVERGED_RESIDUAL_MM: f64 = 1e-6;

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

/// Do not accept a trial that loses most of the surface it was fitted on.
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
}

impl Default for RefineSettings {
    fn default() -> Self {
        Self {
            influence_radius_mm: 2.0,
            matching_ratio: 0.8,
            orientation: Orientation::Match,
            max_iterations: 40,
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
    /// Median absolute residual, in millimetres.
    pub median_abs: f64,
    /// 95th-percentile absolute residual, in millimetres.
    pub p95_abs: f64,
    /// Per world axis, whether rotation about it is undetermined.
    pub weak_rot_axes: [bool; 3],
    /// Per world axis, whether translation along it is undetermined.
    pub weak_trans_axes: [bool; 3],
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
        self.converged
            && self.inliers >= u32::try_from(MIN_CORRESPONDENCES).unwrap_or(u32::MAX)
            && self.coverage.is_finite()
            && self.coverage >= MIN_REFINEMENT_COVERAGE_FRACTION
            && self.inlier_ratio.is_finite()
            && self.inlier_ratio > 0.0
            && self.rms.is_finite()
            && self.rms >= 0.0
            && self.median_abs.is_finite()
            && self.p95_abs.is_finite()
            && !self.weak_rot_axes.into_iter().any(|weak| weak)
            && !self.weak_trans_axes.into_iter().any(|weak| weak)
    }
}

/// One accepted moving-vertex-to-fixed-surface correspondence.
#[derive(Clone, Copy)]
struct Correspondence {
    point: DVec3,
    target: DVec3,
    normal: DVec3,
    residual: f64,
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
/// [`FitRejection::NoImprovement`] instead of silently claiming a refined pose.
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
    let initial_pose = choose_start_pose(&initial_level)?;
    let mut pose = initial_pose.rigid;
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary: Option<Summary> = None;

    for budget in [COARSE_BUDGET, DENSE_BUDGET] {
        let samples = sample_vertices(moving, budget);
        if samples.is_empty() {
            continue;
        }
        let outcome = run_level(&Level {
            moving,
            normals: &normals,
            fixed,
            moving_surface: moving_surface.as_ref(),
            fixed_samples: &fixed_samples,
            samples: &samples,
            settings,
            cancel,
            start: pose,
        });
        let level = match outcome {
            Ok(level) => level,
            // Preserve a successful earlier level when a later level has too
            // few correspondences or cancellation arrives between levels.
            Err(rejection) if summary.is_some() => {
                debug_assert!(
                    matches!(
                        rejection,
                        FitRejection::TooFewPairs { .. } | FitRejection::NoImprovement
                    ),
                    "an unexpected rejection is being swallowed: {rejection:?}"
                );
                break;
            }
            Err(rejection) => return Err(rejection),
        };
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
    // Measure displacement at the mesh centre; the pose translation column can
    // change during rotation even when the geometry moves little.
    let moved_by = (pose.apply(center) - start.apply(center)).length();
    // Two bounded stages make up this total: the coarse hypothesis
    // (`choose_start_pose` admits nothing beyond COARSE_MAX_SHIFT_FACTOR over
    // the moving extent plus the influence radius) and the refinement, which is
    // bounded by the moving mesh's own size plus the influence radius. Read
    // against their sum, the guard refuses a refine that wandered off the patch
    // it was seated on; it is not a second opinion on the coarse stage.
    let allowed = extent.max(1.0) + initial_pose.coarse_shift + settings.influence_radius_mm.abs();
    if moved_by > allowed {
        return Err(FitRejection::Runaway { moved_by, allowed });
    }

    Ok(IcpReport {
        rigid: pose,
        iterations,
        converged,
        inliers: summary.inliers,
        inlier_ratio: summary.inlier_ratio,
        coverage: summary.coverage,
        rms: summary.rms,
        median_abs: summary.median_abs,
        p95_abs: summary.p95_abs,
        weak_rot_axes: summary.weak_rot_axes,
        weak_trans_axes: summary.weak_trans_axes,
    })
}

/// The report for a run that was cancelled before it did anything.
fn idle_report(start: Rigid) -> IcpReport {
    IcpReport {
        rigid: start,
        iterations: 0,
        converged: false,
        inliers: 0,
        inlier_ratio: 0.0,
        coverage: 0.0,
        rms: 0.0,
        median_abs: 0.0,
        p95_abs: 0.0,
        weak_rot_axes: [true; 3],
        weak_trans_axes: [true; 3],
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

/// Search radii from conservative to the operator's configured maximum.
///
/// A broad influence distance can connect two neighbouring teeth, so it is a
/// search budget rather than the first answer. The ladder never exceeds the
/// visible setting and is deterministic for every input.
fn influence_radius_ladder(maximum: f64) -> Vec<f64> {
    if !maximum.is_finite() || maximum <= 0.0 {
        return Vec::new();
    }
    [0.25, 0.5, 1.0]
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
    /// RMS Euclidean point-to-surface distance used to accept a trial pose.
    geometric_rms: f64,
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

#[derive(Clone, Copy)]
struct CoarseCandidate {
    rigid: Rigid,
    summary: Summary,
    reciprocal: Option<ReciprocalSummary>,
    shift: f64,
    component: Option<usize>,
}

#[derive(Clone, Copy)]
struct StartPose {
    rigid: Rigid,
    coarse_shift: f64,
}

struct GlobalSeedContext<'a> {
    samples: &'a [u32],
    moving_anchor: DVec3,
    moving_center: DVec3,
    orientations: &'a [DQuat],
    max_shift: f64,
}

const COARSE_TIE_RELATIVE_RMS: f64 = 0.02;
const COARSE_TIE_COVERAGE: f64 = 0.02;
/// Fraction of the incumbent's forward coverage a candidate must keep to win on
/// residual alone. Relative, not absolute: an absolute allowance lifted the 1%
/// search floor to an effective 3%, which is exactly the regime where a small
/// patch competes with the operator's own start.
const COARSE_COVERAGE_KEEP_FRACTION: f64 = 0.9;
const COARSE_RECIPROCAL_ADVANTAGE: f64 = 0.01;
const COARSE_RECIPROCAL_RMS_FACTOR: f64 = 1.25;
const COARSE_POSE_TRANSLATION_EPS_MM: f64 = 0.01;
const COARSE_POSE_ROTATION_EPS_RAD: f64 = 0.01;
const COARSE_TIE_SHIFT_MM: f64 = 0.5;
const COARSE_MAX_SHIFT_FACTOR: f64 = 4.0;
/// Samples used by the bounded global seed search. This is intentionally much
/// smaller than either ICP level: it locates a plausible patch, then the
/// existing full-resolution objective and reciprocal guard decide whether it
/// is real.
const GLOBAL_SEED_SAMPLE_BUDGET: usize = 512;
/// Surface anchors tried by the global seed search. A triangle representative
/// is a usable surface point even when the moving scan is only a crop of a
/// larger connected scan.
const GLOBAL_ANCHOR_BUDGET: usize = 384;
/// Full-resolution candidates retained after the cheap anchor pass.
const GLOBAL_CANDIDATE_BUDGET: usize = 16;
/// Iterations spent locally refining each global seed before comparing seeds.
/// The normal refinement pass still runs afterwards with the operator's full
/// budget; this bounded pass only prevents a smooth wrong patch from winning
/// on its unrefined anchor residual.
const GLOBAL_SEED_REFINE_ITERATIONS: u32 = 8;
/// Do not pay for a global anchor sweep when the current pose already explains
/// most of the moving samples. A weak local overlap still triggers recovery.
const GLOBAL_SEED_MIN_FORWARD_COVERAGE: f64 = 0.5;

/// Prefer a centered coarse hypothesis when it clearly explains more of the
/// same surface than the caller's rough pose.
///
/// A point-to-plane step cannot see translation tangent to a locally flat patch.
/// When two scans are merely close and shifted sideways, it can therefore settle
/// on the edge it first touched. The bounding-box hypothesis is only a candidate:
/// it wins when the same correspondence objective improves without losing
/// coverage, so partial scans and adjacent anatomy keep the explicit start.
fn choose_start_pose(level: &Level<'_>) -> Result<StartPose, FitRejection> {
    if level.samples.is_empty() || level.cancel.is_cancelled() {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    }
    let Some((moving_center, moving_extent)) = bounds_of(level.moving) else {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    };
    let seed_samples = sample_vertices(level.moving, GLOBAL_SEED_SAMPLE_BUDGET);
    let moving_anchor = sampled_centroid(level.moving, &seed_samples).unwrap_or(moving_center);
    let score = |pose: Rigid| score_candidate(level, pose);
    let mut candidates = Vec::new();
    if let Some((summary, reciprocal)) = score(level.start) {
        candidates.push(CoarseCandidate {
            rigid: level.start,
            summary,
            reciprocal,
            shift: 0.0,
            component: nearest_component_index(level, level.start, moving_center),
        });
    }

    // A component centre is a global coarse hypothesis, not a local radius
    // clamp. The visible influence radius controls correspondence distance;
    // the component bounds provide the bounded set of plausible poses. This
    // is what lets a rough side-by-side placement recover without making the
    // ICP kernel walk an unbounded translation grid.
    let orientation_deltas = coarse_orientation_deltas();
    let max_coarse_shift = moving_extent.max(1.0) * COARSE_MAX_SHIFT_FACTOR
        + level.settings.influence_radius_mm.abs() * COARSE_MAX_SHIFT_FACTOR;
    for (component_index, &(fixed_min, fixed_max)) in
        level.fixed.component_bounds().iter().enumerate()
    {
        let fixed_center = (fixed_min + fixed_max) * 0.5;
        for (delta_index, &delta) in orientation_deltas.iter().enumerate() {
            // `Inverted` is an explicit winding-repair escape hatch. Do not
            // manufacture an upside-down physical pose merely because that
            // would make an otherwise same-facing surface satisfy the setting.
            // The identity candidate still permits a genuinely inverted fixed
            // mesh to refine; global orientation recovery belongs to the normal
            // matching modes.
            if level.settings.orientation == Orientation::Inverted && delta_index != 0 {
                continue;
            }
            // Corrections are expressed in world space. Centering around the
            // fixed component makes the translation candidate explicit; the
            // final displacement guard below expands to cover that proven
            // coarse move instead of rejecting it as a runaway.
            let rotation = delta * level.start.rotation;
            let candidate = Rigid::new(rotation, fixed_center - rotation * moving_center);
            let shift =
                (candidate.apply(moving_center) - level.start.apply(moving_center)).length();
            if !shift.is_finite() || shift > max_coarse_shift {
                continue;
            }
            let Some((summary, reciprocal)) = score(candidate) else {
                continue;
            };
            candidates.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal,
                shift,
                component: Some(component_index),
            });
        }
    }

    let needs_global_seed = candidates.is_empty()
        || candidates
            .iter()
            .all(|candidate| candidate.summary.coverage < GLOBAL_SEED_MIN_FORWARD_COVERAGE);
    if needs_global_seed {
        let global = global_seed_candidates(
            level,
            GlobalSeedContext {
                samples: &seed_samples,
                moving_anchor,
                moving_center,
                orientations: &orientation_deltas,
                max_shift: max_coarse_shift,
            },
        );
        candidates.extend(global);
        refine_seed_candidates(level, &mut candidates, moving_center);
    }

    let Some(best) = candidates.iter().copied().reduce(|current, candidate| {
        if coarse_candidate_is_better(&candidate, &current) {
            candidate
        } else {
            current
        }
    }) else {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    };

    if candidates.iter().copied().any(|candidate| {
        candidate.component != best.component
            && candidate.shift <= best.shift + COARSE_TIE_SHIFT_MM
            && poses_are_distinct(candidate.rigid, best.rigid)
            && coarse_candidates_are_equivalent(&candidate, &best)
    }) {
        return Err(FitRejection::Ambiguous);
    }

    Ok(StartPose {
        rigid: best.rigid,
        coarse_shift: best.shift,
    })
}

/// Search a bounded set of fixed-surface anchors for a crop whose current
/// pose has no useful overlap. The cheap sample pass keeps the number of full
/// nearest-surface evaluations bounded; all returned candidates still use the
/// same reciprocal evidence as every other coarse hypothesis.
fn global_seed_candidates(
    level: &Level<'_>,
    context: GlobalSeedContext<'_>,
) -> Vec<CoarseCandidate> {
    if context.samples.is_empty() {
        return Vec::new();
    }
    let anchor_level = Level {
        moving: level.moving,
        normals: level.normals,
        fixed: level.fixed,
        moving_surface: level.moving_surface,
        fixed_samples: level.fixed_samples,
        samples: context.samples,
        settings: level.settings,
        cancel: level.cancel,
        start: level.start,
    };
    let mut hypotheses = Vec::new();
    for (anchor_index, anchor) in level
        .fixed
        .representative_samples(GLOBAL_ANCHOR_BUDGET)
        .into_iter()
        .enumerate()
    {
        if anchor_index % 32 == 0 && level.cancel.is_cancelled() {
            break;
        }
        for (delta_index, &delta) in context.orientations.iter().enumerate() {
            if level.settings.orientation == Orientation::Inverted && delta_index != 0 {
                continue;
            }
            let rotation = delta * level.start.rotation;
            let candidate = Rigid::new(rotation, anchor.point - rotation * context.moving_anchor);
            let shift = (candidate.apply(context.moving_center)
                - level.start.apply(context.moving_center))
            .length();
            if !shift.is_finite() || shift > context.max_shift {
                continue;
            }
            let Some(summary) = forward_summary(&anchor_level, candidate) else {
                continue;
            };
            hypotheses.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal: None,
                shift,
                component: Some(anchor.component),
            });
        }
    }

    // The acceptance comparator intentionally has tolerance bands and is not
    // a total order. A sort comparator must be one: use a deterministic
    // lexicographic ranking for this cheap shortlist, then return to the
    // tolerance-aware comparator once the full evidence is available.
    hypotheses.sort_by(coarse_seed_order);
    hypotheses.truncate(GLOBAL_CANDIDATE_BUDGET);
    hypotheses
        .into_iter()
        .filter_map(|hypothesis| {
            if level.cancel.is_cancelled() {
                return None;
            }
            score_candidate(level, hypothesis.rigid).map(|(summary, reciprocal)| CoarseCandidate {
                rigid: hypothesis.rigid,
                summary,
                reciprocal,
                shift: hypothesis.shift,
                component: hypothesis.component,
            })
        })
        .collect()
}

/// Let each retained global anchor take a short local ICP step before coarse
/// ranking. A one-shot point-to-surface residual can make a smooth wrong patch
/// look better than the distinctive patch that actually continues to converge.
fn refine_seed_candidates(
    level: &Level<'_>,
    candidates: &mut [CoarseCandidate],
    moving_center: DVec3,
) {
    if candidates.is_empty() {
        return;
    }
    let seed_settings = RefineSettings {
        max_iterations: level
            .settings
            .max_iterations
            .min(GLOBAL_SEED_REFINE_ITERATIONS),
        ..*level.settings
    };
    for candidate in candidates {
        if level.cancel.is_cancelled() {
            break;
        }
        let local_level = Level {
            moving: level.moving,
            normals: level.normals,
            fixed: level.fixed,
            moving_surface: level.moving_surface,
            fixed_samples: level.fixed_samples,
            samples: level.samples,
            settings: &seed_settings,
            cancel: level.cancel,
            start: candidate.rigid,
        };
        let Ok(local) = run_level(&local_level) else {
            continue;
        };
        if !local.summary.geometric_rms.is_finite()
            || local.summary.geometric_rms > candidate.summary.geometric_rms
        {
            continue;
        }
        let reciprocal = reciprocal_evidence(level, local.pose, level.settings.influence_radius_mm);
        if !reciprocal_evidence_is_usable(level, reciprocal) {
            continue;
        }
        candidate.rigid = local.pose;
        candidate.summary = local.summary;
        candidate.reciprocal = reciprocal;
        candidate.shift =
            (local.pose.apply(moving_center) - level.start.apply(moving_center)).length();
    }
}

/// Score a pose against the forward surface and, when possible, the bounded
/// reverse surface. Keeping this in one function prevents the global seed pass
/// from accidentally becoming an acceptance path with weaker evidence.
fn score_candidate(level: &Level<'_>, pose: Rigid) -> Option<(Summary, Option<ReciprocalSummary>)> {
    let summary = forward_summary(level, pose)?;
    let reciprocal = reciprocal_evidence(level, pose, level.settings.influence_radius_mm);
    reciprocal_evidence_is_usable(level, reciprocal).then_some((summary, reciprocal))
}

/// Calculate only the forward objective for a coarse seed or a full candidate.
fn forward_summary(level: &Level<'_>, pose: Rigid) -> Option<Summary> {
    let found = correspondences(level, pose, level.settings.influence_radius_mm);
    let matched = found.iter().flatten().count();
    if !forward_coverage_is_sufficient(matched, level.samples.len()) {
        return None;
    }
    let kept = trim(&found, level.settings.matching_ratio);
    if kept.len() < MIN_CORRESPONDENCES {
        return None;
    }
    let (matrix, _, _) = accumulate(&kept);
    Some(summarize(&kept, matched, level.samples.len(), &matrix))
}

/// A surface crop's bounding-box centre can sit well above its actual surface
/// when curvature is strong. Use the deterministic sample centroid for global
/// anchoring, while retaining the bounds centre for displacement bookkeeping.
#[allow(clippy::cast_precision_loss)]
fn sampled_centroid(soup: Soup<'_>, samples: &[u32]) -> Option<DVec3> {
    let mut sum = DVec3::ZERO;
    let mut count = 0usize;
    for &raw in samples {
        let vertex = usize::try_from(raw).ok()?;
        let point = vertex_at(soup.positions, vertex)?;
        sum += point;
        count += 1;
    }
    (count > 0).then(|| sum / count as f64)
}

fn nearest_component_index(level: &Level<'_>, pose: Rigid, moving_center: DVec3) -> Option<usize> {
    let point = pose.apply(moving_center);
    level
        .fixed
        .component_bounds()
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_center = (left.0 + left.1) * 0.5;
            let right_center = (right.0 + right.1) * 0.5;
            point
                .distance_squared(left_center)
                .total_cmp(&point.distance_squared(right_center))
        })
        .map(|(index, _)| index)
}

/// Whether a candidate keeps enough of the incumbent's forward coverage.
fn coarse_keeps_coverage(candidate: f64, current: f64) -> bool {
    candidate >= current * COARSE_COVERAGE_KEEP_FRACTION
}

/// Whether a candidate explains materially more of the fixed surface.
///
/// Missing evidence on either side is not a gain: a point-cloud layer has no
/// reverse surface to query, and treating that absence as "explains more" let a
/// residual-only win discard most of the operator's coverage.
fn coarse_explains_more_fixed(
    candidate: Option<ReciprocalSummary>,
    current: Option<ReciprocalSummary>,
) -> bool {
    match (candidate, current) {
        (Some(candidate), Some(current)) => {
            candidate.coverage > current.coverage + COARSE_RECIPROCAL_ADVANTAGE
        }
        _ => false,
    }
}

fn coarse_candidate_is_better(candidate: &CoarseCandidate, current: &CoarseCandidate) -> bool {
    // A forward-only low residual can come from a small smooth patch. When
    // both triangle soups are available, prefer a candidate that explains
    // materially more of the fixed surface as long as its forward residual is
    // still within a bounded coarse tolerance. This is what keeps a partial
    // scan from being attracted to a visually similar but wrong window.
    if let (Some(candidate_reciprocal), Some(current_reciprocal)) =
        (candidate.reciprocal, current.reciprocal)
    {
        let reciprocal_advantage = candidate_reciprocal.coverage
            > current_reciprocal.coverage + COARSE_RECIPROCAL_ADVANTAGE
            && candidate.summary.geometric_rms
                <= current.summary.geometric_rms * COARSE_RECIPROCAL_RMS_FACTOR
            && coarse_keeps_coverage(candidate.summary.coverage, current.summary.coverage);
        if reciprocal_advantage {
            return true;
        }
    }
    if candidate.summary.geometric_rms < current.summary.geometric_rms * STALL_IMPROVEMENT {
        // A lower residual is only an improvement if it still explains the same
        // surface. A small smooth patch can beat the true seating on residual
        // alone while covering almost none of the moving scan, and the search
        // floor admits a candidate at 1% coverage. Keep the incumbent unless
        // the candidate keeps its coverage, or explains materially more of the
        // fixed surface. Coarse tolerances, not the commitment floor: the trust
        // gate still decides whether anything may be committed.
        return coarse_keeps_coverage(candidate.summary.coverage, current.summary.coverage)
            || coarse_explains_more_fixed(candidate.reciprocal, current.reciprocal);
    }
    if candidate.summary.geometric_rms > current.summary.geometric_rms * (1.0 / STALL_IMPROVEMENT) {
        return false;
    }
    if candidate.summary.coverage > current.summary.coverage + COARSE_TIE_COVERAGE {
        return true;
    }
    if candidate.summary.coverage + COARSE_TIE_COVERAGE < current.summary.coverage {
        return false;
    }
    let reciprocal_better = match (candidate.reciprocal, current.reciprocal) {
        (Some(candidate), Some(current)) => {
            candidate.coverage > current.coverage + COARSE_TIE_COVERAGE
        }
        (Some(_), None) => true,
        _ => false,
    };
    reciprocal_better || candidate.shift + COARSE_TIE_SHIFT_MM < current.shift
}

fn coarse_seed_order(left: &CoarseCandidate, right: &CoarseCandidate) -> std::cmp::Ordering {
    right
        .summary
        .coverage
        .total_cmp(&left.summary.coverage)
        .then_with(|| {
            left.summary
                .geometric_rms
                .total_cmp(&right.summary.geometric_rms)
        })
        .then_with(|| left.shift.total_cmp(&right.shift))
        .then_with(|| {
            left.rigid
                .translation
                .x
                .total_cmp(&right.rigid.translation.x)
        })
        .then_with(|| {
            left.rigid
                .translation
                .y
                .total_cmp(&right.rigid.translation.y)
        })
        .then_with(|| {
            left.rigid
                .translation
                .z
                .total_cmp(&right.rigid.translation.z)
        })
}

fn coarse_candidates_are_equivalent(candidate: &CoarseCandidate, best: &CoarseCandidate) -> bool {
    let rms_scale = candidate
        .summary
        .geometric_rms
        .max(best.summary.geometric_rms)
        .max(f64::MIN_POSITIVE);
    if (candidate.summary.geometric_rms - best.summary.geometric_rms).abs()
        > rms_scale * COARSE_TIE_RELATIVE_RMS + 1e-6
        || (candidate.summary.coverage - best.summary.coverage).abs() > COARSE_TIE_COVERAGE
    {
        return false;
    }
    match (candidate.reciprocal, best.reciprocal) {
        (Some(candidate), Some(best)) => {
            (candidate.coverage - best.coverage).abs() <= COARSE_TIE_COVERAGE
        }
        (None, None) => true,
        _ => false,
    }
}

fn poses_are_distinct(left: Rigid, right: Rigid) -> bool {
    (left.translation - right.translation).length() > COARSE_POSE_TRANSLATION_EPS_MM
        || (left.rotation * right.rotation.inverse())
            .to_scaled_axis()
            .length()
            > COARSE_POSE_ROTATION_EPS_RAD
}

/// Bounded global orientation probes used before local point-to-plane ICP.
///
/// Importers and manual placement commonly leave a scan quarter-turned or
/// upside down while its centre is already near the target. Comparing only the
/// current rotation then lets the first tangent patch win and can return a
/// sideways pose. These 24 cube orientations cover the principal dental-CAD
/// axis conventions at a fixed, deterministic cost; the subsequent ICP remains
/// responsible for fine arbitrary-angle correction.
fn coarse_orientation_deltas() -> [DQuat; 24] {
    let quarter = std::f64::consts::FRAC_PI_2;
    let turns = [0.0, quarter, std::f64::consts::PI, -quarter];
    core::array::from_fn(|index| {
        let face = index / 4;
        let turn = turns[index % 4];
        match face {
            0 => DQuat::from_euler(EulerRot::XYZ, 0.0, 0.0, turn),
            1 => DQuat::from_euler(EulerRot::XYZ, quarter, 0.0, turn),
            2 => DQuat::from_euler(EulerRot::XYZ, -quarter, 0.0, turn),
            3 => DQuat::from_euler(EulerRot::XYZ, std::f64::consts::PI, 0.0, turn),
            4 => DQuat::from_euler(EulerRot::XYZ, 0.0, quarter, turn),
            _ => DQuat::from_euler(EulerRot::XYZ, 0.0, -quarter, turn),
        }
    })
}

/// Run one resolution level to convergence or to its iteration ceiling.
fn run_level(level: &Level<'_>) -> Result<LevelOutcome, FitRejection> {
    let mut pose = level.start;
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary: Option<Summary> = None;
    let mut best_rms = f64::INFINITY;
    let mut has_accepted_step = false;
    // Keep the pose associated with the best residual.
    let mut best: Option<(Rigid, Summary)> = None;
    let radii = influence_radius_ladder(level.settings.influence_radius_mm);
    let Some(mut radius_slot) = (!radii.is_empty()).then_some(0usize) else {
        return Err(FitRejection::TooFewPairs {
            have: 0,
            need: MIN_CORRESPONDENCES,
        });
    };

    for _ in 0..level.settings.max_iterations {
        if level.cancel.is_cancelled() {
            break;
        }
        let (found, matched) = correspondences_at_radius(level, pose, &radii, &mut radius_slot)?;
        let kept = trim(&found, level.settings.matching_ratio);
        if kept.len() < MIN_CORRESPONDENCES {
            return Err(FitRejection::TooFewPairs {
                have: kept.len(),
                need: MIN_CORRESPONDENCES,
            });
        }
        let (normal_matrix, gradient, centre) = accumulate(&kept);
        let measured = summarize(&kept, matched, level.samples.len(), &normal_matrix);
        let measured_reciprocal = reciprocal_evidence(level, pose, radii[radius_slot]);
        if !reciprocal_evidence_is_usable(level, measured_reciprocal) {
            return Err(FitRejection::NoImprovement);
        }
        summary = Some(measured);
        // `measured` describes the pose the correspondences were found AT, not
        // the one the step below produces. Remember the pair together.
        if measured.geometric_rms.is_finite()
            && measured.geometric_rms < best_rms * STALL_IMPROVEMENT
        {
            best_rms = measured.geometric_rms;
            best = Some((pose, measured));
        }

        let Some(step) = solve_damped(&normal_matrix, &gradient) else {
            if !has_accepted_step && measured.geometric_rms > CONVERGED_RESIDUAL_MM {
                return Err(FitRejection::NoImprovement);
            }
            converged = measured.geometric_rms <= CONVERGED_RESIDUAL_MM;
            break;
        };
        let rotation = DVec3::new(step[0], step[1], step[2]);
        let translation = DVec3::new(step[3], step[4], step[5]);
        let Some((next_pose, next_summary)) = try_backtracked_step(
            level,
            TrialState {
                pose,
                centre,
                rotation,
                translation,
                measured,
                measured_reciprocal,
                radius: radii[radius_slot],
            },
        ) else {
            // Never apply a step that was not evaluated as an improvement. The
            // previous implementation did, so a nearest-surface change could
            // rotate a rough pair sideways while its report still looked valid.
            if !has_accepted_step && measured.geometric_rms > CONVERGED_RESIDUAL_MM {
                return Err(FitRejection::NoImprovement);
            }
            converged = rotation.length() < CONVERGED_ROTATION
                && translation.length() < CONVERGED_TRANSLATION
                && measured.geometric_rms <= CONVERGED_RESIDUAL_MM;
            break;
        };
        pose = next_pose;
        // The trial was fully re-evaluated before it was accepted. Keep its
        // summary with the pose so a convergence break cannot report metrics
        // for the pre-step correspondences.
        summary = Some(next_summary);
        if next_summary.geometric_rms.is_finite() && next_summary.geometric_rms < best_rms {
            best_rms = next_summary.geometric_rms;
            best = Some((next_pose, next_summary));
        }
        has_accepted_step = true;
        iterations += 1;
        if rotation.length() < CONVERGED_ROTATION && translation.length() < CONVERGED_TRANSLATION {
            converged = true;
            break;
        }
    }

    // A stalled level returns its best residual; a converged level returns its
    // final pose.
    let settled = if converged {
        summary.map(|summary| (pose, summary))
    } else {
        best.or_else(|| summary.map(|summary| (pose, summary)))
    };
    let Some((pose, summary)) = settled else {
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

/// Find each sampled vertex's nearest fixed surface point under `pose`.
///
/// Parallel because it is pure: every entry reads only its own vertex, and the
/// output keeps sample order, so the fold that follows stays deterministic.
fn correspondences(
    level: &Level<'_>,
    pose: Rigid,
    influence_radius_mm: f64,
) -> Vec<Option<Correspondence>> {
    level
        .samples
        .par_iter()
        .map(|&raw| {
            let vertex = raw as usize;
            let local = vertex_at(level.moving.positions, vertex)?;
            let point = pose.apply(local);
            let hit = level.fixed.nearest(point, influence_radius_mm)?;
            let moving_normal = pose.apply_normal(level.normals.get(vertex).copied()?);
            let agreement = moving_normal.dot(hit.normal);
            let accepted = match level.settings.orientation {
                Orientation::Match => agreement > 0.0,
                Orientation::Inverted => agreement < 0.0,
                Orientation::Ignored => true,
            };
            if !accepted {
                return None;
            }
            Some(Correspondence {
                point,
                target: hit.point,
                normal: hit.normal,
                residual: (point - hit.point).dot(hit.normal),
            })
        })
        .collect()
}

/// Keep the closest `ratio` of correspondences, in sample order.
///
/// The cutoff value is chosen from a sorted copy, then applied by a pass in
/// sample order, so the kept set is a deterministic subsequence rather than a
/// sort-order artefact. Full point-to-surface distance is used here rather than
/// only the normal residual: a tangent slide can have a tiny plane error while
/// still being a geometrically poor match.
fn trim(found: &[Option<Correspondence>], ratio: f64) -> Vec<Correspondence> {
    let mut magnitudes: Vec<f64> = found
        .iter()
        .flatten()
        .map(|entry| (entry.point - entry.target).length())
        .collect();
    if magnitudes.is_empty() {
        return Vec::new();
    }
    magnitudes.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let wanted = ((magnitudes.len() as f64) * ratio.clamp(0.0, 1.0)).ceil() as usize;
    let cutoff = magnitudes
        .get(wanted.clamp(1, magnitudes.len()) - 1)
        .copied()
        .unwrap_or(f64::INFINITY);
    found
        .iter()
        .flatten()
        .filter(|entry| (entry.point - entry.target).length() <= cutoff)
        .copied()
        .collect()
}

/// Build the point-to-plane normal equations, folded in sample order.
fn accumulate(kept: &[Correspondence]) -> ([[f64; 6]; 6], [f64; 6], DVec3) {
    #[allow(clippy::cast_precision_loss)]
    let count = kept.len().max(1) as f64;
    let centre = kept
        .iter()
        .fold(DVec3::ZERO, |total, entry| total + entry.point)
        / count;

    let mut magnitudes: Vec<f64> = kept.iter().map(|entry| entry.residual.abs()).collect();
    magnitudes.sort_by(f64::total_cmp);
    let median = magnitudes.get(magnitudes.len() / 2).copied().unwrap_or(0.0);
    let huber = median * HUBER_FACTOR;

    let mut matrix = [[0.0f64; 6]; 6];
    let mut gradient = [0.0f64; 6];
    for entry in kept {
        let moment = (entry.point - centre).cross(entry.normal);
        let jacobian = [
            moment.x,
            moment.y,
            moment.z,
            entry.normal.x,
            entry.normal.y,
            entry.normal.z,
        ];
        let magnitude = entry.residual.abs();
        let weight = if huber > f64::MIN_POSITIVE && magnitude > huber {
            huber / magnitude
        } else {
            1.0
        };
        for row in 0..6 {
            gradient[row] -= weight * entry.residual * jacobian[row];
            for column in 0..6 {
                matrix[row][column] += weight * jacobian[row] * jacobian[column];
            }
        }
    }
    (matrix, gradient, centre)
}

/// Residual statistics and the degrees of freedom the geometry left free.
fn summarize(
    kept: &[Correspondence],
    matched: usize,
    sampled: usize,
    matrix: &[[f64; 6]; 6],
) -> Summary {
    let mut magnitudes: Vec<f64> = kept.iter().map(|entry| entry.residual.abs()).collect();
    magnitudes.sort_by(f64::total_cmp);
    #[allow(clippy::cast_precision_loss)]
    let count = kept.len().max(1) as f64;
    let sum_squares: f64 = kept.iter().map(|e| e.residual * e.residual).sum();
    let geometric_sum_squares: f64 = kept
        .iter()
        .map(|entry| (entry.point - entry.target).length_squared())
        .sum();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let p95_slot = ((magnitudes.len() as f64) * 0.95).ceil() as usize;

    let largest = (0..6).fold(0.0f64, |best, index| best.max(matrix[index][index]));
    let limit = largest * WEAK_AXIS_FRACTION;
    let weak = |offset: usize| {
        [
            matrix[offset][offset] <= limit,
            matrix[offset + 1][offset + 1] <= limit,
            matrix[offset + 2][offset + 2] <= limit,
        ]
    };

    #[allow(clippy::cast_precision_loss)]
    let sampled_count = sampled.max(1) as f64;
    Summary {
        inliers: u32::try_from(kept.len()).unwrap_or(u32::MAX),
        inlier_ratio: count / sampled_count,
        #[allow(clippy::cast_precision_loss)]
        coverage: matched as f64 / sampled_count,
        rms: (sum_squares / count).sqrt(),
        geometric_rms: (geometric_sum_squares / count).sqrt(),
        median_abs: magnitudes.get(magnitudes.len() / 2).copied().unwrap_or(0.0),
        p95_abs: magnitudes
            .get(p95_slot.clamp(1, magnitudes.len()) - 1)
            .copied()
            .unwrap_or(0.0),
        weak_rot_axes: weak(0),
        weak_trans_axes: weak(3),
    }
}

/// Solve the damped normal equations, growing the damping until the system is
/// positive definite or the retries run out.
fn solve_damped(matrix: &[[f64; 6]; 6], gradient: &[f64; 6]) -> Option<[f64; 6]> {
    let mut damping = INITIAL_DAMPING;
    for _ in 0..=MAX_DAMPING_RETRIES {
        let mut damped = *matrix;
        for (index, row) in damped.iter_mut().enumerate() {
            row[index] += damping * row[index].max(f64::MIN_POSITIVE);
        }
        if let Some(step) = solve_cholesky(&damped, gradient) {
            if step.iter().all(|value| value.is_finite()) {
                return Some(step);
            }
        }
        damping *= DAMPING_GROWTH;
    }
    None
}

/// Cholesky solve for a symmetric positive-definite 6x6.
fn solve_cholesky(matrix: &[[f64; 6]; 6], gradient: &[f64; 6]) -> Option<[f64; 6]> {
    let mut lower = [[0.0f64; 6]; 6];
    for row in 0..6 {
        for column in 0..=row {
            let mut sum = matrix[row][column];
            #[expect(
                clippy::needless_range_loop,
                reason = "fixed 6x6 Cholesky indices preserve bit-repeatability"
            )]
            for inner in 0..column {
                sum -= lower[row][inner] * lower[column][inner];
            }
            if row == column {
                if sum <= f64::MIN_POSITIVE {
                    return None;
                }
                lower[row][row] = sum.sqrt();
            } else {
                lower[row][column] = sum / lower[column][column];
            }
        }
    }
    let mut forward = [0.0f64; 6];
    for row in 0..6 {
        let mut sum = gradient[row];
        for (inner, solved) in forward.iter().enumerate().take(row) {
            sum -= lower[row][inner] * solved;
        }
        forward[row] = sum / lower[row][row];
    }
    let mut step = [0.0f64; 6];
    for row in (0..6).rev() {
        let mut sum = forward[row];
        for inner in (row + 1)..6 {
            sum -= lower[inner][row] * step[inner];
        }
        step[row] = sum / lower[row][row];
    }
    Some(step)
}

/// Compose a small world-space step about `centre` onto the current pose.
fn apply_step(pose: Rigid, centre: DVec3, rotation: DVec3, translation: DVec3) -> Rigid {
    let delta_rotation = DQuat::from_scaled_axis(rotation);
    let basis = DMat3::from_quat(delta_rotation);
    let delta = Rigid::new(delta_rotation, centre - basis * centre + translation);
    delta.compose(&pose)
}
