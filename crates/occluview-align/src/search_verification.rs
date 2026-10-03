//! Transactional independent evidence at the registration publication boundary.
//! Ranking uses the fractional common-set objective of Phillips, Liu and
//! Tomasi (2006), <https://arxiv.org/abs/cs/0606098>, with the declared physical
//! floor. Training scores never compete numerically with holdout scores.

use crate::{
    AlignmentCandidate, AlignmentSearchResult, CandidateEvidence, CandidateId, Confidence,
    EvidenceReason, Metric, MissingReason, PreparedSurface, RefinementTermination, SearchSettings,
    SeedOrigin,
};
use occluview_geometry::surface::{GeometryControl, GeometryStop};
use std::collections::{BTreeMap, BTreeSet};

/// Preserve the best numerical checkpoint on interruption and classify only
/// completed independent passes. Public top-k never reduces the rival pool.
#[expect(
    clippy::too_many_lines,
    reason = "bounded transactional verification and candidate publication"
)]
pub(crate) fn verify_results(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    settings: &SearchSettings,
    result: &mut AlignmentSearchResult,
    control: &GeometryControl,
) {
    let _phase = crate::search_probe::Span::new(crate::search_probe::Phase::Verification, control);
    let _workspace = match control.reserve(2 * 1024 * 1024) {
        Ok(workspace) => workspace,
        Err(reason) => {
            result.completion = crate::sample::completion(reason);
            result
                .work
                .unfinished_stages
                .push("independent-verification");
            for candidate in &mut result.candidates {
                candidate.reasons.push(EvidenceReason::BudgetExhausted);
            }
            result.candidates.truncate(settings.top_k);
            return;
        }
    };
    let mut candidates = std::mem::take(&mut result.candidates);
    let mut fully_evaluated = BTreeSet::new();
    let mut metadata = BTreeMap::new();
    let mut stop = None;
    // Five full basins and eight cheaper reserved rivals share one physical
    // score. Screened rivals cannot establish the final full-resolution gap.
    for (index, candidate) in candidates.iter_mut().take(13).enumerate() {
        let full = index < 5;
        match measure_candidate(moving, fixed, candidate, settings, full, control) {
            Ok(Some(info)) => {
                metadata.insert(candidate.id, info);
                if full {
                    fully_evaluated.insert(candidate.id);
                }
            }
            Ok(None) => {}
            Err(reason) => {
                stop = Some(reason);
                break;
            }
        }
    }
    let leading_score = candidates
        .iter()
        .filter(|c| fully_evaluated.contains(&c.id))
        .filter_map(|c| measured(c.evidence.score))
        .max_by(f64::total_cmp);
    // A close or unavailable screening score cannot exclude a rival. Promote
    // it before using its score in full-resolution uniqueness evidence.
    if stop.is_none() {
        for candidate in candidates.iter_mut().take(13) {
            if fully_evaluated.contains(&candidate.id) {
                continue;
            }
            let remote = leading_score
                .zip(measured(candidate.evidence.score))
                .is_some_and(|(best, rival)| best - rival > 0.15);
            if remote {
                continue;
            }
            match measure_candidate(moving, fixed, candidate, settings, true, control) {
                Ok(Some(info)) => {
                    metadata.insert(candidate.id, info);
                    fully_evaluated.insert(candidate.id);
                }
                Ok(None) => {}
                Err(reason) => {
                    stop = Some(reason);
                    break;
                }
            }
        }
    }
    // Ranking can bring a previously screened rival into the review slots.
    // Fully evaluate every such entrant, with at most eight promotions.
    for _ in 0..8 {
        if stop.is_some() {
            break;
        }
        candidates.sort_by(verified_order);
        let pending = candidates
            .iter()
            .take(5)
            .position(|c| !fully_evaluated.contains(&c.id));
        let Some(index) = pending else {
            break;
        };
        let candidate = &mut candidates[index];
        match measure_candidate(moving, fixed, candidate, settings, true, control) {
            Ok(Some(info)) => {
                metadata.insert(candidate.id, info);
                fully_evaluated.insert(candidate.id);
            }
            Ok(None) => break,
            Err(reason) => {
                stop = Some(reason);
                break;
            }
        }
    }
    if stop.is_none() {
        candidates.sort_by(verified_order);
        for candidate in candidates.iter_mut().take(5) {
            let Some(&(pose, center, radius)) = metadata.get(&candidate.id) else {
                continue;
            };
            if radius <= 0. {
                continue;
            }
            match crate::icp::spatial_jackknife(
                moving,
                fixed,
                pose,
                settings.normal_policy,
                center,
                control,
            ) {
                Ok(drift) => {
                    candidate.evidence.jackknife_mm_deg = drift;
                    candidate.evidence.jackknife_complete = matches!(drift, Metric::Measured(_));
                }
                Err(reason) => {
                    stop = Some(reason);
                    break;
                }
            }
        }
    }
    if stop.is_none() {
        if let Some(leading) = candidates.first() {
            if let Some(pose) = moving.frame.correction_to_query(fixed.frame, leading.pose) {
                match crate::icp::probe_rivals(moving, pose, &leading.evidence.weak_twists, control)
                {
                    Ok(probes) => {
                        for (ordinal, probe) in probes.into_iter().take(8).enumerate() {
                            match evaluate_probe(moving, fixed, probe, settings, control) {
                                Ok(Some(mut candidate)) => {
                                    candidate.id = CandidateId {
                                        family: 8,
                                        proposal: u32::try_from(ordinal).unwrap_or(0),
                                    };
                                    fully_evaluated.insert(candidate.id);
                                    candidates.push(candidate);
                                }
                                Ok(None) => {}
                                Err(reason) => {
                                    stop = Some(reason);
                                    break;
                                }
                            }
                        }
                    }
                    Err(reason) => stop = Some(reason),
                }
            }
        }
    }
    if let Some(reason) = stop {
        result.completion = crate::sample::completion(reason);
        if !result
            .work
            .unfinished_stages
            .contains(&"independent-verification")
        {
            result
                .work
                .unfinished_stages
                .push("independent-verification");
        }
    } else if candidates
        .iter()
        .take(5)
        .all(|c| fully_evaluated.contains(&c.id))
    {
        result
            .work
            .unfinished_stages
            .retain(|&stage| stage != "independent-verification");
    }
    // The arch-local tooth-pitch autocorrelation set and conservative whole-area
    // uncertainty are not established by principal/global probes. Keep these
    // obligations visible and prohibit highest-class publication.
    result.work.unfinished_stages.push("tooth-pitch-rivals");
    result
        .work
        .unfinished_stages
        .push("whole-area-support-bound");
    for index in 0..candidates.len() {
        if !candidates[index].evidence.holdout_complete {
            continue;
        }
        let Some(base) = moving
            .frame
            .correction_to_query(fixed.frame, candidates[index].pose)
        else {
            continue;
        };
        let mut scores = Vec::new();
        for (other_index, other) in candidates.iter().enumerate() {
            if index == other_index
                || !other.evidence.holdout_complete
                || !fully_evaluated.contains(&other.id)
            {
                continue;
            }
            let Some(rival) = moving.frame.correction_to_query(fixed.frame, other.pose) else {
                continue;
            };
            if let Err(reason) = control.charge_point_pairs(256) {
                stop = Some(reason);
                break;
            }
            let distance =
                crate::candidate_score::pose_distance(base, rival, &moving.samples[0].samples);
            if distance[0] >= 0.5 || distance[1] >= 2f64.to_radians() {
                scores.push(other.evidence.score);
            }
        }
        candidates[index].evidence.rival_gap =
            crate::icp::rival_gap(candidates[index].evidence.score, scores);
    }
    if let Some(reason) = stop {
        result.completion = crate::sample::completion(reason);
    }
    let completion = result.completion;
    for candidate in &mut candidates {
        if !candidate.evidence.holdout_complete {
            // Preserve the numerical pose, never its training coverage as final
            // independent evidence. Retain separate training information.
            let training = candidate.evidence.training_info_eigenvalues;
            let twists = candidate.evidence.training_weak_twists.clone();
            candidate.evidence = CandidateEvidence {
                training_info_eigenvalues: training,
                training_weak_twists: twists,
                euclidean_mm: Metric::Missing(MissingReason::Interrupted),
                ..CandidateEvidence::default()
            };
            candidate.reasons.push(EvidenceReason::BudgetExhausted);
        }
        candidate.reasons.retain(|r| {
            !matches!(
                r,
                EvidenceReason::PolicyConflict
                    | EvidenceReason::UnobservableMotion
                    | EvidenceReason::InsufficientSupport
            )
        });
        if candidate.evidence.orientation_fraction.is_below(0.75) {
            candidate.reasons.push(EvidenceReason::PolicyConflict);
        }
        crate::confidence::classify_candidate(
            candidate,
            completion,
            result.input_check,
            settings.profile,
        );
    }
    // Missing passes keep their original numerical order. Measured scores share
    // one definition and never compare against proposal scores.
    candidates.sort_by(verified_order);
    candidates.truncate(settings.top_k);
    result.candidates = candidates;
    result.work.retained_poses = u32::try_from(result.candidates.len()).unwrap_or(5);
}

#[expect(
    clippy::too_many_arguments,
    reason = "immutable surfaces and one explicit verification resolution"
)]
fn measure_candidate(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    candidate: &mut AlignmentCandidate,
    settings: &SearchSettings,
    full: bool,
    control: &GeometryControl,
) -> Result<Option<(crate::Rigid, glam::DVec3, f64)>, GeometryStop> {
    let Some(pose) = moving
        .frame
        .correction_to_query(fixed.frame, candidate.pose)
    else {
        return Ok(None);
    };
    let mut pass = if full {
        crate::icp::verify_candidate(moving, fixed, pose, settings.normal_policy, control)?
    } else {
        crate::icp::verify_rival_candidate(moving, fixed, pose, settings.normal_policy, control)?
    };
    pass.evidence.training_info_eigenvalues = candidate.evidence.training_info_eigenvalues;
    pass.evidence
        .training_weak_twists
        .clone_from(&candidate.evidence.training_weak_twists);
    candidate.evidence = pass.evidence;
    Ok(Some((pose, pass.center, pass.radius)))
}

fn evaluate_probe(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    probe: crate::Rigid,
    settings: &SearchSettings,
    control: &GeometryControl,
) -> Result<Option<AlignmentCandidate>, GeometryStop> {
    let refined = crate::icp::perturb_refine(
        moving,
        fixed,
        probe,
        settings.normal_policy,
        None,
        5,
        control,
    )?
    .unwrap_or(probe);
    let pass =
        crate::icp::verify_candidate(moving, fixed, refined, settings.normal_policy, control)?;
    let Some(pose) = moving.frame.correction_to_world(fixed.frame, refined) else {
        return Ok(None);
    };
    Ok(Some(AlignmentCandidate {
        id: CandidateId {
            family: 8,
            proposal: 0,
        },
        pose,
        confidence: Confidence::Weak,
        evidence: pass.evidence,
        reasons: vec![EvidenceReason::UniquenessNotEstablished],
        seeds: vec![SeedOrigin::PrincipalFrame],
        refinement: RefinementTermination::NotStarted,
    }))
}

fn measured(metric: Metric<f64>) -> Option<f64> {
    match metric {
        Metric::Measured(v) if v.is_finite() => Some(v),
        _ => None,
    }
}
fn verified_order(a: &AlignmentCandidate, b: &AlignmentCandidate) -> std::cmp::Ordering {
    match (measured(a.evidence.score), measured(b.evidence.score)) {
        (Some(a_score), Some(b_score)) => b_score
            .total_cmp(&a_score)
            .then_with(|| {
                measured(b.evidence.common_area_mm2)
                    .unwrap_or(0.)
                    .total_cmp(&measured(a.evidence.common_area_mm2).unwrap_or(0.))
            })
            .then(a.id.cmp(&b.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

trait Below {
    fn is_below(&self, limit: f64) -> bool;
}
impl Below for Metric<f64> {
    fn is_below(&self, limit: f64) -> bool {
        matches!(self, Metric::Measured(v) if *v < limit)
    }
}
