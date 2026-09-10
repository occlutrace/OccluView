//! Trimmed point-to-plane ICP against the fixed surface.
//!
//! Refinement runs at coarse and dense sample resolutions.
//!
//! Correspondence search is parallel, while normal equations are accumulated
//! serially in sample order to keep floating-point results deterministic.

use glam::{DMat3, DQuat, DVec3};
use rayon::prelude::*;

use crate::pairs::FitRejection;
use crate::sample::{bounds_of, sample_vertices, vertex_at, vertex_normals};
use crate::surface::SurfaceSample;
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

#[path = "icp_overlap.rs"]
mod icp_overlap;
use icp_overlap::{reciprocal_coverage_ok, reciprocal_evidence, reciprocal_evidence_is_usable};
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
/// its own size. A surface with enough pairs but no accepted improvement returns
/// [`FitRejection::NoImprovement`] instead of silently claiming a refined pose.
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
    let mut pose = choose_start_pose(&initial_level);
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
    let allowed = extent.max(1.0);
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

/// Prefer a centered coarse hypothesis when it clearly explains more of the
/// same surface than the caller's rough pose.
///
/// A point-to-plane step cannot see translation tangent to a locally flat patch.
/// When two scans are merely close and shifted sideways, it can therefore settle
/// on the edge it first touched. The bounding-box hypothesis is only a candidate:
/// it wins when the same correspondence objective improves without losing
/// coverage, so partial scans and adjacent anatomy keep the explicit start.
fn choose_start_pose(level: &Level<'_>) -> Rigid {
    if level.samples.is_empty() || level.cancel.is_cancelled() {
        return level.start;
    }
    let Some((moving_center, _moving_extent)) = bounds_of(level.moving) else {
        return level.start;
    };
    let score = |pose: Rigid| {
        let found = correspondences(level, pose, level.settings.influence_radius_mm);
        let matched = found.iter().flatten().count();
        if !forward_coverage_is_sufficient(matched, level.samples.len()) {
            return None;
        }
        let kept = trim(&found, level.settings.matching_ratio);
        (kept.len() >= MIN_CORRESPONDENCES).then(|| {
            let (matrix, _, _) = accumulate(&kept);
            (
                summarize(&kept, matched, level.samples.len(), &matrix),
                reciprocal_evidence(level, pose, level.settings.influence_radius_mm),
            )
        })
    };
    let base = score(level.start);
    let mut best = base.map(|score| (level.start, score, f64::INFINITY));
    // A component centre is a bounded translation hypothesis, not permission
    // to undo a coarse rotation or perform a hidden global registration. Keep
    // it inside the same visible search budget that the correspondence ladder
    // can justify. The mesh extent is deliberately not a fallback here: a
    // large arch must not make an arbitrary far-away component eligible.
    let max_shift = level.settings.influence_radius_mm.abs() * 2.0;
    for &(fixed_min, fixed_max) in level.fixed.component_bounds() {
        let fixed_center = (fixed_min + fixed_max) * 0.5;
        let candidate = Rigid::new(
            level.start.rotation,
            fixed_center - level.start.rotation * moving_center,
        );
        let shift = (candidate.translation - level.start.translation).length();
        if !shift.is_finite() || shift > max_shift {
            continue;
        }
        let Some(centered) = score(candidate) else {
            continue;
        };
        let baseline_coverage_ok = reciprocal_evidence_is_usable(level, centered.1)
            && base.is_none_or(|(_, base_reciprocal)| {
                reciprocal_coverage_ok(base_reciprocal, centered.1)
            });
        let Some((_, current, current_shift)) = best else {
            best = Some((candidate, centered, shift));
            continue;
        };
        let better = centered.0.geometric_rms < current.0.geometric_rms * STALL_IMPROVEMENT;
        let reciprocal_tie_break = match (current.1, centered.1) {
            (Some(current_reciprocal), Some(candidate_reciprocal)) => {
                candidate_reciprocal.coverage > current_reciprocal.coverage + 0.05
                    && candidate_reciprocal.geometric_rms
                        <= current_reciprocal.geometric_rms * 1.25 + 1e-9
                    && centered.0.geometric_rms
                        <= current.0.geometric_rms * (1.0 / STALL_IMPROVEMENT)
            }
            _ => false,
        };
        let tied_and_closer = (centered.0.geometric_rms - current.0.geometric_rms).abs()
            <= f64::EPSILON
            && shift < current_shift;
        if baseline_coverage_ok && (better || reciprocal_tie_break || tied_and_closer) {
            best = Some((candidate, centered, shift));
        }
    }
    best.map_or(level.start, |(pose, _, _)| pose)
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
