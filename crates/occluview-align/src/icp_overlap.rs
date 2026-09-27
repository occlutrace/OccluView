//! Bounded reciprocal overlap evidence for ICP trial validation.
//!
//! The forward ICP statistics remain the public measurement. This module only
//! checks that a proposed pose does not discard the fixed surface that helped
//! justify the current pose.

use rayon::prelude::*;

use crate::Rigid;

use super::{Level, Orientation, MIN_TRIAL_COVERAGE_FRACTION};

/// A reciprocal check based on only a handful of fixed samples can validate a
/// coincidental patch. Keep partial scans valid, but require enough absolute
/// evidence to make the bidirectional guard meaningful.
const MIN_RECIPROCAL_MATCHES: usize = 12;

/// A fixed surface with thousands of representatives must contribute more
/// than twelve lucky hits before it can authorize a trial. This is a small
/// fraction: partial scans remain valid, while a one-percent sliver
/// cannot win a global search merely because its absolute hit count cleared the
/// floor above.
const MIN_RECIPROCAL_COVERAGE_FRACTION: f64 = 0.01;

#[cfg(test)]
mod tests {
    use super::{reciprocal_coverage_ok, reciprocal_summary_is_usable, ReciprocalSummary};

    #[test]
    fn reciprocal_guard_does_not_validate_a_six_point_patch() {
        let tiny_patch = ReciprocalSummary {
            matched: 6,
            coverage: 0.003,
            geometric_rms: 0.001,
        };

        assert!(
            !reciprocal_coverage_ok(Some(tiny_patch), Some(tiny_patch)),
            "a local six-point patch is not enough bidirectional evidence"
        );
    }

    #[test]
    fn reciprocal_guard_needs_a_fixed_surface_fraction() {
        let sparse_patch = ReciprocalSummary {
            matched: 12,
            coverage: 0.009,
            geometric_rms: 0.001,
        };
        let useful_patch = ReciprocalSummary {
            coverage: 0.01,
            ..sparse_patch
        };

        assert!(!reciprocal_summary_is_usable(sparse_patch));
        assert!(reciprocal_summary_is_usable(useful_patch));
        assert!(
            !reciprocal_coverage_ok(Some(sparse_patch), Some(sparse_patch)),
            "an absolute hit floor must not bypass the coverage floor"
        );
        assert!(reciprocal_coverage_ok(
            Some(useful_patch),
            Some(useful_patch)
        ));
    }

    #[test]
    fn reciprocal_guard_rejects_non_finite_summary_values() {
        assert!(!reciprocal_summary_is_usable(ReciprocalSummary {
            matched: 20,
            coverage: f64::NAN,
            geometric_rms: 0.001,
        }));
        assert!(!reciprocal_summary_is_usable(ReciprocalSummary {
            matched: 20,
            coverage: 0.2,
            geometric_rms: f64::INFINITY,
        }));
    }
}

/// Fixed-to-moving evidence evaluated at one pose.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReciprocalSummary {
    pub(super) matched: usize,
    pub(super) coverage: f64,
    pub(super) geometric_rms: f64,
}

fn reciprocal_summary_is_usable(summary: ReciprocalSummary) -> bool {
    summary.matched >= MIN_RECIPROCAL_MATCHES
        && summary.coverage.is_finite()
        && (MIN_RECIPROCAL_COVERAGE_FRACTION..=1.0).contains(&summary.coverage)
        && summary.geometric_rms.is_finite()
        && summary.geometric_rms >= 0.0
}

/// Whether reciprocal evidence is available and large enough to authorize a
/// trial. Point-cloud moving layers have no reverse surface to query, so they
/// retain the forward-only path; triangle meshes must prove both directions.
pub(super) fn reciprocal_evidence_is_usable(
    level: &Level<'_>,
    evidence: Option<ReciprocalSummary>,
) -> bool {
    if level.moving_surface.is_none() || level.fixed_samples.is_empty() {
        return true;
    }
    evidence.is_some_and(reciprocal_summary_is_usable)
}

/// Do not let a trial discard the fixed surface that supported the current
/// pose. A missing reciprocal sample is not itself a failure when the current
/// pose had no reciprocal evidence (common for partial scans), but once a pose
/// has evidence, a trial must retain most of it.
pub(super) fn reciprocal_coverage_ok(
    current: Option<ReciprocalSummary>,
    trial: Option<ReciprocalSummary>,
) -> bool {
    match (current, trial) {
        (Some(current), Some(trial)) => {
            reciprocal_summary_is_usable(current)
                && reciprocal_summary_is_usable(trial)
                && trial.coverage + f64::EPSILON >= current.coverage * MIN_TRIAL_COVERAGE_FRACTION
                && trial.geometric_rms <= current.geometric_rms * 1.25 + 1e-9
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Measure a bounded fixed-to-moving overlap signal at `pose`.
///
/// This is intentionally not folded into the displayed ICP statistics. It is
/// a consistency guard only: the public report remains the forward moving to
/// fixed measurement, while this reciprocal direction prevents a trial from
/// keeping a tiny accidental patch after losing the fixed area it was meant to
/// explain. Partial scans remain valid because there is no absolute coverage
/// threshold — only a relative loss from the current pose is refused.
pub(super) fn reciprocal_evidence(
    level: &Level<'_>,
    pose: Rigid,
    influence_radius_mm: f64,
) -> Option<ReciprocalSummary> {
    let moving_surface = level.moving_surface?;
    if level.fixed_samples.is_empty() {
        return None;
    }
    let inverse = pose.inverse();
    let distances: Vec<Option<f64>> = level
        .fixed_samples
        .par_iter()
        .map(|sample| {
            let local = inverse.apply(sample.point);
            let hit = moving_surface.nearest(local, influence_radius_mm)?;
            let transformed_normal = pose.apply_normal(hit.normal);
            let agreement = sample.normal.dot(transformed_normal);
            let accepted = match level.settings.orientation {
                Orientation::Match => agreement > 0.0,
                Orientation::Inverted => agreement < 0.0,
                Orientation::Ignored => true,
            };
            accepted.then(|| (pose.apply(hit.point) - sample.point).length())
        })
        .collect();
    let matched = distances.iter().flatten().count();
    if matched == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = matched as f64;
    let sum_squares = distances
        .iter()
        .flatten()
        .map(|distance| distance * distance)
        .sum::<f64>();
    #[allow(clippy::cast_precision_loss)]
    let sample_count = level.fixed_samples.len() as f64;
    Some(ReciprocalSummary {
        matched,
        coverage: count / sample_count,
        geometric_rms: (sum_squares / count).sqrt(),
    })
}
