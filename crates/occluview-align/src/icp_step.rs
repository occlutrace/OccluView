//! Bounded correspondence-radius and line-search helpers for ICP.

use glam::DVec3;

use crate::Rigid;

use super::icp_overlap::{
    common_support_coverage, directional_forward_evidence_is_sufficient,
    minimum_directional_forward_matches, reciprocal_evidence, reciprocal_evidence_is_usable,
    reciprocal_trial_is_acceptable, support_coverage_is_sufficient, ReciprocalSummary,
};
use super::icp_solve::{accumulate, apply_step, correspondences, summarize, trim, Correspondence};
use super::{
    FitRejection, Level, Summary, BACKTRACK_SCALES, MIN_CORRESPONDENCES,
    MIN_TRIAL_COVERAGE_FRACTION, STALL_IMPROVEMENT,
};

/// Immutable input captured for one bounded line-search pass.
#[derive(Clone, Copy)]
pub(super) struct TrialState {
    pub(super) pose: Rigid,
    pub(super) centre: DVec3,
    pub(super) rotation: DVec3,
    pub(super) translation: DVec3,
    pub(super) measured: Summary,
    pub(super) measured_reciprocal: Option<ReciprocalSummary>,
    pub(super) radius: f64,
}

/// Find correspondences at the narrowest radius that can determine a pose.
/// Widening is monotonic within a level, so a rough pose gets more reach while
/// a seated pose keeps the conservative local neighbourhood.
#[allow(clippy::cast_precision_loss)]
pub(super) fn correspondences_at_radius(
    level: &Level<'_>,
    pose: Rigid,
    radii: &[f64],
    radius_slot: &mut usize,
) -> Result<(Vec<Option<Correspondence>>, usize), FitRejection> {
    let mut found = correspondences(level, pose, radii[*radius_slot]);
    let mut matched = found.iter().flatten().count();
    let mut reciprocal = reciprocal_evidence(level, pose, radii[*radius_slot]);
    while *radius_slot + 1 < radii.len()
        && (!directional_forward_evidence_is_sufficient(level, matched, level.samples.len())
            || !reciprocal_evidence_is_usable(level, reciprocal)
            || !support_coverage_is_sufficient(common_support_coverage(
                level,
                matched as f64 / level.samples.len().max(1) as f64,
                reciprocal,
            )))
    {
        *radius_slot += 1;
        found = correspondences(level, pose, radii[*radius_slot]);
        matched = found.iter().flatten().count();
        reciprocal = reciprocal_evidence(level, pose, radii[*radius_slot]);
    }
    let forward_coverage = matched as f64 / level.samples.len().max(1) as f64;
    if !directional_forward_evidence_is_sufficient(level, matched, level.samples.len())
        || !reciprocal_evidence_is_usable(level, reciprocal)
        || !support_coverage_is_sufficient(common_support_coverage(
            level,
            forward_coverage,
            reciprocal,
        ))
    {
        return Err(FitRejection::TooFewPairs {
            have: matched,
            need: minimum_directional_forward_matches(level, level.samples.len()),
        });
    }
    Ok((found, matched))
}

/// Evaluate a solved rigid step at bounded fractions and accept only a trial
/// that improves both forward geometry and reciprocal overlap evidence.
pub(super) fn try_backtracked_step(
    level: &Level<'_>,
    state: TrialState,
) -> Option<(Rigid, Summary)> {
    if !reciprocal_evidence_is_usable(level, state.measured_reciprocal) {
        return None;
    }
    for fraction in BACKTRACK_SCALES {
        let trial_pose = apply_step(
            state.pose,
            state.centre,
            state.rotation * fraction,
            state.translation * fraction,
        );
        let trial_found = correspondences(level, trial_pose, state.radius);
        let trial_matched = trial_found.iter().flatten().count();
        if trial_matched < MIN_CORRESPONDENCES {
            continue;
        }
        let trial_reciprocal = reciprocal_evidence(level, trial_pose, state.radius);
        if !reciprocal_evidence_is_usable(level, trial_reciprocal) {
            continue;
        }
        let trial_kept = trim(&trial_found, level.settings.matching_ratio);
        if trial_kept.len() < MIN_CORRESPONDENCES {
            continue;
        }
        let (trial_matrix, _, _) = accumulate(&trial_kept);
        let mut trial_summary = summarize(
            &trial_found,
            &trial_kept,
            trial_matched,
            level.samples.len(),
            &trial_matrix,
        );
        trial_summary.support_coverage =
            common_support_coverage(level, trial_summary.coverage, trial_reciprocal);
        if !support_coverage_is_sufficient(trial_summary.support_coverage)
            || trial_summary.support_coverage + f64::EPSILON
                < state.measured.support_coverage * MIN_TRIAL_COVERAGE_FRACTION
        {
            continue;
        }
        if !reciprocal_trial_is_acceptable(level, state.measured_reciprocal, trial_reciprocal) {
            continue;
        }
        // A step may not trade seating away for a smaller residual: a trimmed
        // least-squares residual falls when a step spreads a deformation over
        // everything. This is a monotonicity rule, not the acceptance test;
        // `is_trustworthy_refinement_for` judges the pose on its median.
        let seated_kept = trial_summary.seated_fraction + 1e-9 >= state.measured.seated_fraction;
        if seated_kept
            && trial_summary.geometric_rms.is_finite()
            && trial_summary.geometric_rms < state.measured.geometric_rms * STALL_IMPROVEMENT
        {
            return Some((trial_pose, trial_summary));
        }
    }
    None
}
