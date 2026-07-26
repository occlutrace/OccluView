//! REFINE: trimmed point-to-plane ICP against the fixed surface.
//!
//! Two resolutions. A coarse sample closes the bulk motion, a dense sample
//! finishes; that converges from farther away than a single dense pass and
//! costs less than one.
//!
//! The correspondence search is parallel — it is pure — but the normal
//! equations are folded **serially in sample order**. A parallel reduction over
//! floating-point sums gives a different answer for a different thread count,
//! and a registration that lands a micron away depending on the machine it ran
//! on is not a measurement.

use glam::{DMat3, DQuat, DVec3};
use rayon::prelude::*;

use crate::pairs::FitRejection;
use crate::sample::{extent_of, sample_vertices, vertex_at, vertex_normals};
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

/// Samples used by the coarse level.
const COARSE_BUDGET: usize = 8_000;
/// Samples used by the dense level.
const DENSE_BUDGET: usize = 40_000;

/// Correspondences below this leave the fit undetermined.
const MIN_CORRESPONDENCES: usize = 6;

/// Huber cut as a multiple of the median absolute residual — the usual 95%
/// efficiency constant for a normal error model.
const HUBER_FACTOR: f64 = 1.345;

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

/// An iteration counts as an improvement only if it cuts the residual by more
/// than this factor. A tenth of a percent is far below anything a scan can
/// resolve, so anything slower than that is wandering, not converging.
const STALL_IMPROVEMENT: f64 = 0.999;

/// Consecutive non-improving iterations before a level gives up. Three, so a
/// single flat step between two real ones cannot end the fit early.
const STALL_ROUNDS: u32 = 3;

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
    /// This is exocad's "maximum influence distance".
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
/// its own size.
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
    if cancel.is_cancelled() {
        return Ok(idle_report(start));
    }

    let normals = vertex_normals(moving);
    let extent = extent_of(moving);
    let mut pose = start;
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary: Option<Summary> = None;

    for budget in [COARSE_BUDGET, DENSE_BUDGET] {
        let samples = sample_vertices(moving, budget);
        if samples.is_empty() {
            continue;
        }
        let level = run_level(&Level {
            moving,
            normals: &normals,
            fixed,
            samples: &samples,
            settings,
            cancel,
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
    let moved_by = (pose.translation - start.translation).length();
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
    samples: &'a [u32],
    settings: &'a RefineSettings,
    cancel: &'a CancelFlag,
    start: Rigid,
}

/// Statistics carried out of a level.
#[derive(Clone, Copy)]
struct Summary {
    inliers: u32,
    inlier_ratio: f64,
    coverage: f64,
    rms: f64,
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

/// Run one resolution level to convergence or to its iteration ceiling.
fn run_level(level: &Level<'_>) -> Result<LevelOutcome, FitRejection> {
    let mut pose = level.start;
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary: Option<Summary> = None;
    let mut best_rms = f64::INFINITY;
    let mut stalled = 0u32;
    // The pose the best residual was measured AT, kept so a level that gives up
    // returns its best answer rather than wherever it happened to wander to.
    let mut best: Option<(Rigid, Summary)> = None;

    for _ in 0..level.settings.max_iterations {
        if level.cancel.is_cancelled() {
            break;
        }
        let found = correspondences(level, pose);
        let matched = found.iter().flatten().count();
        if matched < MIN_CORRESPONDENCES {
            return Err(FitRejection::TooFewPairs {
                have: matched,
                need: MIN_CORRESPONDENCES,
            });
        }
        let kept = trim(&found, level.settings.matching_ratio);
        if kept.len() < MIN_CORRESPONDENCES {
            return Err(FitRejection::TooFewPairs {
                have: kept.len(),
                need: MIN_CORRESPONDENCES,
            });
        }
        let (normal_matrix, gradient, centre) = accumulate(&kept);
        let measured = summarize(&kept, matched, level.samples.len(), &normal_matrix);
        summary = Some(measured);
        // `measured` describes the pose the correspondences were found AT, not
        // the one the step below produces. Remember the pair together.
        if measured.rms.is_finite() && measured.rms < best_rms * STALL_IMPROVEMENT {
            best_rms = measured.rms;
            best = Some((pose, measured));
            stalled = 0;
        } else {
            stalled += 1;
        }

        let Some(step) = solve_damped(&normal_matrix, &gradient) else {
            break;
        };
        let rotation = DVec3::new(step[0], step[1], step[2]);
        let translation = DVec3::new(step[3], step[4], step[5]);
        pose = apply_step(pose, centre, rotation, translation);
        iterations += 1;
        if rotation.length() < CONVERGED_ROTATION && translation.length() < CONVERGED_TRANSLATION {
            converged = true;
            break;
        }

        // Stop when the residual stops falling.
        //
        // The pose-delta test above is the textbook exit and it is the right
        // one for a pair that really does converge: a scan nudged off its own
        // position settles in five iterations. It never fires for a pair that
        // CANNOT converge, though, and exocad's own warning says why — best-fit
        // matching is for identically shaped meshes. Give it an arch with
        // crowns on it against the same arch without, and the step never gets
        // small, it just wanders. Left alone that is every core at full tilt
        // for the whole iteration ceiling, and it does not even end up at its
        // own best answer: on a real 942k-vertex pair the ceiling run finished
        // at 0.42 mm having passed through 0.19 mm on the way.
        if stalled >= STALL_ROUNDS {
            break;
        }
    }

    // A level that gave up hands back the best pose it saw, not the last one it
    // wandered to. A level that converged hands back where it converged.
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
fn correspondences(level: &Level<'_>, pose: Rigid) -> Vec<Option<Correspondence>> {
    level
        .samples
        .par_iter()
        .map(|&raw| {
            let vertex = raw as usize;
            let local = vertex_at(level.moving.positions, vertex)?;
            let point = pose.apply(local);
            let hit = level
                .fixed
                .nearest(point, level.settings.influence_radius_mm)?;
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
/// sort-order artefact.
fn trim(found: &[Option<Correspondence>], ratio: f64) -> Vec<Correspondence> {
    let mut magnitudes: Vec<f64> = found
        .iter()
        .flatten()
        .map(|entry| entry.residual.abs())
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
        .filter(|entry| entry.residual.abs() <= cutoff)
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
