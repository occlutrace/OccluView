//! Conservative geometric classes using independent support and information.
//!
//! Gelfand et al. (2003),
//! <https://pixl.cs.princeton.edu/pubs/Gelfand_2003_GSS/stabicp.pdf>, motivates
//! exposing weak motion directions. The physical cutoffs and precedence here
//! are engineering criteria; they are neither probabilities nor clinical
//! certification. Missing or interrupted evidence cannot satisfy a cutoff.

use crate::{
    AlignmentCandidate, Completion, Confidence, EvidenceReason, InputCheck, Metric, MissingReason,
    ResidualSummary, SearchProfile,
};

fn number(metric: Metric<f64>) -> Option<f64> {
    match metric {
        Metric::Measured(v) if v.is_finite() && v >= 0. => Some(v),
        _ => None,
    }
}
fn residual(metric: Metric<ResidualSummary>) -> Option<ResidualSummary> {
    match metric {
        Metric::Measured(v)
            if [v.median, v.rms, v.p95]
                .into_iter()
                .all(|x| x.is_finite() && x >= 0.)
                && v.p95 >= v.median =>
        {
            Some(v)
        }
        _ => None,
    }
}
fn reason(candidate: &mut AlignmentCandidate, reason: EvidenceReason) {
    if !candidate.reasons.contains(&reason) {
        candidate.reasons.push(reason);
    }
}

/// Size and tightness of the common region.
pub(crate) struct Support {
    overlap: f64,
    area: f64,
    cells: u32,
    residual: ResidualSummary,
}

impl Support {
    /// Absent while any of its measurements is missing.
    pub(crate) fn of(e: &crate::CandidateEvidence) -> Option<Self> {
        let Metric::Measured(cells) = e.effective_cells else {
            return None;
        };
        Some(Self {
            overlap: number(e.overlap_smaller).filter(|v| *v <= 1.)?,
            area: number(e.common_area_mm2)?,
            cells,
            residual: residual(e.euclidean_mm)?,
        })
    }

    /// Enough tight common surface for the pose to be more than Weak.
    pub(crate) fn sufficient(&self) -> bool {
        self.overlap >= 0.20
            && self.area >= 25.
            && self.cells >= 64
            && self.residual.median <= 0.15
            && self.residual.rms <= 0.25
            && self.residual.p95 <= 0.50
    }
}

/// Classify independent evidence with Weak taking precedence over unsupported
/// symmetry claims. Acceptance and optimization termination are irrelevant.
/// Interrupted verification is Weak; incomplete global work is never Verified.
#[expect(
    clippy::too_many_lines,
    reason = "explicit ordered confidence clauses with distinct missing-evidence semantics"
)]
pub(crate) fn classify_candidate(
    candidate: &mut AlignmentCandidate,
    completion: Completion,
    input: InputCheck,
    profile: SearchProfile,
) {
    candidate.confidence = Confidence::Weak;
    if input != InputCheck::Complete {
        reason(candidate, EvidenceReason::UnvalidatedInput);
        return;
    }
    if candidate
        .reasons
        .contains(&EvidenceReason::SuspectedUnitsOrDeformation)
        || candidate.reasons.contains(&EvidenceReason::PolicyConflict)
    {
        return;
    }
    let e = &candidate.evidence;
    let Some(support) = Support::of(e).filter(Support::sufficient) else {
        reason(candidate, EvidenceReason::InsufficientSupport);
        return;
    };
    if !e.holdout_complete {
        return;
    }
    let Support {
        overlap,
        area,
        cells,
        residual: r,
    } = support;
    let lambda = match e.info_eigenvalues {
        Metric::Measured(values)
            if values.into_iter().all(|v| v.is_finite() && v >= 0.)
                && values.windows(2).all(|pair| pair[0] <= pair[1]) =>
        {
            Some(values[0])
        }
        Metric::Missing(MissingReason::Degenerate) => None,
        _ => return,
    };
    if lambda.is_none_or(|v| v < 0.0001) {
        candidate.confidence = Confidence::Ambiguous;
        reason(candidate, EvidenceReason::UnobservableMotion);
        return;
    }
    let gap = number(e.rival_gap).filter(|v| *v <= 1.);
    if gap.is_some_and(|v| v < 0.15) {
        candidate.confidence = Confidence::Ambiguous;
        reason(candidate, EvidenceReason::CloseRival);
        return;
    }
    let drift = match e.jackknife_mm_deg {
        Metric::Measured(d) if d.into_iter().all(|v| v.is_finite() && v >= 0.) => d,
        _ => return,
    };
    if drift[0] > 0.30 || drift[1] > 1. {
        candidate.confidence = Confidence::Ambiguous;
        reason(candidate, EvidenceReason::UnobservableMotion);
        return;
    }
    let Some(inlier) = number(e.inlier_ratio).filter(|v| *v <= 1.) else {
        return;
    };
    let Some(orientation) = number(e.orientation_fraction).filter(|v| *v <= 1.) else {
        reason(candidate, EvidenceReason::MissingNormals);
        return;
    };
    let Some(reciprocal) = number(e.reciprocal_fraction).filter(|v| *v <= 1.) else {
        return;
    };
    let Some(holdout) = number(e.holdout_ratio) else {
        return;
    };
    // A scan whose own triangles face both ways says nothing about how the
    // two surfaces face each other.
    let facing_known = !candidate.reasons.contains(&EvidenceReason::MissingNormals);
    if orientation < 0.75 && facing_known {
        reason(candidate, EvidenceReason::PolicyConflict);
        return;
    }
    if inlier < 0.70 || reciprocal < 0.70 || holdout > 1.75 || !e.jackknife_complete {
        return;
    }
    let verified = completion == Completion::Complete
        && profile != SearchProfile::Local
        && e.verification_complete
        && e.rival_probes_complete
        && overlap >= 0.60
        && area >= 100.
        && cells >= 128
        && r.median <= 0.05
        && r.rms <= 0.08
        && r.p95 <= 0.15
        && inlier >= 0.90
        && lambda.is_some_and(|v| v >= 0.002)
        && orientation >= 0.90
        && reciprocal >= 0.90
        && gap.is_some_and(|v| v >= 0.15)
        && holdout <= 1.25
        && drift[0] <= 0.10
        && drift[1] <= 0.25;
    candidate.confidence = if verified {
        Confidence::Verified
    } else {
        Confidence::Probable
    };
    if !verified && (gap.is_none() || !e.rival_probes_complete || profile == SearchProfile::Local) {
        reason(candidate, EvidenceReason::UniquenessNotEstablished);
    }
    if completion != Completion::Complete {
        reason(candidate, EvidenceReason::BudgetExhausted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateEvidence, CandidateId, RefinementTermination, Rigid, SeedOrigin};

    // Clause-isolation fixture only; the geometry behind the evidence is
    // exercised by the synthetic registration integration tests.
    fn clause_fixture() -> AlignmentCandidate {
        AlignmentCandidate {
            id: CandidateId {
                family: 0,
                proposal: 0,
            },
            pose: Rigid::IDENTITY,
            confidence: Confidence::Weak,
            reasons: vec![],
            seeds: vec![SeedOrigin::Start],
            refinement: RefinementTermination::StepSmall,
            evidence: CandidateEvidence {
                overlap_smaller: Metric::Measured(0.8),
                common_area_mm2: Metric::Measured(200.),
                effective_cells: Metric::Measured(200),
                euclidean_mm: Metric::Measured(ResidualSummary {
                    median: 0.01,
                    rms: 0.02,
                    p95: 0.04,
                }),
                info_eigenvalues: Metric::Measured([0.004, 0.01, 0.05, 0.2, 0.3, 0.4]),
                inlier_ratio: Metric::Measured(0.95),
                orientation_fraction: Metric::Measured(1.),
                reciprocal_fraction: Metric::Measured(0.95),
                rival_gap: Metric::Measured(0.2),
                holdout_ratio: Metric::Measured(1.),
                jackknife_mm_deg: Metric::Measured([0.01, 0.05]),
                verification_complete: true,
                holdout_complete: true,
                jackknife_complete: true,
                rival_probes_complete: true,
                original_surface_exact: [true; 2],
                ..CandidateEvidence::default()
            },
        }
    }
    fn classify(candidate: &mut AlignmentCandidate) {
        classify_candidate(
            candidate,
            Completion::Complete,
            InputCheck::Complete,
            SearchProfile::Standard,
        );
    }

    #[test]
    fn confidence_cutoffs_and_precedence_are_fixed() {
        let base = clause_fixture();
        let mut good = base.clone();
        classify(&mut good);
        assert_eq!(good.confidence, Confidence::Verified);
        let weak: [fn(&mut CandidateEvidence); 10] = [
            |e| e.overlap_smaller = Metric::Measured(0.19),
            |e| e.common_area_mm2 = Metric::Measured(24.),
            |e| e.effective_cells = Metric::Measured(63),
            |e| {
                e.euclidean_mm = Metric::Measured(ResidualSummary {
                    median: 0.16,
                    rms: 0.20,
                    p95: 0.3,
                });
            },
            |e| e.inlier_ratio = Metric::Measured(0.69),
            |e| e.orientation_fraction = Metric::Measured(0.74),
            |e| e.reciprocal_fraction = Metric::Measured(0.69),
            |e| e.holdout_ratio = Metric::Measured(1.76),
            |e| e.holdout_complete = false,
            |e| e.jackknife_complete = false,
        ];
        for change in weak {
            let mut c = base.clone();
            change(&mut c.evidence);
            classify(&mut c);
            assert_eq!(c.confidence, Confidence::Weak);
        }
        for change in [
            |e: &mut CandidateEvidence| e.info_eigenvalues = Metric::Measured([0.; 6]),
            |e: &mut CandidateEvidence| e.rival_gap = Metric::Measured(0.149),
            |e: &mut CandidateEvidence| e.jackknife_mm_deg = Metric::Measured([0.31, 0.]),
        ] {
            let mut c = base.clone();
            change(&mut c.evidence);
            classify(&mut c);
            assert_eq!(c.confidence, Confidence::Ambiguous);
            c.evidence.overlap_smaller = Metric::Measured(0.01);
            classify(&mut c);
            assert_eq!(c.confidence, Confidence::Weak);
        }
    }

    #[test]
    fn confidence_missing_and_incomplete_evidence_never_certifies() {
        for completion in [
            Completion::Deadline,
            Completion::Cancelled,
            Completion::ResourceLimit,
        ] {
            let mut c = clause_fixture();
            classify_candidate(
                &mut c,
                completion,
                InputCheck::Complete,
                SearchProfile::Standard,
            );
            assert_eq!(c.confidence, Confidence::Probable);
        }
        let mut c = clause_fixture();
        classify_candidate(
            &mut c,
            Completion::Complete,
            InputCheck::Complete,
            SearchProfile::Local,
        );
        assert_eq!(c.confidence, Confidence::Probable);
        c.evidence.rival_probes_complete = false;
        classify(&mut c);
        assert_eq!(c.confidence, Confidence::Probable);
        c.evidence.rival_gap = Metric::Missing(MissingReason::NoSupport);
        classify(&mut c);
        assert_eq!(c.confidence, Confidence::Probable);
        c.evidence.info_eigenvalues = Metric::Missing(MissingReason::NotEvaluated);
        classify(&mut c);
        assert_eq!(c.confidence, Confidence::Weak);
        c = clause_fixture();
        c.evidence.euclidean_mm = Metric::Measured(ResidualSummary {
            median: f64::NAN,
            rms: 0.,
            p95: 0.,
        });
        classify(&mut c);
        assert_eq!(c.confidence, Confidence::Weak);
        c = clause_fixture();
        c.reasons.push(EvidenceReason::SuspectedUnitsOrDeformation);
        classify(&mut c);
        assert_eq!(c.confidence, Confidence::Weak);
        c = clause_fixture();
        classify_candidate(
            &mut c,
            Completion::Complete,
            InputCheck::Partial {
                checked: 0,
                total: 1,
            },
            SearchProfile::Standard,
        );
        assert_eq!(c.confidence, Confidence::Weak);
    }
}
