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
    /// The wall deadline was reached.
    Deadline,
    /// The caller requested cancellation.
    Cancelled,
    /// No eligible nondegenerate triangles were available.
    NoUsableSurface,
    /// An input is larger than the search admits.
    ResourceLimit,
}

/// Correspondence orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalPolicy {
    /// Same facing normals.
    #[default]
    Match,
    /// Opposite facing normals.
    Opposed,
    /// Either facing; both are searched.
    Unsigned,
}

/// Why a measurement is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingReason {
    /// No counterparts.
    NoSupport,
    /// The geometric constraint is deficient.
    Degenerate,
    /// This quantity has not been evaluated.
    NotEvaluated,
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
        /// Total scalar count, saturated if necessary.
        total: usize,
    },
}

/// Where a candidate's motion came from (order defines family ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedOrigin {
    /// Current placement, or a caller's seed.
    Start,
    /// Operator landmarks.
    Landmarks,
    /// Look-alike places, surfaces facing the same way.
    FeatureSame,
    /// Look-alike places, surfaces facing opposite ways.
    FeatureOpposed,
    /// Principal frames of the two surfaces.
    PrincipalFrame,
}

/// Stable family and proposal ordinals; never a random identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidateId {
    /// Origin family ordinal.
    pub family: u8,
    /// Proposal ordinal within that family.
    pub proposal: u32,
}

/// How the seating of a pose ended; independent of pose confidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RefinementTermination {
    /// No seating ran.
    #[default]
    NotStarted,
    /// The step became small.
    StepSmall,
    /// The rounds ran out.
    IterationLimit,
    /// No usable correspondence set.
    NoCorrespondences,
    /// The numerical constraint was deficient.
    Singular,
    /// A step left the finite range.
    NumericalTrialRejected,
    /// Cancelled.
    Cancelled,
    /// The deadline ended it.
    Deadline,
}

/// Explanations retained with the pose; none authorize scene edits.
#[derive(Clone, Debug, PartialEq)]
pub enum EvidenceReason {
    /// Not enough common support.
    InsufficientSupport,
    /// Geometric motion is unconstrained.
    UnobservableMotion,
    /// A competing pose remains.
    CloseRival,
    /// The common surfaces do not face the way the normal policy asks.
    PolicyConflict,
    /// The two surfaces differ in size: other units, or a deformed object.
    SuspectedUnitsOrDeformation,
    /// Triangles of a scan do not face one way; its normals are unreliable.
    MissingNormals,
    /// Some numeric input is not yet checked.
    UnvalidatedInput,
    /// Time ended before all evidence was evaluated.
    BudgetExhausted,
    /// Finite zero seed quaternion was replaced with identity.
    InvalidSeed,
    /// The operator's landmarks did not determine a pose.
    LandmarkRejected(FitRejection),
    /// The pose has not been shown to be the only one.
    UniquenessNotEstablished,
    /// Malformed or degenerate triangles were omitted.
    InvalidTopology,
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

/// What one pose is worth. The common region is the surface of either scan
/// that lies within half a millimetre of the other and faces the way the
/// normal policy asks; absent quantities never become zero.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent evidence passes have separate completeness invariants"
)]
pub struct CandidateEvidence {
    /// Moving/fixed eligible areas.
    pub eligible_area_mm2: Metric<[f64; 2]>,
    /// Whether each scan was read on its triangles (true) or on an even
    /// cloud of them (false), separately for moving and fixed.
    pub original_surface_exact: [bool; 2],
    /// Moving/fixed share of eligible area within 0.2 mm of the other scan.
    pub coverage_02: Metric<[f64; 2]>,
    /// Moving/fixed share of eligible area within 0.5 mm of the other scan.
    pub coverage_05: Metric<[f64; 2]>,
    /// Lesser of the two directions' common area, in square millimetres.
    pub common_area_mm2: Metric<f64>,
    /// Common area divided by the smaller eligible area.
    pub overlap_smaller: Metric<f64>,
    /// Distances over the common region, both directions.
    pub euclidean_mm: Metric<ResidualSummary>,
    /// Common area within 0.2 mm divided by common area.
    pub inlier_ratio: Metric<f64>,
    /// Lesser directional count of occupied 1 mm cells of the common region.
    pub effective_cells: Metric<u32>,
    /// Common area divided by all area within 0.5 mm, whichever way it faces.
    pub orientation_fraction: Metric<f64>,
    /// Lesser directional common area divided by the greater.
    pub reciprocal_fraction: Metric<f64>,
    /// Undamped area-weighted plane information of the common region, as
    /// rising eigenvalues; rotations are scaled by the region's radius.
    pub info_eigenvalues: Metric<[f64; 6]>,
    /// The eigenvectors paired with `info_eigenvalues`.
    pub info_eigenvectors: Metric<[[f64; 6]; 6]>,
    /// Motions the common region does not constrain.
    pub weak_twists: Vec<[f64; 6]>,
    /// Share by which this pose's score exceeds its best rival's.
    pub rival_gap: Metric<f64>,
    /// RMS over probes that did not seat the pose, divided by the RMS over
    /// those that did (at least 0.02 mm).
    pub holdout_ratio: Metric<f64>,
    /// Largest drift of the common region's centre, in millimetres, and
    /// largest turn, in degrees, when each eighth of the surface is left out.
    pub jackknife_mm_deg: Metric<[f64; 2]>,
    /// Common area of both directions, each place counted less the farther
    /// it stands off; the ranking objective.
    pub score: Metric<f64>,
    /// Share by which the moving surface would have to grow to lie better on
    /// the fixed one; two scans of one object read zero.
    pub size_trend: Metric<f64>,
    /// All evidence passes completed.
    pub verification_complete: bool,
    /// Probes that did not seat the pose were read in both directions.
    pub holdout_complete: bool,
    /// All eight leave-one-out seatings completed.
    pub jackknife_complete: bool,
    /// Every rival was seated and read to the end.
    pub rival_probes_complete: bool,
}

/// Search domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchProfile {
    /// Global search.
    #[default]
    Standard,
    /// The neighbourhood of the given placements only; never Verified.
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

/// What the caller may choose about a search.
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
    /// Wall allowance including validation.
    pub wall_limit: Duration,
}

/// Motions of one origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyEvidence {
    /// The origin.
    pub family: SeedOrigin,
    /// Motions examined.
    pub examined: u32,
    /// Candidates returned under this origin.
    pub retained: u32,
}

/// Completed work; elapsed time is excluded from determinism comparisons.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SearchWork {
    /// Motions examined and candidates returned, per origin.
    pub families: Vec<FamilyEvidence>,
    /// Examined motion count.
    pub examined_poses: u64,
    /// Returned pose count.
    pub retained_poses: u32,
    /// Completed seating rounds.
    pub iterations: u64,
    /// Elapsed wall time.
    pub elapsed: Duration,
}

/// Versioned interpretation of a result, independent of scene authority.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchProvenance {
    /// Settings in effect, after clamping.
    pub settings: SearchSettings,
    /// Version of the physical evidence thresholds.
    pub threshold_set_id: &'static str,
    /// Algorithm contract version.
    pub algorithm_version: u32,
    /// Caller moving/fixed revision tokens.
    pub input_revisions: [u64; 2],
    /// Whether each input carries exclusions.
    pub masked: [bool; 2],
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
    /// Where the motion came from.
    pub seeds: Vec<SeedOrigin>,
    /// How its seating ended.
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
    /// Completed work.
    pub work: SearchWork,
    /// Reproducible interpretation.
    pub provenance: SearchProvenance,
}

impl SearchSettings {
    /// The preset of a profile, including its wall allowance.
    pub fn for_profile(profile: SearchProfile) -> Self {
        Self {
            normal_policy: NormalPolicy::Match,
            profile,
            top_k: 5,
            reference_regions: RegionPolicy::AllEligible,
            wall_limit: Duration::from_secs(match profile {
                SearchProfile::Standard => 10,
                SearchProfile::Local => 5,
            }),
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
    /// Optional operator constraints; an unusable set is reported, not fatal.
    pub landmarks: &'a [PointPair],
    /// Optional rigid corrections of the authored moving frame.
    pub seeds: &'a [Rigid],
}
