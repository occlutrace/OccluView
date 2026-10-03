//! Reviewable registration results in physical millimetres.
//!
//! Candidate poses are corrections of the snapshotted authored moving frame.
//! Acceptance composes `pose * A0` once. Missing evidence never denotes zero.
//! Confidence describes geometric evidence, independently of operator acceptance.

use crate::{FitRejection, Rigid, Soup};
use glam::{DAffine3, DVec3};
use std::time::Duration;

/// Evidence class; accepting a pose does not change this class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Confidence {
    /// Complete independent geometric verification.
    Verified,
    /// Useful supported geometry with remaining uncertainty.
    Probable,
    /// Supported geometry permits competing motions.
    Ambiguous,
    /// Insufficient independent evidence.
    #[default]
    Weak,
}

/// Why the search stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    /// All scheduled work completed.
    Complete,
    /// A deterministic work allowance was exhausted.
    WorkLimit,
    /// The wall deadline was reached.
    Deadline,
    /// The caller requested cancellation.
    Cancelled,
    /// No eligible nondegenerate triangles were available.
    NoUsableSurface,
    /// The representation exceeded available resources.
    ResourceLimit,
}

/// Correspondence orientation, independent of rotation proposals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalPolicy {
    /// Same facing normals.
    #[default]
    Match,
    /// Opposite facing normals.
    Opposed,
    /// Ignore facing sign.
    Unsigned,
}

/// Why a measurement is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingReason {
    /// No counterparts.
    NoSupport,
    /// Insufficient independent samples.
    TooFewSamples,
    /// The geometric constraint is deficient.
    Degenerate,
    /// This quantity has not been evaluated.
    NotEvaluated,
    /// Its evaluation was interrupted.
    Interrupted,
}

/// Numeric field and side for an invalid input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputField {
    /// Moving position scalar.
    MovingPositions,
    /// Fixed position scalar.
    FixedPositions,
    /// Moving authored affine scalar.
    MovingAffine,
    /// Fixed authored affine scalar.
    FixedAffine,
    /// Seed quaternion scalar.
    SeedRotation,
    /// Seed translation scalar.
    SeedTranslation,
    /// Moving landmark coordinate.
    LandmarkMoving,
    /// Fixed landmark coordinate.
    LandmarkFixed,
    /// Landmark normal scalar.
    LandmarkNormals,
    /// Search radius.
    InfluenceRadius,
    /// Overlap prior.
    OverlapPrior,
}

/// The only numeric hard error; indices address scalars in the named field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignmentInputError {
    /// Encountered a NaN or infinity, including in excluded geometry.
    NonFinite {
        /// Input field containing the value.
        field: InputField,
        /// Scalar index within that field (seeds/normals are flattened).
        index: usize,
    },
}

/// An explicit measurement or an explicit absence of evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Metric<T> {
    /// Complete measurement on the documented population.
    Measured(T),
    /// No plausible numeric value is supplied.
    Missing(MissingReason),
}
impl<T> Default for Metric<T> {
    fn default() -> Self {
        Self::Missing(MissingReason::NotEvaluated)
    }
}

/// Input validation coverage; incomplete checking forbids high confidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputCheck {
    /// All supplied numeric fields have been checked.
    Complete,
    /// Validation ended before the remaining scalars were read.
    Partial {
        /// Scalars checked in stable input order.
        checked: usize,
        /// Total scalar ceiling, saturated if necessary; optional landmark
        /// normals are conservatively included when their shape was not inspected.
        total: usize,
    },
}
/// Stable proposal family (order defines family ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedOrigin {
    /// Current placement.
    Start,
    /// Operator landmarks.
    Landmarks,
    /// Eligible region frame.
    MaskFrame,
    /// Authored relative feature sign.
    FeatureSame,
    /// Reversed relative feature sign.
    FeatureOpposed,
    /// Proper principal frame rotation.
    PrincipalFrame,
    /// Rotation grid and patch translation.
    GridPatch,
    /// Descriptor-free congruent base.
    CongruentBasis,
}

/// Stable family and proposal ordinals; never a random identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidateId {
    /// Origin family ordinal.
    pub family: u8,
    /// Proposal ordinal within that family.
    pub proposal: u32,
}
/// Local termination is independent of pose confidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RefinementTermination {
    /// No local solve ran.
    #[default]
    NotStarted,
    /// No improving trial remained.
    Stationary,
    /// The local step became small.
    StepSmall,
    /// Iteration allowance exhausted.
    IterationLimit,
    /// No usable correspondence set.
    NoCorrespondences,
    /// The numerical constraint was deficient.
    Singular,
    /// A trial was invalid or out of bounds.
    NumericalTrialRejected,
    /// Cancelled local work.
    Cancelled,
    /// Deadline ended local work.
    Deadline,
}

/// Explanations retained with the pose; none authorize scene edits.
#[derive(Clone, Debug, PartialEq)]
pub enum EvidenceReason {
    /// Not enough common support.
    InsufficientSupport,
    /// Geometric motion is unconstrained.
    UnobservableMotion,
    /// A competing basin remains.
    CloseRival,
    /// Requested normal policy is not supported.
    PolicyConflict,
    /// Coherent edge ratios suggest physical scale or deformation.
    SuspectedUnitsOrDeformation,
    /// Competing fitting surfaces remain.
    ShellAlternatives,
    /// Only unsupported border projections were found.
    BorderOnly,
    /// No reliable surface normals.
    MissingNormals,
    /// The representation is incomplete or unsuitable for this solve.
    PartialGeometry,
    /// Some numeric input is not yet checked.
    UnvalidatedInput,
    /// Local optimization stopped improving.
    Stalled,
    /// Work or time ended before all evidence was evaluated.
    BudgetExhausted,
    /// Finite zero seed quaternion was replaced with identity.
    InvalidSeed,
    /// Landmark fitting did not determine a pose.
    LandmarkRejected(FitRejection),
    /// A finite legacy refusal retained its last checkpoint.
    LegacyRejected(FitRejection),
    /// Independent holdout/rival verification has not been established.
    UniquenessNotEstablished,
    /// Malformed or degenerate triangles were omitted.
    InvalidTopology,
    /// A numeric setting was clamped, with its original and effective values.
    SettingClamped {
        /// Numeric setting name.
        field: InputField,
        /// Supplied finite value.
        original: f64,
        /// Effective finite value.
        effective: f64,
    },
}

/// Residual statistics on the stated population, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResidualSummary {
    /// Area-weighted median.
    pub median: f64,
    /// Area-weighted RMS.
    pub rms: f64,
    /// Area-weighted 95th percentile.
    pub p95: f64,
}
/// Evidence populations are explicit; absent quantities never become zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CandidateEvidence {
    /// Moving/fixed eligible areas; unavailable until full area accounting.
    pub eligible_area_mm2: Metric<[f64; 2]>,
    /// Moving/fixed area actually queried. Unqueried eligible area contributes
    /// no support; role evidence is never extrapolated to whole surface area.
    pub queried_population_area_mm2: Metric<[f64; 2]>,
    /// Compatible queried common area per direction, absent without normals.
    pub policy_compatible_area_mm2: Metric<[Option<f64>; 2]>,
    /// Directional area fractions within .2 mm, denominators eligible areas.
    pub coverage_02: Metric<[f64; 2]>,
    /// Directional area fractions within .5 mm, denominators eligible areas.
    pub coverage_05: Metric<[f64; 2]>,
    /// Lesser matched directional area in square millimetres.
    pub common_area_mm2: Metric<f64>,
    /// Common area divided by smaller eligible area.
    pub overlap_smaller: Metric<f64>,
    /// Retained fraction of smaller eligible area.
    pub trim_fraction: Metric<f64>,
    /// Eligible common-region area used for residuals.
    pub common_region_area_mm2: Metric<f64>,
    /// Independent common-region Euclidean residuals.
    pub euclidean_mm: Metric<ResidualSummary>,
    /// Independent common-region plane residuals.
    pub plane_mm: Metric<ResidualSummary>,
    /// Area within .2 divided by area within .5 on common region.
    pub inlier_ratio: Metric<f64>,
    /// Lesser directional count of occupied 1 mm common cells.
    pub effective_cells: Metric<u32>,
    /// Compatible normal area divided by queried common area.
    pub orientation_fraction: Metric<f64>,
    /// Back-projection passing common area divided by queried common area.
    pub reciprocal_fraction: Metric<f64>,
    /// Undamped area-weighted common plane information eigenvalues.
    pub info_eigenvalues: Metric<[f64; 6]>,
    /// Unconstrained normalized motion eigenvectors.
    pub weak_twists: Vec<[f64; 6]>,
    /// Relative score gap to fully evaluated distinct rival.
    pub rival_gap: Metric<f64>,
    /// Holdout RMS divided by max(training RMS,.02 mm).
    pub holdout_ratio: Metric<f64>,
    /// Maximum jackknife centroid/rotation drift in mm/degrees.
    pub jackknife_mm_deg: Metric<[f64; 2]>,
    /// Common-region geometric ranking objective.
    pub score: Metric<f64>,
    /// All mandatory independent evidence passes completed.
    pub verification_complete: bool,
    /// Transitional vertex-based diagnostics; never substituted for area evidence.
    pub legacy_report: Option<crate::IcpReport>,
}
/// Search domain and default resource allowance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchProfile {
    /// Global interactive search.
    #[default]
    Standard,
    /// Longer global search.
    Extended,
    /// Local neighborhood only; never Verified.
    Local,
}

/// Meaning of eligible registration geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RegionPolicy {
    /// All geometry remaining after exclusion masks.
    #[default]
    AllEligible,
    /// Operator reference region; outside remains measurable.
    ReferenceRoi,
}

/// Deterministic work and memory ceilings.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchBudget {
    /// Total nearest queries.
    pub query_calls: u64,
    /// Total scalar triangle distance evaluations.
    pub triangle_tests: u64,
    /// Total descriptor and point-pair evaluations.
    pub point_pair_tests: u64,
    /// Total connected patch graph visits.
    pub patch_edge_visits: u64,
    /// Additional resident job allocations.
    pub memory_bytes: usize,
}

/// Finite settings are clamped with recorded evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchSettings {
    /// Correspondence facing rule.
    pub normal_policy: NormalPolicy,
    /// Global or local domain.
    pub profile: SearchProfile,
    /// Returned pose cap, effective range 1 through 5.
    pub top_k: usize,
    /// Registration population definition.
    pub reference_regions: RegionPolicy,
    /// Radius, effective clamp .5 through 4 mm.
    pub influence_radius_mm: f64,
    /// Optional retained-area ceiling, clamp .01 through 1.
    pub overlap_prior: Option<f64>,
    /// Deterministic work ceilings.
    pub work_budget: SearchBudget,
    /// Wall allowance including validation.
    pub wall_limit: Duration,
}

/// Work and coverage of one deterministic proposal family.
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyEvidence {
    /// Stable proposal origin.
    pub family: SeedOrigin,
    /// This family is enabled for the profile/request.
    pub enabled: bool,
    /// Attempted scoring passes, including cheap proxy passes.
    pub attempted: u64,
    /// Completed scoring passes.
    pub scored: u64,
    /// Basins retained under this primary family id.
    pub retained: u32,
    /// Fully evaluated grid rotations; zero for non-grid families.
    pub rotations_attempted: u32,
    /// Grid translation placements attempted before accurate rescoring.
    pub translations_attempted: u64,
    /// Charged local point-pair work.
    pub point_pair_tests: u64,
    /// Optional family's explicit local point-pair ceiling.
    pub local_point_pair_limit: Option<u64>,
    /// Local/global interruption, or none after a completed schedule.
    pub interruption: Option<Completion>,
    /// The configured family schedule finished. An interrupted grid prefix
    /// does not have the complete grid's covering certificate.
    pub complete: bool,
}

/// Completed work; elapsed time is excluded from determinism comparisons.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SearchWork {
    /// Family schedules, including interruption and actual grid coverage.
    pub families: Vec<FamilyEvidence>,
    /// Examined proposal count.
    pub examined_poses: u64,
    /// Returned finite pose count.
    pub retained_poses: u32,
    /// Completed local iterations.
    pub iterations: u64,
    /// Charged nearest calls.
    pub query_calls: u64,
    /// Charged distance tests.
    pub triangle_tests: u64,
    /// Charged point-pair tests.
    pub point_pair_tests: u64,
    /// Charged graph edge visits.
    pub patch_edge_visits: u64,
    /// Charged numeric/topology/bucket/cell operations.
    pub preprocessing_operations: u64,
    /// Conservative largest additional allocation reservation.
    pub peak_memory_bytes: u64,
    /// Elapsed wall time.
    pub elapsed: Duration,
    /// Scheduled evidence not completed.
    pub unfinished_stages: Vec<&'static str>,
}

/// Versioned interpretation of a result, independent of scene authority.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchProvenance {
    /// Effective ordinary-operation ceiling, including topology and bucket work.
    pub operation_limit: u64,
    /// Effective settings after validation/clamping and profile ceilings;
    /// absent if interruption precedes numeric validation.
    pub effective_settings: Option<SearchSettings>,
    /// Version of physical evidence thresholds; absent verifier remains explicit.
    pub threshold_set_id: &'static str,
    /// Version of rotation recipe and frozen prefix ordering.
    pub grid_recipe_id: &'static str,
    /// Algorithm contract version.
    pub algorithm_version: u32,
    /// Caller moving/fixed revision tokens.
    pub input_revisions: [u64; 2],
    /// Fixed PRNG seed.
    pub seed: u64,
    /// Effective facing policy.
    pub normal_policy: NormalPolicy,
    /// Effective eligibility policy.
    pub reference_regions: RegionPolicy,
    /// Whether each input carries exclusions.
    pub masked: [bool; 2],
    /// Area fraction covered by complete representation accounting.
    pub processed_area_fraction: Metric<[f64; 2]>,
    /// Effective work allowance.
    pub budget: SearchBudget,
}

/// Finite proper rigid correction of the snapshotted moving input frame.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentCandidate {
    /// Stable origin identifier.
    pub id: CandidateId,
    /// Correction; absolute scene affine is pose times authored affine.
    pub pose: Rigid,
    /// Independent geometric confidence.
    pub confidence: Confidence,
    /// Measured or explicitly missing evidence.
    pub evidence: CandidateEvidence,
    /// Uncertainty and interruption explanations.
    pub reasons: Vec<EvidenceReason>,
    /// Proposal origins.
    pub seeds: Vec<SeedOrigin>,
    /// Local solve termination.
    pub refinement: RefinementTermination,
}

/// One to five finite candidates for every numerically finite request.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentSearchResult {
    /// Ranked reviewable poses.
    pub candidates: Vec<AlignmentCandidate>,
    /// Terminal reason.
    pub completion: Completion,
    /// Validation coverage.
    pub input_check: InputCheck,
    /// Completed work and unfinished evidence.
    pub work: SearchWork,
    /// Reproducible interpretation.
    pub provenance: SearchProvenance,
}

impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            query_calls: 8_000_000,
            triangle_tests: 80_000_000,
            point_pair_tests: 32_000_000,
            patch_edge_visits: 2_000_000,
            memory_bytes: 256 * 1024 * 1024,
        }
    }
}
impl SearchSettings {
    /// Construct the complete preset, including its wall and work allowances.
    /// Callers can then lower individual limits explicitly. Merely changing
    /// `profile` on an existing settings value preserves its explicit limits.
    pub fn for_profile(profile: SearchProfile) -> Self {
        let (wall, queries, triangles, pairs, edges) = match profile {
            SearchProfile::Standard => (10, 8_000_000, 80_000_000, 32_000_000, 2_000_000),
            SearchProfile::Extended => (30, 20_000_000, 240_000_000, 128_000_000, 8_000_000),
            SearchProfile::Local => (2, 1_000_000, 10_000_000, 32_000_000, 2_000_000),
        };
        Self {
            normal_policy: NormalPolicy::Match,
            profile,
            top_k: 5,
            reference_regions: RegionPolicy::AllEligible,
            influence_radius_mm: 2.,
            overlap_prior: None,
            wall_limit: Duration::from_secs(wall),
            work_budget: SearchBudget {
                query_calls: queries,
                triangle_tests: triangles,
                point_pair_tests: pairs,
                patch_edge_visits: edges,
                memory_bytes: 256 * 1024 * 1024,
            },
        }
    }
}
impl Default for SearchSettings {
    fn default() -> Self {
        Self::for_profile(SearchProfile::Standard)
    }
}
/// A borrowed mesh and its finite authored frame; geometry is never mutated.
#[derive(Clone, Copy, Debug)]
pub struct MeshInput<'a> {
    /// Triangle geometry in local coordinates.
    pub soup: Soup<'a>,
    /// Authored local-to-world affine, including scale/shear if present.
    pub world_from_local: DAffine3,
    /// Caller-owned geometry/pose/mask revision token.
    pub revision: u64,
}
/// Landmark coordinates in the corresponding mesh local frames.
#[derive(Clone, Copy, Debug)]
pub struct PointPair {
    /// Moving landmark.
    pub moving_local: DVec3,
    /// Fixed landmark.
    pub fixed_local: DVec3,
    /// Optional moving/fixed local normals.
    pub normals_local: Option<[DVec3; 2]>,
}
/// Complete immutable registration input. Empty seed list starts at identity.
#[derive(Clone, Copy, Debug)]
pub struct AlignmentInput<'a> {
    /// Moving local mesh and authored frame.
    pub moving: MeshInput<'a>,
    /// Fixed local mesh and authored frame.
    pub fixed: MeshInput<'a>,
    /// Optional operator constraints; unavailable fits remain warnings.
    pub landmarks: &'a [PointPair],
    /// Optional rigid corrections of the authored moving frame.
    pub seeds: &'a [Rigid],
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    #[test]
    fn profiles_set_effective_deadline_and_work_allowance() {
        for (profile, seconds, queries, triangles, pairs) in [
            (
                SearchProfile::Standard,
                10,
                8_000_000,
                80_000_000,
                32_000_000,
            ),
            (
                SearchProfile::Extended,
                30,
                20_000_000,
                240_000_000,
                128_000_000,
            ),
            (SearchProfile::Local, 2, 1_000_000, 10_000_000, 32_000_000),
        ] {
            let mut settings = SearchSettings::for_profile(profile);
            let control = crate::SearchControl::new(crate::CancelFlag::new(), settings.wall_limit);
            let limits = control.geometry_control(&settings).limits();
            assert_eq!(settings.wall_limit, Duration::from_secs(seconds));
            assert_eq!(
                (
                    limits.query_calls,
                    limits.triangle_tests,
                    limits.point_pair_tests
                ),
                (queries, triangles, pairs)
            );
            settings.work_budget.query_calls = 7;
            assert_eq!(control.geometry_control(&settings).limits().query_calls, 7);
        }
    }
}
