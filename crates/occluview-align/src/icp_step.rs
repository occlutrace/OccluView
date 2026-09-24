//! Bounded correspondence-radius and line-search helpers for ICP.

use glam::DVec3;

use crate::Rigid;

use super::icp_overlap::{
    reciprocal_coverage_ok, reciprocal_evidence, reciprocal_evidence_is_usable, ReciprocalSummary,
};
use super::{
    accumulate, apply_step, correspondences, forward_coverage_is_sufficient,
    minimum_forward_matches, summarize, trim, Correspondence, FitRejection, Level, Summary,
    BACKTRACK_SCALES, MIN_CORRESPONDENCES, MIN_TRIAL_COVERAGE_FRACTION, STALL_IMPROVEMENT,
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
pub(super) fn correspondences_at_radius(
    level: &Level<'_>,
    pose: Rigid,
    radii: &[f64],
    radius_slot: &mut usize,
) -> Result<(Vec<Option<Correspondence>>, usize), FitRejection> {
    let mut found = correspondences(level, pose, radii[*radius_slot]);
    let mut matched = found.iter().flatten().count();
    while !forward_coverage_is_sufficient(matched, level.samples.len())
        && *radius_slot + 1 < radii.len()
    {
        *radius_slot += 1;
        found = correspondences(level, pose, radii[*radius_slot]);
        matched = found.iter().flatten().count();
    }
    if !forward_coverage_is_sufficient(matched, level.samples.len()) {
        return Err(FitRejection::TooFewPairs {
            have: matched,
            need: minimum_forward_matches(level.samples.len()),
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
        let trial_kept = trim(&trial_found, level.settings.matching_ratio);
        if trial_kept.len() < MIN_CORRESPONDENCES {
            continue;
        }
        let (trial_matrix, _, _) = accumulate(&trial_kept);
        let trial_summary = summarize(
            &trial_found,
            &trial_kept,
            trial_matched,
            level.samples.len(),
            &trial_matrix,
        );
        if trial_summary.coverage + f64::EPSILON
            < state.measured.coverage * MIN_TRIAL_COVERAGE_FRACTION
        {
            continue;
        }
        let trial_reciprocal = reciprocal_evidence(level, trial_pose, state.radius);
        if !reciprocal_evidence_is_usable(level, trial_reciprocal) {
            continue;
        }
        if !reciprocal_coverage_ok(state.measured_reciprocal, trial_reciprocal) {
            continue;
        }
        // A step may not trade seating away for a smaller residual. A trimmed
        // least-squares residual always falls when the step spreads a
        // deformation over everything, which is how a prepared model ends up
        // seated on its operated region instead of its unchanged one. This is a
        // monotonicity rule, not the acceptance test: a level may start and stay
        // unseated, and the pose it converges to is still judged by
        // `is_trustworthy_refinement_for`, which refuses anything below
        // `MIN_SEATED_FRACTION`. Nothing here may lower seating, and nothing
        // here may authorize a pose on its own.
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
