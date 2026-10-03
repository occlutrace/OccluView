//! Bounded proposal search under the reviewable result contract.
//!
//! Area-based proposals use Horn (1987),
//! <https://doi.org/10.1364/JOSAA.4.000629>, and the independently specified
//! fractional common-area score inspired by Phillips et al. (2006),
//! <https://arxiv.org/abs/cs/0606098>. Completed training scores survive
//! interruption but never stand in for independent verification.

use crate::search_result::{
    AlignmentCandidate, AlignmentInput, AlignmentInputError, AlignmentSearchResult,
    CandidateEvidence, CandidateId, Completion, Confidence, EvidenceReason, InputCheck, InputField,
    Metric, RefinementTermination, SearchProvenance, SearchSettings, SearchWork, SeedOrigin,
};
use crate::{prepare_alignment_surface, SurfaceSide};
use crate::{Rigid, SearchControl};
use glam::DQuat;
use occluview_geometry::surface::GeometryControl;

#[expect(
    clippy::too_many_lines,
    reason = "ordered preparation and terminal checkpoint accounting"
)]
pub(crate) fn search(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
) -> Result<AlignmentSearchResult, AlignmentInputError> {
    let _probe = crate::search_probe::Session::new();
    let mut result = initial_result(input, settings);
    let geometry = control.geometry_control(settings);
    let limits = geometry.limits();
    result.provenance.operation_limit = limits.operations;
    result.provenance.budget.query_calls = limits.query_calls;
    result.provenance.budget.triangle_tests = limits.triangle_tests;
    result.provenance.budget.point_pair_tests = limits.point_pair_tests;
    result.provenance.budget.memory_bytes = limits.memory_bytes;
    result.provenance.budget.patch_edge_visits = settings.work_budget.patch_edge_visits.min(
        if settings.profile == crate::SearchProfile::Extended {
            8_000_000
        } else {
            2_000_000
        },
    );
    if let Some(done) = validate(input, settings, control, &geometry, &mut result)? {
        result.completion = done;
        result.candidates[0]
            .reasons
            .push(EvidenceReason::UnvalidatedInput);
        let counters = geometry.counters();
        result.work.preprocessing_operations = counters.operations;
        result.work.peak_memory_bytes = counters.peak_memory_bytes;
        result.work.elapsed = control.elapsed();
        return Ok(result);
    }
    let candidate = &mut result.candidates[0];
    let start = input.seeds.first().copied().unwrap_or(Rigid::IDENTITY);
    candidate.pose = canonical_seed(start, &mut candidate.reasons);
    let mut effective = settings.clone();
    effective.top_k = effective.top_k.clamp(1, 5);
    effective.influence_radius_mm = clamp_setting(
        settings.influence_radius_mm,
        0.5,
        4.,
        InputField::InfluenceRadius,
        &mut candidate.reasons,
    );
    effective.overlap_prior = settings.overlap_prior.map(|q| {
        clamp_setting(
            q,
            0.01,
            1.,
            InputField::OverlapPrior,
            &mut candidate.reasons,
        )
    });
    let mut recorded_settings = effective.clone();
    recorded_settings.work_budget = result.provenance.budget.clone();
    recorded_settings.wall_limit = geometry.wall_limit();
    result.provenance.effective_settings = Some(recorded_settings);
    let preparation =
        crate::search_probe::Span::new(crate::search_probe::Phase::Preparation, &geometry);
    let moving = prepare_alignment_surface(
        input.moving,
        SurfaceSide::Moving,
        settings.reference_regions,
        &geometry,
    )?;
    let fixed = prepare_alignment_surface(
        input.fixed,
        SurfaceSide::Fixed,
        settings.reference_regions,
        &geometry,
    )?;
    drop(preparation);
    if let (Metric::Measured(a), Metric::Measured(b)) =
        (moving.eligible_area_mm2, fixed.eligible_area_mm2)
    {
        result.candidates[0].evidence.eligible_area_mm2 = Metric::Measured([a, b]);
        result.provenance.processed_area_fraction = Metric::Measured([1., 1.]);
        result
            .work
            .unfinished_stages
            .retain(|&stage| stage != "area-accounting");
    }
    record_preparation(&fixed, &mut result);
    record_preparation(&moving, &mut result);
    if let (Some(moving_surface), Some(fixed_surface)) = (&moving.surface, &fixed.surface) {
        run_proposals(
            input,
            &effective,
            &geometry,
            moving_surface,
            fixed_surface,
            &mut result,
        );
    }
    let counters = geometry.counters();
    result.work.query_calls = counters.query_calls;
    result.work.triangle_tests = counters.triangle_tests;
    result.work.point_pair_tests = counters.point_pair_tests;
    result.work.preprocessing_operations = counters.operations;
    result.work.peak_memory_bytes = counters.peak_memory_bytes;
    if let Some(stop) = geometry.checkpoint() {
        result.completion = crate::sample::completion(stop);
        result.candidates[0]
            .reasons
            .push(EvidenceReason::BudgetExhausted);
    }
    if let Some(done) = control.checkpoint(settings.wall_limit) {
        result.completion = done;
        result.candidates[0].refinement = if done == Completion::Cancelled {
            RefinementTermination::Cancelled
        } else {
            RefinementTermination::Deadline
        };
        result.candidates[0]
            .reasons
            .push(EvidenceReason::BudgetExhausted);
    }
    result.work.elapsed = control.elapsed();
    Ok(result)
}

fn initial_result(input: &AlignmentInput<'_>, settings: &SearchSettings) -> AlignmentSearchResult {
    AlignmentSearchResult {
        candidates: vec![AlignmentCandidate {
            id: CandidateId {
                family: 0,
                proposal: 0,
            },
            pose: Rigid::IDENTITY,
            confidence: Confidence::Weak,
            evidence: CandidateEvidence::default(),
            reasons: vec![EvidenceReason::UniquenessNotEstablished],
            seeds: vec![SeedOrigin::Start],
            refinement: RefinementTermination::NotStarted,
        }],
        completion: Completion::Complete,
        input_check: InputCheck::Complete,
        work: SearchWork {
            retained_poses: 1,
            unfinished_stages: vec![
                "area-accounting",
                "controlled-query-accounting",
                "independent-verification",
                "rival-probes",
            ],
            ..SearchWork::default()
        },
        provenance: SearchProvenance {
            operation_limit: 0,
            proposal_operation_allowance: 0,
            algorithm_version: 34,
            effective_settings: None,
            threshold_set_id: "geometric-evidence-v1-conservative-holdout",
            grid_recipe_id: "haar-polar-6x12-12x24-farthest72-v1",
            input_revisions: [input.moving.revision, input.fixed.revision],
            seed: 0x4f56_5f41_4c52_3100,
            normal_policy: settings.normal_policy,
            reference_regions: settings.reference_regions,
            masked: [
                input.moving.soup.mask.is_some(),
                input.fixed.soup.mask.is_some(),
            ],
            processed_area_fraction: Metric::default(),
            budget: settings.work_budget.clone(),
        },
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one stable scalar validation order with explicit field offsets"
)]
fn validate(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
    geometry: &GeometryControl,
    result: &mut AlignmentSearchResult,
) -> Result<Option<Completion>, AlignmentInputError> {
    if input.landmarks.len() > 128 {
        let total = input
            .moving
            .soup
            .positions
            .len()
            .saturating_add(input.fixed.soup.positions.len())
            .saturating_add(24)
            .saturating_add(input.seeds.len().saturating_mul(7))
            .saturating_add(input.landmarks.len().saturating_mul(12))
            .saturating_add(2);
        result.input_check = InputCheck::Partial { checked: 0, total };
        return Ok(Some(Completion::ResourceLimit));
    }
    let total = input
        .moving
        .soup
        .positions
        .len()
        .saturating_add(input.fixed.soup.positions.len())
        .saturating_add(24)
        .saturating_add(input.seeds.len().saturating_mul(7))
        .saturating_add(
            input
                .landmarks
                .iter()
                .map(|p| {
                    if p.normals_local.is_some() {
                        12usize
                    } else {
                        6
                    }
                })
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(1 + usize::from(settings.overlap_prior.is_some()));
    let mut checked = 0usize;
    let mut check = |field, index, value: f64| -> Result<Option<Completion>, AlignmentInputError> {
        if checked.is_multiple_of(128) {
            if let Some(done) = control.checkpoint(settings.wall_limit) {
                result.input_check = InputCheck::Partial { checked, total };
                return Ok(Some(done));
            }
        }
        if let Err(stop) = geometry.charge_operations(1) {
            result.input_check = InputCheck::Partial { checked, total };
            return Ok(Some(crate::sample::completion(stop)));
        }
        if !value.is_finite() {
            return Err(AlignmentInputError::NonFinite { field, index });
        }
        checked = checked.saturating_add(1);
        Ok(None)
    };
    for (soup, field) in [
        (input.moving.soup, InputField::MovingPositions),
        (input.fixed.soup, InputField::FixedPositions),
    ] {
        for (i, &value) in soup.positions.iter().enumerate() {
            if i >= geometry
                .limits()
                .input_vertices
                .saturating_mul(3)
                .saturating_add(if soup.vertex_count() <= geometry.limits().input_vertices {
                    soup.positions.len() % 3
                } else {
                    0
                })
            {
                // Stop before reading another record; excluded input is still
                // subject to the same scalar validation ceiling.
                result.input_check = InputCheck::Partial { checked, total };
                return Ok(Some(Completion::ResourceLimit));
            }
            if let Some(done) = check(field, i, f64::from(value))? {
                return Ok(Some(done));
            }
        }
    }
    for (affine, field) in [
        (input.moving.world_from_local, InputField::MovingAffine),
        (input.fixed.world_from_local, InputField::FixedAffine),
    ] {
        for (i, value) in affine.to_cols_array().into_iter().enumerate() {
            if let Some(done) = check(field, i, value)? {
                return Ok(Some(done));
            }
        }
    }
    for (i, seed) in input.seeds.iter().enumerate() {
        for (j, value) in seed.rotation.to_array().into_iter().enumerate() {
            if let Some(done) = check(
                InputField::SeedRotation,
                i.saturating_mul(4).saturating_add(j),
                value,
            )? {
                return Ok(Some(done));
            }
        }
        for (j, value) in seed.translation.to_array().into_iter().enumerate() {
            if let Some(done) = check(
                InputField::SeedTranslation,
                i.saturating_mul(3).saturating_add(j),
                value,
            )? {
                return Ok(Some(done));
            }
        }
    }
    for (i, pair) in input.landmarks.iter().enumerate() {
        for (field, point) in [
            (InputField::LandmarkMoving, pair.moving_local),
            (InputField::LandmarkFixed, pair.fixed_local),
        ] {
            for (j, value) in point.to_array().into_iter().enumerate() {
                if let Some(done) = check(field, i.saturating_mul(3).saturating_add(j), value)? {
                    return Ok(Some(done));
                }
            }
        }
        if let Some(normals) = pair.normals_local {
            for (j, value) in normals.into_iter().flat_map(|n| n.to_array()).enumerate() {
                if let Some(done) = check(
                    InputField::LandmarkNormals,
                    i.saturating_mul(6).saturating_add(j),
                    value,
                )? {
                    return Ok(Some(done));
                }
            }
        }
    }
    if let Some(done) = check(InputField::InfluenceRadius, 0, settings.influence_radius_mm)? {
        return Ok(Some(done));
    }
    if let Some(q) = settings.overlap_prior {
        if let Some(done) = check(InputField::OverlapPrior, 0, q)? {
            return Ok(Some(done));
        }
    }
    Ok(None)
}

fn canonical_seed(seed: Rigid, reasons: &mut Vec<EvidenceReason>) -> Rigid {
    let q = seed.rotation.to_array();
    let max = q.into_iter().map(f64::abs).fold(0., f64::max);
    if max == 0. {
        reasons.push(EvidenceReason::InvalidSeed);
        return Rigid {
            rotation: DQuat::IDENTITY,
            translation: seed.translation,
        };
    }
    let scaled = DQuat::from_array(q.map(|x| x / max)).normalize();
    let sign = [scaled.w, scaled.z, scaled.y, scaled.x]
        .into_iter()
        .find(|x| *x != 0.)
        .unwrap_or(1.);
    Rigid {
        rotation: if sign < 0. { -scaled } else { scaled },
        translation: seed.translation,
    }
}

fn clamp_setting(
    value: f64,
    low: f64,
    high: f64,
    field: InputField,
    reasons: &mut Vec<EvidenceReason>,
) -> f64 {
    let effective = value.clamp(low, high);
    if value.to_bits() != effective.to_bits() {
        reasons.push(EvidenceReason::SettingClamped {
            field,
            original: value,
            effective,
        });
    }
    effective
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "result lifetime includes both prepared surfaces and immutable request"
)]
fn run_proposals(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    geometry: &GeometryControl,
    moving: &crate::PreparedSurface,
    fixed: &crate::PreparedSurface,
    result: &mut AlignmentSearchResult,
) {
    let mut reasons = result.candidates[0].reasons.clone();
    let seeds: Vec<_> = input
        .seeds
        .iter()
        .take(32)
        .map(|&seed| canonical_seed(seed, &mut reasons))
        .collect();
    let request = AlignmentInput {
        seeds: &seeds,
        ..*input
    };
    // Ordinary bucket/sort work is bounded independently of query and triangle
    // calls. Leave half of the post-preparation allowance to numerical work;
    // an unfinished producer keeps its checkpoint and explicit partial status.
    let proposal_allowance = geometry
        .limits()
        .operations
        .saturating_sub(geometry.counters().operations)
        / 2;
    result.provenance.proposal_operation_allowance = proposal_allowance;
    let proposal_control = geometry.with_operation_allowance(proposal_allowance);
    let proposals = crate::icp::generate_hypotheses(
        moving,
        fixed,
        &request,
        settings,
        &proposal_control,
        crate::icp::ProposalOptions::default(),
    );
    result.work.examined_poses = proposals.examined;
    result.work.families.clone_from(&proposals.families);
    if !proposals.family_stops.is_empty() {
        result.completion = Completion::WorkLimit;
        result.work.unfinished_stages.push("optional-proposals");
        reasons.push(EvidenceReason::BudgetExhausted);
    }
    result.work.patch_edge_visits = proposals.graph_visits;
    if let Some(stop) = proposals.stop {
        result.completion = crate::sample::completion(stop);
        reasons.push(EvidenceReason::BudgetExhausted);
    }
    if proposals.incomplete_patches {
        reasons.push(EvidenceReason::PartialGeometry);
    }
    if proposals
        .landmark_scale
        .is_some_and(|ratio| (ratio - 1.).abs() > 0.02)
    {
        reasons.push(EvidenceReason::SuspectedUnitsOrDeformation);
    }
    if let Some(warning) = proposals.landmark_warning {
        reasons.push(EvidenceReason::LandmarkRejected(warning));
    }
    let mut pool = proposals.pool;
    if let Some(best) = proposals.best {
        if !pool.iter().any(|p| p.id == best.id) {
            pool.push(best);
        }
    }
    let mut pool = crate::candidate_score::refinement_candidates(&pool, settings.normal_policy);
    // These 16 retained basins form the numerical refinement boundary.
    // Only terminal publication reduces the reviewed result to the UI cap.
    pool.sort_by(crate::candidate_score::proposal_order);
    let refinement_allowance = geometry
        .limits()
        .operations
        .saturating_sub(geometry.counters().operations)
        * 3
        / 4;
    let refinement_control = geometry.with_operation_allowance(refinement_allowance);
    let mut refined =
        crate::icp::run_multiscale(moving, fixed, pool, settings, &refinement_control);
    let proposal_completion = result.completion;
    result.work.iterations = refined.iterations;
    result.work.refinement_scored_poses = refined.scored;
    result.work.examined_poses = proposals.examined.saturating_add(refined.scored);
    result.work.refined_basins = refined.basins;
    refined
        .candidates
        .sort_by(|a, b| crate::candidate_score::proposal_order(&a.proposal, &b.proposal));
    publish_refinement(moving, fixed, &refined, &reasons, result);
    let mut cache = match crate::search_verification::VerificationCache::new(
        moving,
        fixed,
        settings.normal_policy,
        geometry,
    ) {
        Ok(cache) => cache,
        Err(stop) => {
            result.completion = crate::sample::completion(stop);
            for candidate in &mut result.candidates {
                candidate.reasons.push(EvidenceReason::BudgetExhausted);
            }
            result.candidates.truncate(settings.top_k);
            return;
        }
    };
    for attempt in 0..2 {
        result.work.iterations = refined.iterations;
        result.work.refinement_scored_poses = refined.scored;
        result.work.examined_poses = proposals.examined.saturating_add(refined.scored);
        result.work.refined_basins = refined.basins;
        result
            .work
            .unfinished_stages
            .retain(|&stage| stage != "multiscale-refinement");
        reasons.retain(|reason| *reason != EvidenceReason::BudgetExhausted);
        result.completion = proposal_completion;
        if let Some(stop) = refined.stop {
            result.completion = crate::sample::completion(stop);
            result.work.unfinished_stages.push("multiscale-refinement");
            reasons.push(EvidenceReason::BudgetExhausted);
        } else if proposal_completion != Completion::Complete {
            reasons.push(EvidenceReason::BudgetExhausted);
        }
        refined
            .candidates
            .sort_by(|a, b| crate::candidate_score::proposal_order(&a.proposal, &b.proposal));
        publish_refinement(moving, fixed, &refined, &reasons, result);
        crate::search_verification::verify_results(settings, result, geometry, &mut cache);
        let evidence_complete = !result
            .work
            .unfinished_stages
            .contains(&"independent-verification");
        if attempt != 0
            || refined.stop != Some(occluview_geometry::surface::GeometryStop::WorkLimit)
            || !evidence_complete
            || geometry.checkpoint().is_some()
        {
            break;
        }
        result.work.refinement_resumptions += 1;
        // Completed independent passes release their unused headroom. Same-pose
        // evidence is cached; changed checkpoints require new exact passes.
        let before_resume = geometry.counters();
        crate::icp::resume_multiscale(moving, fixed, &mut refined, settings, geometry, false);
        crate::search_probe::continuation(
            before_resume,
            geometry.counters(),
            refined.stop.is_none(),
        );
    }
    result.work.retained_poses = u32::try_from(result.candidates.len()).unwrap_or(5);
    if proposals.stop.is_none() {
        result
            .work
            .unfinished_stages
            .retain(|&stage| stage != "controlled-query-accounting");
    } else {
        result.work.unfinished_stages.push("proposal-families");
    }
}

fn publish_refinement(
    moving: &crate::PreparedSurface,
    fixed: &crate::PreparedSurface,
    refined: &crate::icp::RefinementBatch,
    reasons: &[EvidenceReason],
    result: &mut AlignmentSearchResult,
) {
    let mut candidates = Vec::with_capacity(5);
    for refined_proposal in refined.candidates.iter().take(16) {
        let proposal = &refined_proposal.proposal;
        let Some(pose) = moving.frame.correction_to_world(fixed.frame, proposal.pose) else {
            continue;
        };
        let score = &proposal.score;
        let mut candidate_reasons = reasons.to_vec();
        if score.orientation.is_some_and(|fraction| fraction < 0.75) {
            candidate_reasons.push(EvidenceReason::PolicyConflict);
        }
        if score.overlap < 0.2 {
            candidate_reasons.push(EvidenceReason::InsufficientSupport);
        }
        if refined_proposal.unsigned_fallback {
            candidate_reasons.push(EvidenceReason::PolicyConflict);
        }
        if refined_proposal
            .information
            .as_ref()
            .is_some_and(|info| !info.weak.is_empty())
        {
            candidate_reasons.push(EvidenceReason::UnobservableMotion);
        }
        candidates.push(AlignmentCandidate {
            id: proposal.id,
            pose,
            confidence: Confidence::Weak,
            evidence: CandidateEvidence {
                eligible_area_mm2: Metric::Measured([
                    moving.eligible_area_mm2,
                    fixed.eligible_area_mm2,
                ]),
                representation_area_mm2: Metric::Measured([
                    moving.represented_area_mm2,
                    fixed.represented_area_mm2,
                ]),
                original_surface_exact: [moving.exact_original, fixed.exact_original],
                training_info_eigenvalues: refined_proposal
                    .information
                    .as_ref()
                    .map_or_else(Metric::default, |info| Metric::Measured(info.values)),
                training_weak_twists: refined_proposal
                    .information
                    .as_ref()
                    .map_or_else(Vec::new, |info| info.weak.clone()),
                score: Metric::Measured(score.score),
                queried_population_area_mm2: Metric::Measured(score.population_area),
                policy_compatible_area_mm2: Metric::Measured(score.policy_support),
                coverage_02: Metric::Measured(score.coverage_02),
                coverage_05: Metric::Measured(score.coverage_05),
                common_area_mm2: Metric::Measured(score.common_area),
                overlap_smaller: Metric::Measured(score.overlap),
                trim_fraction: Metric::Measured(score.fraction),
                orientation_fraction: score
                    .orientation
                    .map_or_else(Metric::default, Metric::Measured),
                ..CandidateEvidence::default()
            },
            reasons: candidate_reasons,
            seeds: proposal.origins.clone(),
            refinement: refined_proposal.termination,
        });
    }
    if !candidates.is_empty() {
        result.candidates = candidates;
    }
}

fn record_preparation(prepared: &crate::SurfacePreparation, result: &mut AlignmentSearchResult) {
    if prepared.quality.invalid_triangles != 0
        || prepared.quality.degenerate_triangles != 0
        || prepared.quality.trailing_positions != 0
        || prepared.quality.trailing_indices != 0
    {
        result.candidates[0]
            .reasons
            .push(EvidenceReason::InvalidTopology);
    }
    if !prepared.quality.orientation_coherent && prepared.surface.is_some() {
        result.candidates[0]
            .reasons
            .push(EvidenceReason::MissingNormals);
    }
    if prepared.completion != Completion::Complete {
        result.completion = prepared.completion;
        if prepared.input_check != InputCheck::Complete {
            result.input_check = prepared.input_check;
        }
        result.candidates[0]
            .reasons
            .push(EvidenceReason::PartialGeometry);
    }
}
