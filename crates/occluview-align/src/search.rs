//! The reviewable search: the input is checked, the search runs, and every
//! pose it returns is classed by its evidence.
//!
//! A numerically finite request always yields at least one finite pose. A
//! pose the evidence does not support is returned as Weak with the reasons;
//! it is never withheld and never dressed up.

use crate::registration::{register, Registration};
use crate::search_result::{
    AlignmentCandidate, AlignmentInput, AlignmentInputError, AlignmentSearchResult,
    CandidateEvidence, CandidateId, Completion, Confidence, EvidenceReason, FamilyEvidence,
    InputCheck, InputField, Metric, RefinementTermination, SearchProvenance, SearchSettings,
    SearchWork, SeedOrigin,
};
use crate::{Rigid, SearchControl};
use glam::DQuat;

/// Most triangles and vertices one scan may have.
const MAX_TRIANGLES: usize = 8_000_000;
const MAX_VERTICES: usize = 12_000_000;
/// Most seeds and landmark pairs one request may carry.
const MAX_SEEDS: usize = 32;
const MAX_LANDMARKS: usize = 128;
/// Scalars checked between two looks at the clock.
const CHECK_BLOCK: usize = 4_096;
/// A moving surface that reads this much larger or smaller than the fixed
/// one is another object or in other units.
const SIZE_CONFLICT: f64 = 0.01;

/// Search for reviewable finite corrections of the authored moving frame.
///
/// Geometry and authored affines are immutable; accepting a pose is a
/// separate act of the application.
///
/// # Errors
/// Returns the field and index of an encountered NaN or infinity. Nothing
/// else is an error: an input the search cannot use, or a search ended early,
/// returns the poses it has with the reasons.
pub fn search_alignment(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
) -> Result<AlignmentSearchResult, AlignmentInputError> {
    let mut effective = settings.clone();
    effective.top_k = effective.top_k.clamp(1, 5);
    let mut reasons = Vec::new();
    let seeds: Vec<Rigid> = input
        .seeds
        .iter()
        .take(MAX_SEEDS)
        .map(|&seed| canonical_seed(seed, &mut reasons))
        .collect();
    let mut result = initial_result(input, &effective, seeds.first().copied(), reasons);
    let too_large = [input.moving, input.fixed].iter().any(|mesh| {
        mesh.soup.triangle_count() > MAX_TRIANGLES || mesh.soup.vertex_count() > MAX_VERTICES
    }) || input.landmarks.len() > MAX_LANDMARKS;
    let unchecked = if too_large {
        result.input_check = InputCheck::Partial {
            checked: 0,
            total: scalar_count(input),
        };
        Some(Completion::ResourceLimit)
    } else {
        validate(input, &effective, control, &mut result)?
    };
    if let Some(done) = unchecked {
        result.completion = done;
        result.candidates[0]
            .reasons
            .push(EvidenceReason::UnvalidatedInput);
    } else {
        let request = AlignmentInput {
            seeds: &seeds,
            ..*input
        };
        let registration = register(&request, &effective, control);
        publish(registration, &effective, &mut result);
    }
    result.work.elapsed = control.elapsed();
    Ok(result)
}

/// The answer before any search: the placement as given, Weak.
fn initial_result(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    start: Option<Rigid>,
    mut reasons: Vec<EvidenceReason>,
) -> AlignmentSearchResult {
    reasons.push(EvidenceReason::UniquenessNotEstablished);
    AlignmentSearchResult {
        candidates: vec![AlignmentCandidate {
            id: CandidateId {
                family: SeedOrigin::Start as u8,
                proposal: 0,
            },
            pose: start.unwrap_or(Rigid::IDENTITY),
            confidence: Confidence::Weak,
            evidence: CandidateEvidence::default(),
            reasons,
            seeds: vec![SeedOrigin::Start],
            refinement: RefinementTermination::NotStarted,
        }],
        completion: Completion::Complete,
        input_check: InputCheck::Complete,
        work: SearchWork {
            retained_poses: 1,
            ..SearchWork::default()
        },
        provenance: SearchProvenance {
            settings: settings.clone(),
            threshold_set_id: "geometric-evidence-v2",
            algorithm_version: 40,
            input_revisions: [input.moving.revision, input.fixed.revision],
            masked: [
                input.moving.soup.mask.is_some(),
                input.fixed.soup.mask.is_some(),
            ],
        },
    }
}

/// Turn the search's poses into reviewable candidates and class each one.
fn publish(
    registration: Registration,
    settings: &SearchSettings,
    result: &mut AlignmentSearchResult,
) {
    let mut reasons = result.candidates[0].reasons.clone();
    if registration.omitted != [0; 2] {
        reasons.push(EvidenceReason::InvalidTopology);
    }
    if registration.coherent.contains(&Some(false)) {
        reasons.push(EvidenceReason::MissingNormals);
    }
    if let Some(rejection) = registration.landmark_rejection {
        reasons.push(EvidenceReason::LandmarkRejected(rejection));
    }
    result.completion = registration.stopped.unwrap_or(Completion::Complete);
    if matches!(
        registration.stopped,
        Some(Completion::Deadline | Completion::Cancelled)
    ) {
        reasons.push(EvidenceReason::BudgetExhausted);
    }
    result.work.iterations = registration.rounds;
    result.work.examined_poses = registration
        .examined
        .iter()
        .map(|&(_, count)| u64::from(count))
        .sum();
    result.work.families = registration
        .examined
        .iter()
        .map(|&(family, examined)| FamilyEvidence {
            family,
            examined,
            retained: u32::try_from(
                registration
                    .found
                    .iter()
                    .filter(|found| found.origin == family)
                    .count(),
            )
            .unwrap_or(u32::MAX),
        })
        .collect();
    if registration.found.is_empty() {
        let first = &mut result.candidates[0];
        first.evidence.eligible_area_mm2 = Metric::Measured(registration.areas);
        first.reasons = reasons;
        return;
    }
    reasons.retain(|reason| *reason != EvidenceReason::UniquenessNotEstablished);
    result.candidates = registration
        .found
        .into_iter()
        .map(|found| AlignmentCandidate {
            id: CandidateId {
                family: found.origin as u8,
                proposal: found.proposal,
            },
            pose: found.pose,
            confidence: Confidence::Weak,
            evidence: found.evidence,
            reasons: reasons.clone(),
            seeds: vec![found.origin],
            refinement: found.termination,
        })
        .collect();
    for candidate in &mut result.candidates {
        if matches!(candidate.evidence.size_trend, Metric::Measured(trend) if trend.abs() > SIZE_CONFLICT)
        {
            candidate
                .reasons
                .push(EvidenceReason::SuspectedUnitsOrDeformation);
        }
        crate::confidence::classify_candidate(
            candidate,
            result.completion,
            result.input_check,
            settings.profile,
        );
    }
    result.work.retained_poses = u32::try_from(result.candidates.len()).unwrap_or(u32::MAX);
}

/// Scalars of a request, in the order they are checked.
fn scalar_count(input: &AlignmentInput<'_>) -> usize {
    let landmarks = input
        .landmarks
        .iter()
        .map(|pair| {
            if pair.normals_local.is_some() {
                12usize
            } else {
                6
            }
        })
        .fold(0usize, usize::saturating_add);
    input
        .moving
        .soup
        .positions
        .len()
        .saturating_add(input.fixed.soup.positions.len())
        .saturating_add(24)
        .saturating_add(input.seeds.len().saturating_mul(7))
        .saturating_add(landmarks)
}

/// Check every supplied scalar, excluded geometry included. Ends early only
/// when the clock or the caller ends the search; the coverage reached is
/// then recorded.
fn validate(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
    result: &mut AlignmentSearchResult,
) -> Result<Option<Completion>, AlignmentInputError> {
    let total = scalar_count(input);
    let mut checked = 0usize;
    if let Some(done) = control.checkpoint(settings.wall_limit) {
        result.input_check = InputCheck::Partial { checked, total };
        return Ok(Some(done));
    }
    let positions = [
        (input.moving.soup.positions, InputField::MovingPositions),
        (input.fixed.soup.positions, InputField::FixedPositions),
    ];
    for (values, field) in positions {
        for (block, chunk) in values.chunks(CHECK_BLOCK).enumerate() {
            if let Some(done) = control.checkpoint(settings.wall_limit) {
                result.input_check = InputCheck::Partial { checked, total };
                return Ok(Some(done));
            }
            if let Some(offset) = chunk.iter().position(|value| !value.is_finite()) {
                return Err(AlignmentInputError::NonFinite {
                    field,
                    index: block * CHECK_BLOCK + offset,
                });
            }
            checked += chunk.len();
        }
    }
    let finite = |field, values: &[f64], base: usize| match values
        .iter()
        .position(|value| !value.is_finite())
    {
        Some(offset) => Err(AlignmentInputError::NonFinite {
            field,
            index: base + offset,
        }),
        None => Ok(()),
    };
    finite(
        InputField::MovingAffine,
        &input.moving.world_from_local.to_cols_array(),
        0,
    )?;
    finite(
        InputField::FixedAffine,
        &input.fixed.world_from_local.to_cols_array(),
        0,
    )?;
    for (i, seed) in input.seeds.iter().enumerate() {
        finite(InputField::SeedRotation, &seed.rotation.to_array(), i * 4)?;
        finite(
            InputField::SeedTranslation,
            &seed.translation.to_array(),
            i * 3,
        )?;
    }
    for (i, pair) in input.landmarks.iter().enumerate() {
        finite(
            InputField::LandmarkMoving,
            &pair.moving_local.to_array(),
            i * 3,
        )?;
        finite(
            InputField::LandmarkFixed,
            &pair.fixed_local.to_array(),
            i * 3,
        )?;
        if let Some([moving, fixed]) = pair.normals_local {
            finite(InputField::LandmarkNormals, &moving.to_array(), i * 6)?;
            finite(InputField::LandmarkNormals, &fixed.to_array(), i * 6 + 3)?;
        }
    }
    Ok(None)
}

/// A seed with a unit rotation of fixed sign; a zero rotation becomes the
/// identity and is reported.
fn canonical_seed(seed: Rigid, reasons: &mut Vec<EvidenceReason>) -> Rigid {
    let q = seed.rotation.to_array();
    let max = q.into_iter().map(f64::abs).fold(0., f64::max);
    if max == 0. || !max.is_finite() {
        if max == 0. {
            reasons.push(EvidenceReason::InvalidSeed);
        }
        return Rigid {
            rotation: if max == 0. {
                DQuat::IDENTITY
            } else {
                seed.rotation
            },
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
