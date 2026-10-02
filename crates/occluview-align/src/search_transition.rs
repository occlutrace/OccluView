//! Retain legacy registration checkpoints under the reviewable result contract.
//!
//! The numerical kernels remain the independently implemented Horn pair fit
//! (Horn, 1987, <https://doi.org/10.1364/JOSAA.4.000629>) and trimmed plane ICP
//! (Chetverikov et al., 2005, <https://doi.org/10.1016/j.imavis.2004.05.007>).
//! This boundary changes authority, not those algorithms: vertex diagnostics
//! never stand in for independent area-weighted verification.

use crate::search_result::{
    AlignmentCandidate, AlignmentInput, AlignmentInputError, AlignmentSearchResult,
    CandidateEvidence, CandidateId, Completion, Confidence, EvidenceReason, InputCheck, InputField,
    Metric, NormalPolicy, RefinementTermination, SearchProfile, SearchProvenance, SearchSettings,
    SearchWork, SeedOrigin,
};
use crate::{
    fit_pairs, FitBounds, FitRejection, RefineSettings, Rigid, SearchControl, SurfaceIndex,
};
use glam::{DAffine3, DMat3, DQuat, DVec3};

const MAX_TRANSITION_TRIANGLES: usize = 80_000;
const MAX_TRANSITION_VERTICES: usize = 100_000;

pub(crate) fn search(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
) -> Result<AlignmentSearchResult, AlignmentInputError> {
    let mut result = initial_result(input, settings);
    if let Some(done) = validate(input, settings, control, &mut result)? {
        result.completion = done;
        result.candidates[0]
            .reasons
            .push(EvidenceReason::UnvalidatedInput);
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
    if [input.moving.soup, input.fixed.soup].iter().any(|soup| {
        !soup.positions.len().is_multiple_of(3)
            || !soup.indices.len().is_multiple_of(3)
            || soup
                .indices
                .iter()
                .any(|&i| usize::try_from(i).unwrap_or(usize::MAX) >= soup.vertex_count())
    }) {
        candidate.reasons.push(EvidenceReason::InvalidTopology);
    }
    if oversized(input)
        || input.landmarks.len() > 128
        || settings.work_budget.memory_bytes < 64 * 1024 * 1024
    {
        result.completion = Completion::ResourceLimit;
        candidate.reasons.push(EvidenceReason::PartialGeometry);
    } else if settings.work_budget.query_calls == 0 || settings.work_budget.triangle_tests == 0 {
        result.completion = Completion::WorkLimit;
        candidate.reasons.push(EvidenceReason::BudgetExhausted);
    } else if let Some(authored) = rigid_affine(input.moving.world_from_local) {
        // The legacy index consumes world-space f32 fixed vertices. A different
        // fixed frame or a nonrigid moving frame requires the f64 surface view.
        if input.fixed.world_from_local == DAffine3::IDENTITY {
            run_legacy(input, &effective, control, authored, &mut result);
        } else {
            candidate.reasons.push(EvidenceReason::PartialGeometry);
        }
    } else {
        candidate.reasons.push(EvidenceReason::PartialGeometry);
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
            algorithm_version: 1,
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
    result: &mut AlignmentSearchResult,
) -> Result<Option<Completion>, AlignmentInputError> {
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

fn rigid_affine(affine: DAffine3) -> Option<Rigid> {
    let columns = [
        affine.matrix3.x_axis,
        affine.matrix3.y_axis,
        affine.matrix3.z_axis,
    ];
    if columns
        .iter()
        .any(|x| (x.length_squared() - 1.).abs() > 1e-5)
        || [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .any(|(a, b)| columns[a].dot(columns[b]).abs() > 1e-5)
        || affine.matrix3.determinant() <= 0.
    {
        return None;
    }
    Some(Rigid::new(
        DQuat::from_mat3(&DMat3::from_cols(columns[0], columns[1], columns[2])),
        affine.translation,
    ))
}

fn oversized(input: &AlignmentInput<'_>) -> bool {
    [input.moving, input.fixed].iter().any(|mesh| {
        mesh.soup.triangle_count() > MAX_TRANSITION_TRIANGLES
            || mesh.soup.vertex_count() > MAX_TRANSITION_VERTICES
            || mesh.soup.positions.iter().any(|x| x.abs() > 100_000.)
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "paired legacy diagnostics and correction composition at one boundary"
)]
fn run_legacy(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
    authored: Rigid,
    result: &mut AlignmentSearchResult,
) {
    let Some(fixed) = SurfaceIndex::build(input.fixed.soup) else {
        result.completion = Completion::NoUsableSurface;
        result.candidates[0]
            .reasons
            .push(EvidenceReason::InsufficientSupport);
        return;
    };
    // A finite landmark failure is a seed warning; scan search still runs.
    let mut start = result.candidates[0].pose.compose(&authored);
    if !start.is_finite() {
        result.candidates[0]
            .reasons
            .push(EvidenceReason::InvalidSeed);
        return;
    }
    if !input.landmarks.is_empty() {
        let moving: Vec<_> = input
            .landmarks
            .iter()
            .map(|p| authored.apply(p.moving_local))
            .collect();
        let fixed_points: Vec<_> = input.landmarks.iter().map(|p| p.fixed_local).collect();
        if moving.iter().chain(&fixed_points).all(|p| p.is_finite()) {
            let bounds = FitBounds {
                moving_center: DVec3::ZERO,
                fixed_center: DVec3::ZERO,
                moving_extent: 1e6,
                fixed_extent: 1e6,
            };
            let moving_normals: Option<Vec<_>> = input
                .landmarks
                .iter()
                .map(|p| p.normals_local.map(|n| authored.apply_normal(n[0])))
                .collect();
            let fixed_normals: Option<Vec<_>> = input
                .landmarks
                .iter()
                .map(|p| {
                    p.normals_local.map(|n| {
                        if settings.normal_policy == NormalPolicy::Opposed {
                            -n[1]
                        } else {
                            n[1]
                        }
                    })
                })
                .collect();
            let normals = moving_normals
                .as_ref()
                .zip(fixed_normals.as_ref())
                .map(|(moving, fixed)| (moving.as_slice(), fixed.as_slice()));
            match fit_pairs(&moving, &fixed_points, normals, &bounds) {
                Ok(fit) => {
                    start = fit.rigid.compose(&authored);
                    result.candidates[0].id = CandidateId {
                        family: 1,
                        proposal: 0,
                    };
                    if (fit.unit_ratio - 1.).abs() > 0.02 {
                        result.candidates[0]
                            .reasons
                            .push(EvidenceReason::SuspectedUnitsOrDeformation);
                    }
                    result.candidates[0].seeds.push(SeedOrigin::Landmarks);
                }
                Err(reason) => result.candidates[0]
                    .reasons
                    .push(EvidenceReason::LandmarkRejected(reason)),
            }
        } else {
            result.candidates[0]
                .reasons
                .push(EvidenceReason::PartialGeometry);
        }
    }
    let refine_settings = RefineSettings {
        influence_radius_mm: settings.influence_radius_mm,
        matching_ratio: settings.overlap_prior.unwrap_or(0.8),
        orientation: match settings.normal_policy {
            NormalPolicy::Match => crate::Orientation::Match,
            NormalPolicy::Opposed => crate::Orientation::Inverted,
            NormalPolicy::Unsigned => crate::Orientation::Ignored,
        },
        local_only: settings.profile == SearchProfile::Local,
        max_iterations: 40,
    };
    let mut last = None;
    let mut watchdog_failed = false;
    let legacy_cancel = crate::CancelFlag::new();
    let outcome = std::thread::scope(|scope| {
        let (finished, stopped) = std::sync::mpsc::channel::<()>();
        let cancel = &legacy_cancel;
        let watchdog = std::thread::Builder::new()
            .name("alignment-deadline".into())
            .spawn_scoped(scope, move || loop {
                if control.checkpoint(settings.wall_limit).is_some() {
                    cancel.cancel();
                    break;
                }
                if stopped.recv_timeout(std::time::Duration::from_millis(5))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            });
        if watchdog.is_err() {
            watchdog_failed = true;
            legacy_cancel.cancel();
        }
        let outcome = crate::icp::refine_checkpointed(
            input.moving.soup,
            &fixed,
            start,
            &refine_settings,
            &legacy_cancel,
            &mut |report| {
                if report.rigid.is_finite() {
                    last = Some(report);
                }
                if control.checkpoint(settings.wall_limit).is_some() {
                    legacy_cancel.cancel();
                }
            },
        );
        let _ = finished.send(());
        outcome
    });
    if watchdog_failed {
        result.completion = Completion::ResourceLimit;
        result.candidates[0]
            .reasons
            .push(EvidenceReason::BudgetExhausted);
    }
    result.work.examined_poses = 1;
    if let Ok(report) = outcome {
        if report.rigid.is_finite() {
            last = Some(report);
        }
    }
    if let Some(report) = last {
        let correction = report.rigid.compose(&authored.inverse());
        if correction.is_finite() {
            result.candidates[0].pose =
                canonical_seed(correction, &mut result.candidates[0].reasons);
            result.work.iterations = u64::from(report.iterations);
            // Infinite/missing legacy statistics must never enter diagnostics.
            if [
                report.rms,
                report.geometric_rms,
                report.median_abs,
                report.p95_abs,
                report.coverage,
                report.support_coverage,
                report.inlier_ratio,
                report.effective_matching_ratio,
                report.verified_coverage,
                report.verified_support_coverage,
                report.verified_median_mm,
                report.verified_stability,
            ]
            .into_iter()
            .all(f64::is_finite)
            {
                result.candidates[0].evidence.legacy_report = Some(report);
            }
            result.candidates[0].refinement = if report.converged {
                RefinementTermination::Stationary
            } else {
                RefinementTermination::IterationLimit
            };
        }
    }
    if let Err(reason) = outcome {
        result.candidates[0]
            .reasons
            .push(EvidenceReason::LegacyRejected(reason));
        result.candidates[0].refinement = match reason {
            FitRejection::TooFewPairs { .. } => RefinementTermination::NoCorrespondences,
            FitRejection::NoImprovement | FitRejection::Ambiguous => {
                RefinementTermination::Stationary
            }
            _ => RefinementTermination::NumericalTrialRejected,
        };
    }
}
