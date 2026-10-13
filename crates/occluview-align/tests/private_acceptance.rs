//! Opt-in acceptance against a maintainer-owned corpus. Ordinary test runs
//! never read private files; the one integration case is explicitly ignored.
#![allow(
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
)]

use glam::{DAffine3, DQuat, DVec3};
use occluview_align::{
    search_alignment, AlignmentInput, Completion, Confidence, InputCheck, MeshInput,
    Metric, NormalPolicy, PointPair, RegionPolicy, Rigid, SearchControl, SearchProfile,
    SearchSettings, Soup,
};
use occluview_formats::dispatch::read_file_bytes;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    moving: AssetSpec,
    fixed: AssetSpec,
    search: SearchSpec,
    expectation: Expectation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetSpec {
    file: String,
    /// Explicit maintainer-reviewed conversion from source coordinates to mm.
    scale_to_mm: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchSpec {
    normal_policy: NormalPolicySpec,
    profile: ProfileSpec,
    top_k: usize,
    reference_regions: RegionSpec,
    wall_limit_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NormalPolicySpec {
    Match,
    Opposed,
    Unsigned,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProfileSpec {
    Standard,
    Local,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RegionSpec {
    AllEligible,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Expectation {
    Pose {
        truth: TruthPose,
        transform_tolerance: TransformTolerance,
        control_landmarks: Vec<ControlLandmark>,
        maximum_control_landmark_error_mm: f64,
        quality: PoseQualityBounds,
    },
    NotVerified {
        maximum_confidence: ConfidenceSpec,
        required_reason: ExpectedReason,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TruthPose {
    rotation_xyzw: [f64; 4],
    translation_mm: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformTolerance {
    rotation_deg: f64,
    translation_mm: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlLandmark {
    moving_mm: [f64; 3],
    fixed_mm: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PoseQualityBounds {
    minimum_confidence: ConfidenceSpec,
    minimum_overlap_smaller: f64,
    maximum_rms_mm: f64,
    maximum_p95_mm: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfidenceSpec {
    Weak,
    Ambiguous,
    Probable,
    Verified,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedReason {
    InsufficientSupport,
    UnobservableMotion,
    CloseRival,
    PolicyConflict,
    SuspectedUnitsOrDeformation,
    MissingNormals,
    UniquenessNotEstablished,
    InvalidTopology,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CaseResult {
    case_id: String,
    passed: bool,
    completion: &'static str,
    input_check_complete: bool,
    confidence: &'static str,
    reasons: Vec<&'static str>,
    candidate_family: u8,
    candidate_proposal: u32,
    candidate_evidence: Vec<CandidateOutcome>,
    rotation_error_deg: Option<f64>,
    translation_error_mm: Option<f64>,
    maximum_control_landmark_error_mm: Option<f64>,
    overlap_smaller: Option<f64>,
    rms_mm: Option<f64>,
    p95_mm: Option<f64>,
    examined_poses: u64,
    iterations: u64,
    elapsed_ms: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CandidateOutcome {
    family: u8,
    proposal: u32,
    confidence: &'static str,
    reasons: Vec<&'static str>,
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Weak => "weak",
        Confidence::Ambiguous => "ambiguous",
        Confidence::Probable => "probable",
        Confidence::Verified => "verified",
    }
}

fn confidence_rank(confidence: Confidence) -> u8 {
    match confidence {
        Confidence::Weak => 0,
        Confidence::Ambiguous => 1,
        Confidence::Probable => 2,
        Confidence::Verified => 3,
    }
}

fn candidate_outcomes(
    candidates: &[occluview_align::AlignmentCandidate],
) -> Vec<CandidateOutcome> {
    candidates
        .iter()
        .map(|candidate| CandidateOutcome {
            family: candidate.id.family,
            proposal: candidate.id.proposal,
            confidence: confidence_name(candidate.confidence),
            reasons: candidate.reasons.iter().map(reason_name).collect(),
        })
        .collect()
}

fn not_verified_candidates_pass(
    result: &occluview_align::AlignmentSearchResult,
    maximum_confidence: Confidence,
    required_selected_reason: &str,
) -> bool {
    let Some(selected) = result.candidates.first() else {
        return false;
    };
    let every_candidate_is_below_ceiling = result.candidates.iter().all(|candidate| {
        confidence_rank(candidate.confidence) <= confidence_rank(maximum_confidence)
    });
    let selected_reason_matches = selected
        .reasons
        .iter()
        .any(|reason| reason_name(reason) == required_selected_reason);
    every_candidate_is_below_ceiling && selected_reason_matches
}

fn required_confidence(spec: ConfidenceSpec) -> Confidence {
    match spec {
        ConfidenceSpec::Weak => Confidence::Weak,
        ConfidenceSpec::Ambiguous => Confidence::Ambiguous,
        ConfidenceSpec::Probable => Confidence::Probable,
        ConfidenceSpec::Verified => Confidence::Verified,
    }
}

fn reason_name(reason: &occluview_align::EvidenceReason) -> &'static str {
    use occluview_align::EvidenceReason;
    match reason {
        EvidenceReason::InsufficientSupport => "insufficient_support",
        EvidenceReason::UnobservableMotion => "unobservable_motion",
        EvidenceReason::CloseRival => "close_rival",
        EvidenceReason::PolicyConflict => "policy_conflict",
        EvidenceReason::SuspectedUnitsOrDeformation => "suspected_units_or_deformation",
        EvidenceReason::MissingNormals => "missing_normals",
        EvidenceReason::UnvalidatedInput => "unvalidated_input",
        EvidenceReason::BudgetExhausted => "budget_exhausted",
        EvidenceReason::InvalidSeed => "invalid_seed",
        EvidenceReason::LandmarkRejected(_) => "landmark_rejected",
        EvidenceReason::UniquenessNotEstablished => "uniqueness_not_established",
        EvidenceReason::InvalidTopology => "invalid_topology",
    }
}

fn expected_reason_name(reason: ExpectedReason) -> &'static str {
    match reason {
        ExpectedReason::InsufficientSupport => "insufficient_support",
        ExpectedReason::UnobservableMotion => "unobservable_motion",
        ExpectedReason::CloseRival => "close_rival",
        ExpectedReason::PolicyConflict => "policy_conflict",
        ExpectedReason::SuspectedUnitsOrDeformation => "suspected_units_or_deformation",
        ExpectedReason::MissingNormals => "missing_normals",
        ExpectedReason::UniquenessNotEstablished => "uniqueness_not_established",
        ExpectedReason::InvalidTopology => "invalid_topology",
    }
}

fn metric_number(metric: Metric<f64>) -> Option<f64> {
    match metric {
        Metric::Measured(value) if value.is_finite() && (0.0..=1.0).contains(&value) => Some(value),
        Metric::Measured(_) | Metric::Missing(_) => None,
    }
}

fn residual_numbers(
    metric: Metric<occluview_align::ResidualSummary>,
) -> (Option<f64>, Option<f64>) {
    match metric {
        Metric::Measured(value)
            if value.median.is_finite()
                && value.rms.is_finite()
                && value.p95.is_finite()
                && value.median >= 0.0
                && value.rms >= 0.0
                && value.p95 >= value.median =>
        {
            (Some(value.rms), Some(value.p95))
        }
        Metric::Measured(_) | Metric::Missing(_) => (None, None),
    }
}

fn validate_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix("case_") else {
        return false;
    };
    suffix.len() == 12 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn validate_manifest(manifest: &Manifest) {
    assert_eq!(manifest.schema_version, 1, "unsupported private manifest schema");
    assert!(!manifest.cases.is_empty(), "private manifest contains no cases");
    let mut ids = std::collections::HashSet::new();
    for case in &manifest.cases {
        assert!(validate_id(&case.id), "case ID must be a random case_<12 hex> token");
        assert!(ids.insert(&case.id), "private manifest contains a duplicate case ID");
        for asset in [&case.moving, &case.fixed] {
            assert!(valid_relative_file(&asset.file), "case asset path must be relative and traversal-free");
            assert!(asset.scale_to_mm.is_finite() && asset.scale_to_mm > 0.0, "asset scale_to_mm must be finite and positive");
        }
        assert!((1..=5).contains(&case.search.top_k), "search top_k must be between 1 and 5");
        assert!(case.search.wall_limit_ms > 0, "search wall_limit_ms must be positive");
        if let Expectation::Pose {
            truth,
            transform_tolerance,
            control_landmarks,
            maximum_control_landmark_error_mm,
            quality,
        } = &case.expectation
        {
            assert!((3..=128).contains(&control_landmarks.len()), "pose cases require 3 to 128 independent control landmarks");
            assert!(truth.rotation_xyzw.into_iter().chain(truth.translation_mm).all(f64::is_finite), "expected pose must be finite");
            let quaternion = DQuat::from_xyzw(
                truth.rotation_xyzw[0],
                truth.rotation_xyzw[1],
                truth.rotation_xyzw[2],
                truth.rotation_xyzw[3],
            );
            assert!((quaternion.length() - 1.0).abs() <= 1.0e-9, "expected pose quaternion must be normalized");
            assert!(valid_nonnegative(transform_tolerance.rotation_deg), "rotation tolerance must be finite and nonnegative");
            assert!(valid_nonnegative(transform_tolerance.translation_mm), "translation tolerance must be finite and nonnegative");
            assert!(valid_nonnegative(*maximum_control_landmark_error_mm), "control landmark bound must be finite and nonnegative");
            assert!(control_landmarks.iter().all(|landmark| {
                landmark.moving_mm.into_iter().chain(landmark.fixed_mm).all(f64::is_finite)
            }), "control landmarks must be finite");
            assert!(quality.minimum_overlap_smaller.is_finite() && (0.0..=1.0).contains(&quality.minimum_overlap_smaller), "minimum overlap bound must be between zero and one");
            assert!(valid_nonnegative(quality.maximum_rms_mm), "maximum RMS bound must be finite and nonnegative");
            assert!(valid_nonnegative(quality.maximum_p95_mm), "maximum p95 bound must be finite and nonnegative");
        }
        if let Expectation::NotVerified { maximum_confidence, .. } = &case.expectation {
            assert!(
                confidence_rank(required_confidence(*maximum_confidence)) < confidence_rank(Confidence::Verified),
                "not_verified case must cap confidence below verified"
            );
        }
    }
}

fn valid_relative_file(file: &str) -> bool {
    !file.is_empty()
        && !file.contains('\\')
        && !file.contains(':')
        && !Path::new(file).is_absolute()
        && Path::new(file)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn resolve_asset(root: &Path, asset: &AssetSpec) -> PathBuf {
    let mut candidate = root.to_path_buf();
    for component in Path::new(&asset.file).components() {
        let Component::Normal(part) = component else {
            panic!("case asset path must be relative and traversal-free");
        };
        candidate.push(part);
        let metadata = fs::symlink_metadata(&candidate)
            .unwrap_or_else(|_| panic!("case asset is unavailable"));
        assert!(!metadata.file_type().is_symlink(), "case asset path cannot include symlinks");
    }
    let canonical = candidate
        .canonicalize()
        .unwrap_or_else(|_| panic!("case asset is unavailable"));
    assert!(canonical.starts_with(root), "case asset resolves outside corpus root");
    assert!(canonical.is_file(), "case asset is not a file");
    canonical
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    encoded
}

fn update_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn corpus_digest(manifest_sha: &[u8; 32], entries: &[(String, String, [u8; 32])]) -> String {
    let mut hasher = Sha256::new();
    update_field(&mut hasher, b"occluview-private-acceptance-corpus-v1");
    update_field(&mut hasher, manifest_sha);
    for (case_id, private_relative_path, file_sha) in entries {
        update_field(&mut hasher, case_id.as_bytes());
        update_field(&mut hasher, private_relative_path.as_bytes());
        update_field(&mut hasher, file_sha);
    }
    hex_digest(&hasher.finalize())
}

struct LoadedGeometry {
    positions: Vec<f32>,
    indices: Vec<u32>,
    file_sha: [u8; 32],
}

fn load_mesh(path: &Path, scale_to_mm: f64) -> LoadedGeometry {
    let file_bytes = read_file_bytes(path).unwrap_or_else(|_| panic!("case asset could not be read"));
    let file_sha = sha256(file_bytes.as_slice());
    let mesh = file_bytes
        .dispatch()
        .unwrap_or_else(|_| panic!("case asset could not be parsed by the product reader"));
    let mut positions = Vec::with_capacity(mesh.vertices().len() * 3);
    for vertex in mesh.vertices() {
        for coordinate in vertex.position {
            let scaled = f64::from(coordinate) * scale_to_mm;
            let coordinate_mm = scaled as f32;
            assert!(scaled.is_finite() && coordinate_mm.is_finite(), "scaled asset coordinate is not finite");
            positions.push(coordinate_mm);
        }
    }
    LoadedGeometry {
        positions,
        indices: mesh.indices().to_vec(),
        file_sha,
    }
}

fn search_settings(spec: &SearchSpec) -> SearchSettings {
    SearchSettings {
        normal_policy: match spec.normal_policy {
            NormalPolicySpec::Match => NormalPolicy::Match,
            NormalPolicySpec::Opposed => NormalPolicy::Opposed,
            NormalPolicySpec::Unsigned => NormalPolicy::Unsigned,
        },
        profile: match spec.profile {
            ProfileSpec::Standard => SearchProfile::Standard,
            ProfileSpec::Local => SearchProfile::Local,
        },
        top_k: spec.top_k,
        reference_regions: match spec.reference_regions {
            RegionSpec::AllEligible => RegionPolicy::AllEligible,
        },
        wall_limit: Duration::from_millis(spec.wall_limit_ms),
    }
}

fn confidence_satisfies(actual: Confidence, expected: Confidence) -> bool {
    confidence_rank(actual) >= confidence_rank(expected)
}

fn pose_result(case: &Case, result: &occluview_align::AlignmentSearchResult) -> CaseResult {
    let candidate = &result.candidates[0];
    let mut passed = result.completion == Completion::Complete
        && result.input_check == InputCheck::Complete;
    let mut rotation_error_deg = None;
    let mut translation_error_mm = None;
    let mut maximum_control_landmark_error_mm = None;

    if let Expectation::Pose {
        truth,
        transform_tolerance,
        control_landmarks,
        maximum_control_landmark_error_mm: landmark_bound,
        quality,
    } = &case.expectation
    {
        let expected_rotation = DQuat::from_xyzw(
            truth.rotation_xyzw[0],
            truth.rotation_xyzw[1],
            truth.rotation_xyzw[2],
            truth.rotation_xyzw[3],
        );
        let expected = Rigid::new(expected_rotation, DVec3::from_array(truth.translation_mm));
        let relative = (candidate.pose.rotation * expected.rotation.conjugate()).normalize();
        let angle = 2.0 * relative.w.abs().clamp(-1.0, 1.0).acos().to_degrees();
        let translation = candidate.pose.translation.distance(expected.translation);
        rotation_error_deg = Some(angle);
        translation_error_mm = Some(translation);

        let mut maximum_landmark_error = 0.0_f64;
        for landmark in control_landmarks {
            let moved = candidate.pose.apply(DVec3::from_array(landmark.moving_mm));
            let error = moved.distance(DVec3::from_array(landmark.fixed_mm));
            maximum_landmark_error = maximum_landmark_error.max(error);
        }
        maximum_control_landmark_error_mm = Some(maximum_landmark_error);
        passed &= angle <= transform_tolerance.rotation_deg
            && translation <= transform_tolerance.translation_mm
            && maximum_landmark_error <= *landmark_bound
            && confidence_satisfies(candidate.confidence, required_confidence(quality.minimum_confidence));
        let overlap = metric_number(candidate.evidence.overlap_smaller);
        let (rms, p95) = residual_numbers(candidate.evidence.euclidean_mm);
        passed &= overlap.is_some_and(|value| value >= quality.minimum_overlap_smaller)
            && rms.is_some_and(|value| value <= quality.maximum_rms_mm)
            && p95.is_some_and(|value| value <= quality.maximum_p95_mm);
    }

    let overlap_smaller = metric_number(candidate.evidence.overlap_smaller);
    let (rms_mm, p95_mm) = residual_numbers(candidate.evidence.euclidean_mm);
    CaseResult {
        case_id: case.id.clone(),
        passed,
        completion: match result.completion {
            Completion::Complete => "complete",
            Completion::Deadline => "deadline",
            Completion::Cancelled => "cancelled",
            Completion::NoUsableSurface => "no_usable_surface",
            Completion::ResourceLimit => "resource_limit",
        },
        input_check_complete: result.input_check == InputCheck::Complete,
        confidence: confidence_name(candidate.confidence),
        reasons: candidate.reasons.iter().map(reason_name).collect(),
        candidate_family: candidate.id.family,
        candidate_proposal: candidate.id.proposal,
        candidate_evidence: candidate_outcomes(&result.candidates),
        rotation_error_deg,
        translation_error_mm,
        maximum_control_landmark_error_mm,
        overlap_smaller,
        rms_mm,
        p95_mm,
        examined_poses: result.work.examined_poses,
        iterations: result.work.iterations,
        elapsed_ms: result.work.elapsed.as_secs_f64() * 1000.0,
    }
}

fn rejection_result(case: &Case, result: &occluview_align::AlignmentSearchResult) -> CaseResult {
    let candidate = &result.candidates[0];
    let Expectation::NotVerified {
        maximum_confidence,
        required_reason,
    } = &case.expectation
    else {
        unreachable!("pose case uses pose_result")
    };
    let expected_reason = expected_reason_name(*required_reason);
    let reasons: Vec<_> = candidate.reasons.iter().map(reason_name).collect();
    let passed = result.completion == Completion::Complete
        && result.input_check == InputCheck::Complete
        && not_verified_candidates_pass(
            result,
            required_confidence(*maximum_confidence),
            expected_reason,
        );
    let (rms_mm, p95_mm) = residual_numbers(candidate.evidence.euclidean_mm);
    CaseResult {
        case_id: case.id.clone(),
        passed,
        completion: match result.completion {
            Completion::Complete => "complete",
            Completion::Deadline => "deadline",
            Completion::Cancelled => "cancelled",
            Completion::NoUsableSurface => "no_usable_surface",
            Completion::ResourceLimit => "resource_limit",
        },
        input_check_complete: result.input_check == InputCheck::Complete,
        confidence: confidence_name(candidate.confidence),
        reasons,
        candidate_family: candidate.id.family,
        candidate_proposal: candidate.id.proposal,
        candidate_evidence: candidate_outcomes(&result.candidates),
        rotation_error_deg: None,
        translation_error_mm: None,
        maximum_control_landmark_error_mm: None,
        overlap_smaller: metric_number(candidate.evidence.overlap_smaller),
        rms_mm,
        p95_mm,
        examined_poses: result.work.examined_poses,
        iterations: result.work.iterations,
        elapsed_ms: result.work.elapsed.as_secs_f64() * 1000.0,
    }
}

fn write_progress(path: &Path, manifest_sha: &[u8; 32], cases: &[CaseResult], passed: bool) {
    let document = json!({
        "schema_version": 1,
        "manifest_sha256": hex_digest(manifest_sha),
        "corpus_sha256": null,
        "all_passed": passed,
        "cases": cases,
    });
    let bytes = serde_json::to_vec_pretty(&document).expect("acceptance results serialize");
    fs::write(path, bytes).expect("acceptance progress writes");
}

#[test]
#[ignore = "requires an explicitly supplied private manifest and corpus"]
fn private_acceptance_manifest() {
    let manifest_path = PathBuf::from(
        std::env::var_os("OCCLUVIEW_PRIVATE_MANIFEST")
            .expect("OCCLUVIEW_PRIVATE_MANIFEST is required"),
    );
    let corpus_path = PathBuf::from(
        std::env::var_os("OCCLUVIEW_PRIVATE_CORPUS")
            .expect("OCCLUVIEW_PRIVATE_CORPUS is required"),
    );
    let result_path = PathBuf::from(
        std::env::var_os("OCCLUVIEW_PRIVATE_RESULT")
            .expect("OCCLUVIEW_PRIVATE_RESULT is required"),
    );
    let manifest_path = manifest_path
        .canonicalize()
        .expect("private manifest is unavailable");
    let corpus_root = corpus_path
        .canonicalize()
        .expect("private corpus is unavailable");
    assert!(corpus_root.is_dir(), "private corpus is not a directory");
    let manifest_bytes = fs::read(&manifest_path).expect("private manifest is unavailable");
    let manifest_sha = sha256(&manifest_bytes);
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .unwrap_or_else(|_| panic!("private manifest is invalid JSON or does not match schema"));
    validate_manifest(&manifest);

    let mut results = Vec::with_capacity(manifest.cases.len());
    let mut source_records = Vec::with_capacity(manifest.cases.len() * 2);
    for case in &manifest.cases {
        let moving_path = resolve_asset(&corpus_root, &case.moving);
        let fixed_path = resolve_asset(&corpus_root, &case.fixed);
        let moving_geometry = load_mesh(&moving_path, case.moving.scale_to_mm);
        let fixed_geometry = load_mesh(&fixed_path, case.fixed.scale_to_mm);
        source_records.push((case.id.clone(), case.moving.file.clone(), moving_geometry.file_sha));
        source_records.push((case.id.clone(), case.fixed.file.clone(), fixed_geometry.file_sha));

        let moving_soup = Soup {
            positions: &moving_geometry.positions,
            indices: &moving_geometry.indices,
            mask: None,
        };
        let fixed_soup = Soup {
            positions: &fixed_geometry.positions,
            indices: &fixed_geometry.indices,
            mask: None,
        };
        let no_constraints: [PointPair; 0] = [];
        let input = AlignmentInput {
            moving: MeshInput {
                soup: moving_soup,
                world_from_local: DAffine3::IDENTITY,
                revision: 1,
            },
            fixed: MeshInput {
                soup: fixed_soup,
                world_from_local: DAffine3::IDENTITY,
                revision: 2,
            },
            // Control landmarks are held out and never constrain the search.
            landmarks: &no_constraints,
            seeds: &[],
        };
        let result = search_alignment(
            &input,
            &search_settings(&case.search),
            &SearchControl::new(
                occluview_align::CancelFlag::new(),
                Duration::from_millis(case.search.wall_limit_ms),
            ),
        )
        .unwrap_or_else(|_| panic!("case {} has non-finite alignment input", case.id));
        assert!(!result.candidates.is_empty(), "case {} returned no candidates", case.id);
        let case_result = match &case.expectation {
            Expectation::Pose { .. } => pose_result(case, &result),
            Expectation::NotVerified { .. } => rejection_result(case, &result),
        };
        let passed = case_result.passed;
        results.push(case_result);
        write_progress(&result_path, &manifest_sha, &results, false);
        assert!(passed, "private acceptance case {} did not meet its manifest contract", case.id);
    }

    let manifest_after = fs::read(&manifest_path).expect("private manifest changed during acceptance");
    assert_eq!(sha256(&manifest_after), manifest_sha, "private manifest changed during acceptance");
    for case in &manifest.cases {
        for asset in [&case.moving, &case.fixed] {
            let path = resolve_asset(&corpus_root, asset);
            let bytes = read_file_bytes(&path).expect("private corpus changed during acceptance");
            let before = source_records
                .iter()
                .find(|(case_id, relative, _)| case_id == &case.id && relative == &asset.file)
                .expect("private source snapshot exists")
                .2;
            assert_eq!(sha256(bytes.as_slice()), before, "private corpus changed during acceptance");
        }
    }

    let corpus_sha = corpus_digest(&manifest_sha, &source_records);
    let final_document = json!({
        "schema_version": 1,
        "manifest_sha256": hex_digest(&manifest_sha),
        "corpus_sha256": corpus_sha,
        "all_passed": true,
        "cases": results,
    });
    fs::write(
        &result_path,
        serde_json::to_vec_pretty(&final_document).expect("acceptance results serialize"),
    )
    .expect("acceptance result writes");
}

#[test]
fn changing_asset_bytes_changes_content_identity_at_same_name_and_size() {
    let before = b"same-name-and-size:AAAA";
    let after = b"same-name-and-size:BBBB";
    assert_eq!(before.len(), after.len());
    let left = corpus_digest(
        &[7; 32],
        &[("case_0123456789ab".into(), "arch/scan.stl".into(), sha256(before))],
    );
    let right = corpus_digest(
        &[7; 32],
        &[("case_0123456789ab".into(), "arch/scan.stl".into(), sha256(after))],
    );
    assert_ne!(left, right);
}

#[test]
fn not_verified_rejects_verified_candidate_after_weak_selected_candidate() {
    use occluview_align::{
        AlignmentCandidate, AlignmentSearchResult, CandidateId, Completion, EvidenceReason,
        InputCheck, RefinementTermination, Rigid, SearchProvenance, SearchWork,
    };

    let result = AlignmentSearchResult {
        candidates: vec![
            AlignmentCandidate {
                id: CandidateId {
                    family: 0,
                    proposal: 0,
                },
                pose: Rigid::IDENTITY,
                confidence: Confidence::Weak,
                evidence: occluview_align::CandidateEvidence::default(),
                reasons: vec![EvidenceReason::InsufficientSupport],
                seeds: Vec::new(),
                refinement: RefinementTermination::NotStarted,
            },
            AlignmentCandidate {
                id: CandidateId {
                    family: 1,
                    proposal: 0,
                },
                pose: Rigid::IDENTITY,
                confidence: Confidence::Verified,
                evidence: occluview_align::CandidateEvidence::default(),
                reasons: vec![EvidenceReason::CloseRival],
                seeds: Vec::new(),
                refinement: RefinementTermination::NotStarted,
            },
        ],
        completion: Completion::Complete,
        input_check: InputCheck::Complete,
        work: SearchWork::default(),
        provenance: SearchProvenance {
            settings: SearchSettings::for_profile(SearchProfile::Standard),
            threshold_set_id: "private-acceptance-test",
            algorithm_version: 1,
            input_revisions: [0, 0],
            masked: [false, false],
        },
    };

    assert!(!not_verified_candidates_pass(
        &result,
        Confidence::Probable,
        "insufficient_support",
    ));
}

/// An otherwise valid pose case; each validation test breaks exactly one field
/// so the failure it asserts comes from that field alone.
fn valid_pose_case(id: &str) -> Case {
    Case {
        id: id.to_string(),
        moving: AssetSpec {
            file: "arch/moving.stl".into(),
            scale_to_mm: 1.0,
        },
        fixed: AssetSpec {
            file: "arch/fixed.stl".into(),
            scale_to_mm: 1.0,
        },
        search: SearchSpec {
            normal_policy: NormalPolicySpec::Match,
            profile: ProfileSpec::Standard,
            top_k: 3,
            reference_regions: RegionSpec::AllEligible,
            wall_limit_ms: 1_000,
        },
        expectation: Expectation::Pose {
            truth: TruthPose {
                rotation_xyzw: [0.0, 0.0, 0.0, 1.0],
                translation_mm: [0.0, 0.0, 0.0],
            },
            transform_tolerance: TransformTolerance {
                rotation_deg: 1.0,
                translation_mm: 1.0,
            },
            control_landmarks: vec![
                ControlLandmark {
                    moving_mm: [0.0, 0.0, 0.0],
                    fixed_mm: [0.0, 0.0, 0.0],
                },
                ControlLandmark {
                    moving_mm: [1.0, 0.0, 0.0],
                    fixed_mm: [1.0, 0.0, 0.0],
                },
                ControlLandmark {
                    moving_mm: [0.0, 1.0, 0.0],
                    fixed_mm: [0.0, 1.0, 0.0],
                },
            ],
            maximum_control_landmark_error_mm: 1.0,
            quality: PoseQualityBounds {
                minimum_confidence: ConfidenceSpec::Probable,
                minimum_overlap_smaller: 0.5,
                maximum_rms_mm: 1.0,
                maximum_p95_mm: 1.0,
            },
        },
    }
}

fn manifest_with(cases: Vec<Case>) -> Manifest {
    Manifest {
        schema_version: 1,
        cases,
    }
}

#[test]
fn a_valid_manifest_passes_validation() {
    validate_manifest(&manifest_with(vec![valid_pose_case("case_0123456789ab")]));
}

#[test]
#[should_panic(expected = "unsupported private manifest schema")]
fn manifest_validation_fails_closed_on_an_unknown_schema() {
    let mut manifest = manifest_with(vec![valid_pose_case("case_0123456789ab")]);
    manifest.schema_version = 2;
    validate_manifest(&manifest);
}

#[test]
#[should_panic(expected = "no cases")]
fn manifest_validation_fails_closed_on_zero_fixtures() {
    validate_manifest(&manifest_with(Vec::new()));
}

#[test]
#[should_panic(expected = "case ID")]
fn manifest_validation_fails_closed_on_a_malformed_case_id() {
    validate_manifest(&manifest_with(vec![valid_pose_case("case_0123456789AB")]));
}

#[test]
#[should_panic(expected = "duplicate case ID")]
fn manifest_validation_fails_closed_on_duplicate_case_ids() {
    validate_manifest(&manifest_with(vec![
        valid_pose_case("case_0123456789ab"),
        valid_pose_case("case_0123456789ab"),
    ]));
}

#[test]
#[should_panic(expected = "traversal-free")]
fn manifest_validation_rejects_a_traversal_asset_path() {
    let mut case = valid_pose_case("case_0123456789ab");
    case.moving.file = "../scan.stl".into();
    validate_manifest(&manifest_with(vec![case]));
}

#[test]
#[should_panic(expected = "finite and positive")]
fn manifest_validation_rejects_a_non_positive_asset_scale() {
    let mut case = valid_pose_case("case_0123456789ab");
    case.fixed.scale_to_mm = 0.0;
    validate_manifest(&manifest_with(vec![case]));
}

#[test]
#[should_panic(expected = "3 to 128")]
fn manifest_validation_requires_three_independent_landmarks() {
    let mut case = valid_pose_case("case_0123456789ab");
    if let Expectation::Pose {
        control_landmarks, ..
    } = &mut case.expectation
    {
        control_landmarks.truncate(2);
    }
    validate_manifest(&manifest_with(vec![case]));
}

#[test]
#[should_panic(expected = "below verified")]
fn manifest_validation_caps_a_not_verified_ceiling_below_verified() {
    let mut case = valid_pose_case("case_0123456789ab");
    case.expectation = Expectation::NotVerified {
        maximum_confidence: ConfidenceSpec::Verified,
        required_reason: ExpectedReason::InsufficientSupport,
    };
    validate_manifest(&manifest_with(vec![case]));
}

#[test]
fn asset_paths_must_be_relative_and_traversal_free() {
    assert!(valid_relative_file("arch/scan.stl"));
    assert!(valid_relative_file("a/b/c/scan.stl"));
    assert!(!valid_relative_file("/abs/scan.stl"));
    assert!(!valid_relative_file("../scan.stl"));
    assert!(!valid_relative_file("arch/../../scan.stl"));
    assert!(!valid_relative_file("./scan.stl"));
    assert!(!valid_relative_file("arch\\scan.stl"));
    assert!(!valid_relative_file("C:/scan.stl"));
    assert!(!valid_relative_file(""));
}

/// A private temporary directory removed when this guard drops.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let base = std::env::temp_dir().join(format!(
            "occluview-private-acceptance-{}-{nanos}",
            std::process::id(),
        ));
        for attempt in 0..1024_u32 {
            let candidate = if attempt == 0 {
                base.clone()
            } else {
                base.with_extension(format!("retry-{attempt}"))
            };
            match std::fs::create_dir(&candidate) {
                Ok(()) => return Self(candidate),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create a private fixture directory: {error}"),
            }
        }
        panic!("cannot create a unique private fixture directory");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[should_panic(expected = "unavailable")]
fn a_missing_private_asset_fails_closed() {
    let root = TempDir::new();
    let asset = AssetSpec {
        file: "missing.stl".into(),
        scale_to_mm: 1.0,
    };
    let _ = resolve_asset(&root.0, &asset);
}
