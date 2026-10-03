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

struct CachedMeasurement {
    id: CandidateId,
    pose: crate::Rigid,
    full: bool,
    evidence: CandidateEvidence,
    center: glam::DVec3,
    radius: f64,
    jackknife: Option<Metric<[f64; 2]>>,
}

/// Completed evidence belongs to these immutable prepared populations and
/// normal policy. Pose changes require fresh measurement; screening evidence
/// never substitutes for full verification. Borrowing prevents surface edits.
pub(crate) struct VerificationCache<'a> {
    surfaces: [&'a PreparedSurface; 2],
    policy: crate::NormalPolicy,
    measurements: Vec<CachedMeasurement>,
    probes: Vec<(crate::Rigid, AlignmentCandidate)>,
    _memory: occluview_geometry::surface::GeometryMemory,
}

impl<'a> VerificationCache<'a> {
    /// Admit a bounded job-private cache before storing completed evidence.
    ///
    /// # Errors
    /// Returns cancellation, deadline or resource exhaustion on admission.
    pub(crate) fn new(
        moving: &'a PreparedSurface,
        fixed: &'a PreparedSurface,
        policy: crate::NormalPolicy,
        control: &GeometryControl,
    ) -> Result<Self, GeometryStop> {
        let memory = control.reserve(512 * 1024)?;
        let mut measurements = Vec::new();
        measurements
            .try_reserve_exact(32)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        let mut probes = Vec::new();
        probes
            .try_reserve_exact(8)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        Ok(Self {
            surfaces: [moving, fixed],
            policy,
            measurements,
            probes,
            _memory: memory,
        })
    }

    fn matches(
        &self,
        moving: &PreparedSurface,
        fixed: &PreparedSurface,
        policy: crate::NormalPolicy,
    ) -> bool {
        std::ptr::eq(self.surfaces[0], moving)
            && std::ptr::eq(self.surfaces[1], fixed)
            && self.policy == policy
    }
}

/// Preserve the best numerical checkpoint on interruption and classify only
/// completed independent passes. Public top-k never reduces the rival pool.
#[expect(
    clippy::too_many_lines,
    reason = "bounded transactional verification and candidate publication"
)]
pub(crate) fn verify_results(
    settings: &SearchSettings,
    result: &mut AlignmentSearchResult,
    control: &GeometryControl,
    cache: &mut VerificationCache<'_>,
) {
    let [moving, fixed] = cache.surfaces;
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
        match measure_candidate(moving, fixed, candidate, settings, full, control, cache) {
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
            match measure_candidate(moving, fixed, candidate, settings, true, control, cache) {
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
        match measure_candidate(moving, fixed, candidate, settings, true, control, cache) {
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
            let cached = cache
                .measurements
                .iter()
                .find(|entry| {
                    cache.matches(moving, fixed, settings.normal_policy)
                        && entry.id == candidate.id
                        && entry.pose == pose
                })
                .and_then(|entry| entry.jackknife);
            let drift = if let Some(drift) = cached {
                control.charge_operations(1).map(|()| drift)
            } else {
                crate::icp::spatial_jackknife(
                    moving,
                    fixed,
                    pose,
                    settings.normal_policy,
                    center,
                    control,
                )
            };
            match drift {
                Ok(drift) => {
                    if let Some(entry) = cache
                        .measurements
                        .iter_mut()
                        .find(|entry| entry.id == candidate.id && entry.pose == pose)
                    {
                        entry.jackknife = Some(drift);
                    }
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
                            match evaluate_probe(probe, settings, control, cache, ordinal) {
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
    for stage in ["tooth-pitch-rivals", "whole-area-support-bound"] {
        if !result.work.unfinished_stages.contains(&stage) {
            result.work.unfinished_stages.push(stage);
        }
    }
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
    // Publish a confident class only after mandatory verification establishes
    // population, scale and rival coverage. Keep finite candidates and
    // established ambiguity reviewable before that authority boundary.
    for candidate in &mut candidates {
        if candidate.confidence == Confidence::Probable && !candidate.evidence.verification_complete
        {
            candidate.confidence = Confidence::Weak;
            if !candidate
                .reasons
                .contains(&EvidenceReason::UniquenessNotEstablished)
            {
                candidate
                    .reasons
                    .push(EvidenceReason::UniquenessNotEstablished);
            }
        }
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
    cache: &mut VerificationCache<'_>,
) -> Result<Option<(crate::Rigid, glam::DVec3, f64)>, GeometryStop> {
    let Some(pose) = moving
        .frame
        .correction_to_query(fixed.frame, candidate.pose)
    else {
        return Ok(None);
    };
    control.charge_operations(cache.measurements.len() as u64 + 1)?;
    let valid = cache.matches(moving, fixed, settings.normal_policy);
    if let Some(entry) = cache.measurements.iter().find(|entry| {
        valid && entry.id == candidate.id && entry.pose == pose && (entry.full || !full)
    }) {
        let training = candidate.evidence.training_info_eigenvalues;
        let twists = candidate.evidence.training_weak_twists.clone();
        candidate.evidence.clone_from(&entry.evidence);
        candidate.evidence.training_info_eigenvalues = training;
        candidate.evidence.training_weak_twists = twists;
        return Ok(Some((pose, entry.center, entry.radius)));
    }
    let mut pass = if full {
        crate::icp::verify_candidate(moving, fixed, pose, settings.normal_policy, control)?
    } else {
        crate::icp::verify_rival_candidate(moving, fixed, pose, settings.normal_policy, control)?
    };
    pass.evidence.training_info_eigenvalues = candidate.evidence.training_info_eigenvalues;
    pass.evidence
        .training_weak_twists
        .clone_from(&candidate.evidence.training_weak_twists);
    if valid {
        let entry = CachedMeasurement {
            id: candidate.id,
            pose,
            full,
            evidence: pass.evidence.clone(),
            center: pass.center,
            radius: pass.radius,
            jackknife: None,
        };
        if let Some(held) = cache
            .measurements
            .iter_mut()
            .find(|entry| entry.id == candidate.id)
        {
            *held = entry;
        } else if cache.measurements.len() < 32 {
            cache.measurements.push(entry);
        }
    }
    candidate.evidence = pass.evidence;
    Ok(Some((pose, pass.center, pass.radius)))
}

fn evaluate_probe(
    probe: crate::Rigid,
    settings: &SearchSettings,
    control: &GeometryControl,
    cache: &mut VerificationCache<'_>,
    ordinal: usize,
) -> Result<Option<AlignmentCandidate>, GeometryStop> {
    let [moving, fixed] = cache.surfaces;
    control.charge_operations(1)?;
    let valid = cache.matches(moving, fixed, settings.normal_policy);
    if let Some((seed, candidate)) = cache.probes.get(ordinal) {
        if valid && *seed == probe {
            return Ok(Some(candidate.clone()));
        }
    }
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
    let candidate = AlignmentCandidate {
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
    };
    if valid {
        if let Some(entry) = cache.probes.get_mut(ordinal) {
            *entry = (probe, candidate.clone());
        } else if ordinal == cache.probes.len() && ordinal < 8 {
            cache.probes.push((probe, candidate.clone()));
        }
    }
    Ok(Some(candidate))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeshInput, RegionPolicy, Rigid, SurfaceSide};
    use glam::DAffine3;

    #[test]
    #[allow(clippy::unwrap_used)]
    #[expect(
        clippy::too_many_lines,
        reason = "one exact cache fixture with pose, policy, resolution and cancellation controls"
    )]
    fn completed_pose_verification_reuses_exact_evidence() {
        let control = GeometryControl::unlimited();
        let mesh = crate::proposal_test_support::arch::plane();
        let surface = crate::prepare_alignment_surface(
            MeshInput {
                soup: mesh.soup(),
                world_from_local: DAffine3::IDENTITY,
                revision: 1,
            },
            SurfaceSide::Moving,
            RegionPolicy::AllEligible,
            &control,
        )
        .unwrap()
        .surface
        .unwrap();
        let settings = SearchSettings::default();
        let mut cache =
            VerificationCache::new(&surface, &surface, settings.normal_policy, &control).unwrap();
        let mut candidate = AlignmentCandidate {
            id: CandidateId {
                family: 0,
                proposal: 0,
            },
            pose: Rigid::IDENTITY,
            confidence: Confidence::Weak,
            evidence: CandidateEvidence::default(),
            reasons: Vec::new(),
            seeds: vec![SeedOrigin::Start],
            refinement: RefinementTermination::NotStarted,
        };
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &settings,
            true,
            &control,
            &mut cache,
        )
        .unwrap();
        let evidence = candidate.evidence.clone();
        let before = control.counters().query_calls;
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &settings,
            true,
            &control,
            &mut cache,
        )
        .unwrap();
        assert_eq!(candidate.evidence, evidence);
        assert_eq!(control.counters().query_calls, before);
        candidate.pose.translation.z = 0.05;
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &settings,
            true,
            &control,
            &mut cache,
        )
        .unwrap();
        assert!(control.counters().query_calls > before);
        let before = control.counters().query_calls;
        let other_policy = SearchSettings {
            normal_policy: crate::NormalPolicy::Unsigned,
            ..settings.clone()
        };
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &other_policy,
            true,
            &control,
            &mut cache,
        )
        .unwrap();
        assert!(control.counters().query_calls > before);
        candidate.id.proposal = 1;
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &settings,
            false,
            &control,
            &mut cache,
        )
        .unwrap();
        let before = control.counters().query_calls;
        measure_candidate(
            &surface,
            &surface,
            &mut candidate,
            &settings,
            true,
            &control,
            &mut cache,
        )
        .unwrap();
        assert!(control.counters().query_calls > before);
        let cancel = occluview_geometry::surface::CancelFlag::new();
        cancel.cancel();
        let cancelled = GeometryControl::new(cancel, std::time::Duration::MAX, control.limits());
        assert_eq!(
            measure_candidate(
                &surface,
                &surface,
                &mut candidate,
                &settings,
                true,
                &cancelled,
                &mut cache
            ),
            Err(GeometryStop::Cancelled)
        );
    }
}
