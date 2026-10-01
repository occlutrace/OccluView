//! Deterministic point-to-plane solve for one ICP resolution level.

use glam::{DMat3, DQuat, DVec3};
use rayon::prelude::*;

use crate::pairs::FitRejection;
use crate::sample::vertex_at;
use crate::Rigid;

use super::icp_overlap::{
    common_support_coverage, reciprocal_evidence, reciprocal_evidence_is_usable,
};
use super::icp_step::{correspondences_at_radius, try_backtracked_step, TrialState};
use super::{
    influence_radius_ladder, Level, LevelOutcome, Orientation, Summary, COARSE_TIE_SEATED,
    CONVERGED_ROTATION, CONVERGED_TRANSLATION, DAMPING_GROWTH, HUBER_FACTOR, INITIAL_DAMPING,
    MAX_DAMPING_RETRIES, MIN_CORRESPONDENCES, SEATED_BAND_MM, STALL_IMPROVEMENT,
    WEAK_AXIS_FRACTION,
};

/// One accepted moving-vertex-to-fixed-surface correspondence.
#[derive(Clone, Copy)]
pub(super) struct Correspondence {
    pub(super) point: DVec3,
    pub(super) target: DVec3,
    pub(super) normal: DVec3,
    pub(super) residual: f64,
}

/// Run one resolution level to convergence or to its iteration ceiling.
#[expect(
    clippy::too_many_lines,
    reason = "one iteration loop whose branches are the documented stop and trial rules; \
              splitting it hides the frame in which `summary` and `pose` must stay paired"
)]
pub(super) fn run_level(level: &Level<'_>) -> Result<LevelOutcome, FitRejection> {
    let mut pose = level.start;
    let mut iterations = 0u32;
    let mut converged = false;
    let mut summary: Option<Summary> = None;
    let mut best_rms = f64::INFINITY;
    let mut best_seated = f64::NEG_INFINITY;
    // Keep the pose that seats most of the surface; the residual only breaks a
    // tie between two poses that seat the same amount.
    let mut best: Option<(Rigid, Summary)> = None;
    let radii = if level.settings.local_only {
        // The operator has already brought the scans together. Respect the
        // requested physical reach instead of looking 4x farther.
        vec![level.settings.influence_radius_mm]
    } else {
        influence_radius_ladder(level.settings.influence_radius_mm)
    };
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
        let measured_reciprocal = reciprocal_evidence(level, pose, radii[radius_slot]);
        if !reciprocal_evidence_is_usable(level, measured_reciprocal) {
            // Unusable reciprocal evidence means this iteration cannot be
            // trusted, not that the fit is worthless, so a pose the level has
            // already measured is kept. With nothing measured yet there is no
            // result to keep, so only that case is a refusal.
            if summary.is_none() && best.is_none() {
                return Err(FitRejection::NoImprovement);
            }
            break;
        }
        let mut measured = summarize(&found, &kept, matched, level.samples.len(), &normal_matrix);
        measured.support_coverage =
            common_support_coverage(level, measured.coverage, measured_reciprocal);
        summary = Some(measured);
        // The correspondences describe `pose` at the start of the iteration,
        // not the candidate the step below produces. Keep the summary paired.
        let seats_more = measured.seated_fraction > best_seated + COARSE_TIE_SEATED;
        let seats_same = (measured.seated_fraction - best_seated).abs() <= COARSE_TIE_SEATED;
        if seats_more
            || (seats_same
                && measured.geometric_rms.is_finite()
                && measured.geometric_rms < best_rms * STALL_IMPROVEMENT)
        {
            best_rms = measured.geometric_rms;
            best_seated = measured.seated_fraction;
            best = Some((pose, measured));
        }

        let Some(step) = solve_damped(&normal_matrix, &gradient) else {
            // A rank-deficient system is a stop, not a refusal. It says the
            // local model has no further direction to move, which is what a
            // seated pair looks like; the pose goes back through `best` below.
            // A real pair never reaches an exact zero residual, so refusing
            // here would discard the fit at the point it converged.
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
            // Apply a step only after evaluation shows an improvement: a
            // nearest-surface change could rotate a rough pair sideways while
            // its report still looked valid.
            //
            // Stopping here is not an error. The line search could not improve
            // on the level's current result, which defines a settled pose, so
            // `converged` describes this outcome. The rejected step's travel
            // does not describe the pose in `best`, and `converged` does not
            // residual: a residual threshold is a quality judgement and this
            // function only decides where to stop.
            converged = true;
            break;
        };
        pose = next_pose;
        // The accepted trial carries a fully re-evaluated summary. Keep it
        // paired with the pose so a convergence break cannot report metrics
        // for the pre-step correspondences.
        summary = Some(next_summary);
        let seats_more = next_summary.seated_fraction > best_seated + COARSE_TIE_SEATED;
        let seats_same = (next_summary.seated_fraction - best_seated).abs() <= COARSE_TIE_SEATED;
        if seats_more
            || (seats_same
                && next_summary.geometric_rms.is_finite()
                && next_summary.geometric_rms < best_rms)
        {
            best_rms = next_summary.geometric_rms;
            best_seated = next_summary.seated_fraction;
            best = Some((next_pose, next_summary));
        }
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
pub(super) fn correspondences(
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
            // A border hit on a physically smaller fixed scan is outside the
            // common surface the registration is meant to explain. In the
            // reverse crop role, the full moving arch projects many unrelated
            // vertices onto the crop's open cut edge; those hits overwhelm the
            // trimmed tail even when the interior crop is seated exactly.
            // Keep the established forward behavior when the fixed surface is
            // equal or larger, and make the solver use the same open-edge rule
            // as final verification for this fragment-as-target case.
            if hit.on_border && super::icp_overlap::fixed_surface_is_smaller(level) {
                return None;
            }
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
pub(super) fn trim(found: &[Option<Correspondence>], ratio: f64) -> Vec<Correspondence> {
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
pub(super) fn accumulate(kept: &[Correspondence]) -> ([[f64; 6]; 6], [f64; 6], DVec3) {
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
pub(super) fn summarize(
    found: &[Option<Correspondence>],
    kept: &[Correspondence],
    matched: usize,
    sampled: usize,
    matrix: &[[f64; 6]; 6],
) -> Summary {
    // How much of the surface is actually seated.
    //
    // Taken over every correspondence found at this pose, not over `kept`:
    // the trimmed set is chosen by distance, so a deformed majority always
    // fills it and pushes the rigid part of the same surface out — measuring
    // seating inside `kept` would report the deformation's own coherence and
    // call it a fit. Counted against the sampled population, so a pose that
    // explains only a sliver cannot look seated either.
    let seated = found
        .iter()
        .flatten()
        .filter(|entry| (entry.point - entry.target).length() <= SEATED_BAND_MM)
        .count();
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

    let (weak_rot_axes, weak_trans_axes) = weak_axes_from_normal_matrix(matrix);

    #[allow(clippy::cast_precision_loss)]
    let sampled_count = sampled.max(1) as f64;
    Summary {
        inliers: u32::try_from(kept.len()).unwrap_or(u32::MAX),
        inlier_ratio: count / sampled_count,
        support_coverage: 0.0,
        #[allow(clippy::cast_precision_loss)]
        coverage: matched as f64 / sampled_count,
        rms: (sum_squares / count).sqrt(),
        geometric_rms: (geometric_sum_squares / count).sqrt(),
        #[allow(clippy::cast_precision_loss)]
        seated_fraction: seated as f64 / sampled_count,
        median_abs: magnitudes.get(magnitudes.len() / 2).copied().unwrap_or(0.0),
        p95_abs: magnitudes
            .get(p95_slot.clamp(1, magnitudes.len()) - 1)
            .copied()
            .unwrap_or(0.0),
        weak_rot_axes,
        weak_trans_axes,
    }
}

/// Classify weak motion directions from the weighted normal matrix.
///
/// The rotational and translational Jacobian columns have different units, so
/// a raw eigendecomposition would make the answer depend on the mesh scale.
/// Normalize each column first to form a dimensionless correlation matrix. A
/// zero eigenvalue then means that some combination of motion columns is
/// unobservable, even when every individual diagonal is large. That is the
/// case a diagonal-only guard misses for repeated or locally symmetric
/// surfaces.
pub(super) fn weak_axes_from_normal_matrix(matrix: &[[f64; 6]; 6]) -> ([bool; 3], [bool; 3]) {
    let mut diagonal = [0.0; 6];
    for (index, value) in diagonal.iter_mut().enumerate() {
        *value = matrix[index][index];
    }
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        return ([true; 3], [true; 3]);
    }
    let mut weak = [false; 6];
    if !diagonal.iter().any(|value| *value > f64::MIN_POSITIVE) {
        return ([true; 3], [true; 3]);
    }

    // Normalize by each column's own energy. Comparing raw rotation and
    // translation diagonals would make rank depend on the mesh's unit scale;
    // a correlation matrix keeps only the geometry's angular relationships.
    let mut correlation = [[0.0_f64; 6]; 6];
    for row in 0..6 {
        for column in 0..6 {
            let row_scale = diagonal[row];
            let column_scale = diagonal[column];
            let denominator = (row_scale * column_scale).sqrt();
            correlation[row][column] = if denominator.is_finite() && denominator > 0.0 {
                matrix[row][column] / denominator
            } else {
                0.0
            };
        }
    }
    let (eigenvalues, eigenvectors) = symmetric_eigendecomposition(correlation);
    let mut found_weak_eigenvalue = false;
    for (eigen_index, &eigenvalue) in eigenvalues.iter().enumerate() {
        if !eigenvalue.is_finite() || eigenvalue <= WEAK_AXIS_FRACTION {
            found_weak_eigenvalue = true;
            for axis in 0..6 {
                if eigenvectors[axis][eigen_index].abs() > 1e-3 {
                    weak[axis] = true;
                }
            }
        }
    }
    if found_weak_eigenvalue && !weak.into_iter().any(|axis| axis) {
        // A malformed matrix must fail closed even if its eigenvectors did not
        // produce a usable component classification.
        weak = [true; 6];
    }

    ([weak[0], weak[1], weak[2]], [weak[3], weak[4], weak[5]])
}

/// Deterministic Jacobi eigendecomposition for a real symmetric 6x6 matrix.
/// The normal matrix is tiny and assembled in a fixed order, so a bounded
/// fixed-sweep solver is preferable to introducing a scale-sensitive external
/// linear-algebra dependency into the alignment kernel.
pub(super) fn symmetric_eigendecomposition(mut matrix: [[f64; 6]; 6]) -> ([f64; 6], [[f64; 6]; 6]) {
    let mut vectors = [[0.0_f64; 6]; 6];
    for (index, row) in vectors.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    for _ in 0..64 {
        let mut pivot = (0, 1);
        let mut largest = 0.0_f64;
        for (row_index, row) in matrix.iter().enumerate() {
            for (column, &value) in row.iter().enumerate().skip(row_index + 1) {
                let magnitude = value.abs();
                if magnitude > largest {
                    largest = magnitude;
                    pivot = (row_index, column);
                }
            }
        }
        if largest <= 1e-12 {
            break;
        }
        let (p, q) = pivot;
        let app = matrix[p][p];
        let aqq = matrix[q][q];
        let apq = matrix[p][q];
        if apq == 0.0 {
            continue;
        }
        let tau = (aqq - app) / (2.0 * apq);
        let sign = if tau >= 0.0 { 1.0 } else { -1.0 };
        let t = sign / (tau.abs() + (1.0 + tau * tau).sqrt());
        let cosine = 1.0 / (1.0 + t * t).sqrt();
        let sine = t * cosine;

        let mut rotated_p = [0.0_f64; 6];
        let mut rotated_q = [0.0_f64; 6];
        for (index, (row, (new_p, new_q))) in matrix
            .iter()
            .zip(rotated_p.iter_mut().zip(rotated_q.iter_mut()))
            .enumerate()
        {
            if index == p || index == q {
                continue;
            }
            let aip = row[p];
            let aiq = row[q];
            *new_p = cosine * aip - sine * aiq;
            *new_q = sine * aip + cosine * aiq;
        }
        for (index, (row, (&new_p, &new_q))) in matrix
            .iter_mut()
            .zip(rotated_p.iter().zip(rotated_q.iter()))
            .enumerate()
        {
            if index == p || index == q {
                continue;
            }
            row[p] = new_p;
            row[q] = new_q;
        }
        for (index, (slot, &value)) in matrix[p].iter_mut().zip(rotated_p.iter()).enumerate() {
            if index != p && index != q {
                *slot = value;
            }
        }
        for (index, (slot, &value)) in matrix[q].iter_mut().zip(rotated_q.iter()).enumerate() {
            if index != p && index != q {
                *slot = value;
            }
        }
        matrix[p][p] = cosine * cosine * app - 2.0 * sine * cosine * apq + sine * sine * aqq;
        matrix[q][q] = sine * sine * app + 2.0 * sine * cosine * apq + cosine * cosine * aqq;
        matrix[p][q] = 0.0;
        matrix[q][p] = 0.0;

        for row in &mut vectors {
            let vip = row[p];
            let viq = row[q];
            row[p] = cosine * vip - sine * viq;
            row[q] = sine * vip + cosine * viq;
        }
    }
    let eigenvalues = core::array::from_fn(|index| matrix[index][index]);
    (eigenvalues, vectors)
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
///
/// The pivot guard here is `sum <= f64::MIN_POSITIVE`, looser than
/// `observability::cholesky`'s `!sum.is_finite() || sum <= MIN_PIVOT`. That
/// difference is deliberate, not drift. This solver is only reached through
/// [`solve_damped`], which grows the damping until the factorisation succeeds and
/// checks the resulting step with `is_finite`, so a marginal pivot is a retry; a
/// NaN pivot yields a NaN step that `solve_damped` rejects rather than returns.
/// The whitening factorisation in `observability` has no damping to grow and no
/// retry, so it has to reject an untrusted pivot on the spot.
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
pub(super) fn apply_step(pose: Rigid, centre: DVec3, rotation: DVec3, translation: DVec3) -> Rigid {
    let delta_rotation = DQuat::from_scaled_axis(rotation);
    let basis = DMat3::from_quat(delta_rotation);
    let delta = Rigid::new(delta_rotation, centre - basis * centre + translation);
    delta.compose(&pose)
}
