//! Bounded coarse pose search for scan registration.

use glam::{DMat3, DQuat, DVec3, EulerRot};
use std::collections::BTreeMap;

use crate::pairs::FitRejection;
use crate::sample::{bounds_of, sample_vertices, vertex_at};
use crate::{Rigid, Soup};

use super::icp_overlap::{
    common_support_coverage, directional_forward_evidence_is_sufficient, fixed_surface_is_smaller,
    reciprocal_evidence, reciprocal_evidence_is_usable, support_coverage_is_sufficient,
    ReciprocalSummary,
};
use super::icp_solve::{accumulate, correspondences, run_level, summarize, trim};
use super::icp_unique::principal_axes;
use super::{
    turn_between, Level, Orientation, RefineSettings, Summary, COARSE_TIE_SEATED,
    MIN_CORRESPONDENCES, STALL_IMPROVEMENT,
};

#[derive(Clone, Copy)]
pub(super) struct CoarseCandidate {
    pub(super) rigid: Rigid,
    pub(super) summary: Summary,
    pub(super) reciprocal: Option<ReciprocalSummary>,
    pub(super) shift: f64,
    pub(super) component: Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) struct StartPose {
    pub(super) rigid: Rigid,
    pub(super) coarse_shift: f64,
}

struct GlobalSeedContext<'a> {
    samples: &'a [u32],
    moving_anchor: DVec3,
    moving_center: DVec3,
    orientations: &'a [DQuat],
    max_shift: f64,
}

const COARSE_TIE_RELATIVE_RMS: f64 = 0.02;
const COARSE_TIE_COVERAGE: f64 = 0.02;
const COARSE_RECIPROCAL_ADVANTAGE: f64 = 0.01;
const COARSE_RECIPROCAL_RMS_FACTOR: f64 = 1.25;
/// Fraction of the incumbent's forward coverage a candidate must keep to win on
/// residual alone. Relative, not absolute: an absolute allowance would lift the
/// 1% search floor to an effective 3%, the regime where a small patch competes
/// with the operator's own start.
const COARSE_COVERAGE_KEEP_FRACTION: f64 = 0.9;
const COARSE_TIE_SHIFT_MM: f64 = 0.5;
/// How far two coarse hypotheses may move the scan and still count as one
/// answer, as a fraction of the correspondence radius.
///
/// Half the radius: hypotheses that place the surface inside the distance the
/// search itself treats as "the same place" are one seating seen twice, not two
/// answers to choose between. See [`poses_are_distinct`].
const COARSE_ANSWER_TOLERANCE_FRACTION: f64 = 0.5;
const COARSE_MAX_SHIFT_FACTOR: f64 = 4.0;
/// Samples used by the bounded global seed search. This is intentionally much
/// smaller than either ICP level: it locates a plausible patch, then the
/// existing full-resolution objective and reciprocal guard decide whether it
/// is real.
const GLOBAL_SEED_SAMPLE_BUDGET: usize = 512;
/// Surface anchors tried by the global seed search. A triangle representative
/// is a usable surface point even when the moving scan is only a crop of a
/// larger connected scan.
const GLOBAL_ANCHOR_BUDGET: usize = 384;
/// Reciprocal representatives for the cheap seed ranking when the fixed
/// surface is the smaller side. The retained candidates are re-scored with
/// the full reciprocal witness before they can enter coarse comparison.
const GLOBAL_SEED_SUPPORT_SAMPLE_BUDGET: usize = 64;
/// Full-resolution candidates retained after the cheap anchor pass.
const GLOBAL_CANDIDATE_BUDGET: usize = 16;
/// Iterations spent locally refining each global seed before comparing seeds.
/// The normal refinement pass still runs afterwards with the operator's full
/// budget; this bounded pass only prevents a smooth wrong patch from winning
/// on its unrefined anchor residual.
const GLOBAL_SEED_REFINE_ITERATIONS: u32 = 8;
/// Do not pay for a global anchor sweep when the current pose already explains
/// most of the moving samples. A weak local overlap still triggers recovery.
const GLOBAL_SEED_MIN_SUPPORT_COVERAGE: f64 = 0.5;
/// A high-coverage but high-residual local patch is still accidental evidence.
/// The threshold is a fraction of the operator's correspondence radius, so it
/// scales with the same physical tolerance instead of a mesh-size-independent
/// magic number.
const GLOBAL_SEED_MAX_START_RMS_FRACTION: f64 = 0.25;

/// Prefer a centered coarse hypothesis when it clearly explains more of the
/// same surface than the caller's rough pose.
///
/// A point-to-plane step cannot see translation tangent to a locally flat patch.
/// When two scans are merely close and shifted sideways, it can therefore settle
/// on the edge it first touched. The bounding-box hypothesis is only a candidate:
/// it wins when the same correspondence objective improves without losing
/// coverage, so partial scans and adjacent anatomy keep the explicit start.
pub(super) fn choose_start_pose(level: &Level<'_>) -> Result<StartPose, FitRejection> {
    if level.samples.is_empty() || level.cancel.is_cancelled() {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    }
    let Some((moving_center, moving_extent)) = bounds_of(level.moving) else {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    };
    let seed_samples = sample_vertices(level.moving, GLOBAL_SEED_SAMPLE_BUDGET);
    let moving_anchor = sampled_centroid(level.moving, &seed_samples).unwrap_or(moving_center);
    let score = |pose: Rigid| score_candidate(level, pose);
    let mut candidates = Vec::new();
    // Remember whether the operator's pose itself produced usable evidence.
    // A centered component seed can see a neighbouring surface through a
    // generous influence radius even when the requested pose has no hit at
    // all. That accidental seed must not decide whether global recovery runs,
    // or a small partial crop stays sideways.
    let (start_candidate, start_has_strong_evidence) = coarse_start_candidate(level, moving_center);
    if let Some(candidate) = start_candidate {
        candidates.push(candidate);
    }

    // A component centre is a global coarse hypothesis, not a local radius
    // clamp. The visible influence radius controls correspondence distance;
    // the component bounds provide the bounded set of plausible poses. This
    // is what lets a rough side-by-side placement recover without making the
    // ICP kernel walk an unbounded translation grid.
    let orientation_deltas = coarse_orientation_deltas();
    let max_coarse_shift = moving_extent.max(1.0) * COARSE_MAX_SHIFT_FACTOR
        + level.settings.influence_radius_mm.abs() * COARSE_MAX_SHIFT_FACTOR;
    for (component_index, &(fixed_min, fixed_max)) in
        level.fixed.component_bounds().iter().enumerate()
    {
        let fixed_center = (fixed_min + fixed_max) * 0.5;
        for (delta_index, &delta) in orientation_deltas.iter().enumerate() {
            // `Inverted` is an explicit winding-repair escape hatch. Do not
            // manufacture an upside-down physical pose merely because that
            // would make an otherwise same-facing surface satisfy the setting.
            // The identity candidate still permits a genuinely inverted fixed
            // mesh to refine; global orientation recovery belongs to the normal
            // matching modes.
            if level.settings.orientation == Orientation::Inverted && delta_index != 0 {
                continue;
            }
            // Corrections are expressed in world space. Centering around the
            // fixed component makes the translation candidate explicit; the
            // final displacement guard below expands to cover that proven
            // coarse move instead of rejecting it as a runaway.
            let rotation = delta * level.start.rotation;
            let candidate = Rigid::new(rotation, fixed_center - rotation * moving_center);
            let shift =
                (candidate.apply(moving_center) - level.start.apply(moving_center)).length();
            if !shift.is_finite() || shift > max_coarse_shift {
                continue;
            }
            let Some((summary, reciprocal)) = score(candidate) else {
                continue;
            };
            candidates.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal,
                shift,
                component: Some(component_index),
            });
        }
    }

    let needs_global_seed = !start_has_strong_evidence
        || candidates.is_empty()
        || candidates
            .iter()
            .all(|candidate| candidate.summary.support_coverage < GLOBAL_SEED_MIN_SUPPORT_COVERAGE);
    if needs_global_seed {
        let global = global_seed_candidates(
            level,
            GlobalSeedContext {
                samples: &seed_samples,
                moving_anchor,
                moving_center,
                orientations: &orientation_deltas,
                max_shift: max_coarse_shift,
            },
        );
        candidates.extend(global);
        refine_seed_candidates(level, &mut candidates, moving_center);
        candidates.extend(principal_frame_candidates(
            level,
            moving_center,
            max_coarse_shift,
        ));
    }

    let Some(best) = candidates.iter().copied().reduce(|current, candidate| {
        if coarse_candidate_is_better(&candidate, &current) {
            candidate
        } else {
            current
        }
    }) else {
        return Ok(StartPose {
            rigid: level.start,
            coarse_shift: 0.0,
        });
    };

    if candidates.iter().copied().any(|candidate| {
        coarse_candidates_are_ambiguous(
            &candidate,
            &best,
            moving_extent,
            COARSE_ANSWER_TOLERANCE_FRACTION * level.settings.influence_radius_mm.abs(),
        )
    }) {
        return Err(FitRejection::Ambiguous);
    }

    Ok(StartPose {
        rigid: best.rigid,
        coarse_shift: best.shift,
    })
}

fn coarse_start_candidate(
    level: &Level<'_>,
    moving_center: DVec3,
) -> (Option<CoarseCandidate>, bool) {
    let evidence = score_candidate(level, level.start);
    let strong = evidence.is_some_and(|(summary, _)| {
        summary.support_coverage >= GLOBAL_SEED_MIN_SUPPORT_COVERAGE
            && summary.geometric_rms.is_finite()
            && summary.geometric_rms
                <= level.settings.influence_radius_mm.abs().max(f64::EPSILON)
                    * GLOBAL_SEED_MAX_START_RMS_FRACTION
    });
    let candidate = evidence.map(|(summary, reciprocal)| CoarseCandidate {
        rigid: level.start,
        summary,
        reciprocal,
        shift: 0.0,
        component: nearest_component_index(level, level.start, moving_center),
    });
    (candidate, strong)
}

/// Search a bounded set of fixed-surface anchors for a crop whose current
/// pose has no useful overlap. The cheap sample pass keeps the number of full
/// nearest-surface evaluations bounded; all returned candidates still use the
/// same reciprocal evidence as every other coarse hypothesis.
fn global_seed_candidates(
    level: &Level<'_>,
    context: GlobalSeedContext<'_>,
) -> Vec<CoarseCandidate> {
    if context.samples.is_empty() {
        return Vec::new();
    }
    if fixed_surface_is_smaller(level) {
        return global_seed_candidates_from_small_fixed(level, context);
    }

    let anchor_level = Level {
        moving: level.moving,
        normals: level.normals,
        fixed: level.fixed,
        moving_surface: level.moving_surface,
        fixed_samples: level.fixed_samples,
        samples: context.samples,
        settings: level.settings,
        cancel: level.cancel,
        start: level.start,
    };
    let mut hypotheses = Vec::new();
    for (anchor_index, anchor) in level
        .fixed
        .representative_samples(GLOBAL_ANCHOR_BUDGET)
        .into_iter()
        .enumerate()
    {
        if anchor_index % 32 == 0 && level.cancel.is_cancelled() {
            break;
        }
        for (delta_index, &delta) in context.orientations.iter().enumerate() {
            if level.settings.orientation == Orientation::Inverted && delta_index != 0 {
                continue;
            }
            let rotation = delta * level.start.rotation;
            let candidate = Rigid::new(rotation, anchor.point - rotation * context.moving_anchor);
            let shift = (candidate.apply(context.moving_center)
                - level.start.apply(context.moving_center))
            .length();
            if !shift.is_finite() || shift > context.max_shift {
                continue;
            }
            let Some(mut summary) = forward_summary(&anchor_level, candidate) else {
                continue;
            };
            // This is deliberately provisional: the small side determines
            // shortlist support in both role orientations, while every
            // retained hypothesis gets the full reciprocal evidence in
            // `score_candidate` below.
            summary.support_coverage = summary.coverage;
            hypotheses.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal: None,
                shift,
                component: Some(anchor.component),
            });
        }
    }

    // The acceptance comparator intentionally has tolerance bands and is not
    // a total order. A sort comparator must be one: use a deterministic
    // lexicographic ranking for this cheap shortlist, then return to the
    // tolerance-aware comparator once the full evidence is available.
    hypotheses.sort_by(coarse_seed_order);
    hypotheses.truncate(GLOBAL_CANDIDATE_BUDGET);
    hypotheses
        .into_iter()
        .filter_map(|hypothesis| {
            if level.cancel.is_cancelled() {
                return None;
            }
            score_candidate(level, hypothesis.rigid).map(|(summary, reciprocal)| CoarseCandidate {
                rigid: hypothesis.rigid,
                summary,
                reciprocal,
                shift: hypothesis.shift,
                component: hypothesis.component,
            })
        })
        .collect()
}

/// Seed a large moving arch against a small fixed crop in the reverse frame.
/// Anchoring the full moving centroid to the crop cannot generate the correct
/// translation: the crop corresponds to one local arch patch, not its centre.
/// Instead, align the crop centroid to bounded representatives on the moving
/// surface under the inverse pose, then score retained candidates in the public
/// moving-to-fixed direction.
#[allow(clippy::cast_precision_loss)]
fn global_seed_candidates_from_small_fixed(
    level: &Level<'_>,
    context: GlobalSeedContext<'_>,
) -> Vec<CoarseCandidate> {
    let Some(moving_surface) = level.moving_surface else {
        return Vec::new();
    };
    let fixed_anchors = level.fixed.representative_samples(GLOBAL_ANCHOR_BUDGET);
    if fixed_anchors.is_empty() {
        return Vec::new();
    }
    let fixed_center = fixed_anchors
        .iter()
        .fold(DVec3::ZERO, |sum, sample| sum + sample.point)
        / fixed_anchors.len() as f64;
    let support_samples = level
        .fixed
        .representative_samples(GLOBAL_SEED_SUPPORT_SAMPLE_BUDGET);
    let moving_anchors = moving_surface.representative_samples(GLOBAL_ANCHOR_BUDGET);
    if support_samples.is_empty() || moving_anchors.is_empty() {
        return Vec::new();
    }
    let reverse_level = Level {
        moving: level.moving,
        normals: level.normals,
        fixed: level.fixed,
        moving_surface: Some(moving_surface),
        fixed_samples: &support_samples,
        samples: context.samples,
        settings: level.settings,
        cancel: level.cancel,
        start: level.start,
    };
    let mut hypotheses = Vec::new();
    for (anchor_index, anchor) in moving_anchors.into_iter().enumerate() {
        if anchor_index % 32 == 0 && level.cancel.is_cancelled() {
            break;
        }
        for (delta_index, &delta) in context.orientations.iter().enumerate() {
            if level.settings.orientation == Orientation::Inverted && delta_index != 0 {
                continue;
            }
            let rotation = delta * level.start.rotation;
            let candidate = Rigid::new(rotation, fixed_center - rotation * anchor.point);
            let shift = (candidate.apply(context.moving_center)
                - level.start.apply(context.moving_center))
            .length();
            if !shift.is_finite() || shift > context.max_shift {
                continue;
            }
            let Some(mut summary) = forward_summary(level, candidate) else {
                continue;
            };
            // Cheap reciprocal support uses only 64 small-side representatives.
            // It orders the anchor shortlist; full witnesses gate every
            // retained candidate below and every later ICP trial.
            let reciprocal = reciprocal_evidence(
                &reverse_level,
                candidate,
                level.settings.influence_radius_mm,
            );
            summary.support_coverage = reciprocal.map_or(0.0, |evidence| evidence.coverage);
            hypotheses.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal,
                shift,
                component: Some(anchor.component),
            });
        }
    }

    hypotheses.sort_by(coarse_seed_order);
    hypotheses.truncate(GLOBAL_CANDIDATE_BUDGET);
    hypotheses
        .into_iter()
        .filter_map(|hypothesis| {
            if level.cancel.is_cancelled() {
                return None;
            }
            score_candidate(level, hypothesis.rigid).map(|(summary, reciprocal)| CoarseCandidate {
                rigid: hypothesis.rigid,
                summary,
                reciprocal,
                shift: hypothesis.shift,
                component: hypothesis.component,
            })
        })
        .collect()
}

/// Let each retained global anchor take a short local ICP step before coarse
/// ranking. A one-shot point-to-surface residual can make a smooth wrong patch
/// look better than the distinctive patch that actually continues to converge.
fn refine_seed_candidates(
    level: &Level<'_>,
    candidates: &mut [CoarseCandidate],
    moving_center: DVec3,
) {
    if candidates.is_empty() {
        return;
    }
    let seed_settings = RefineSettings {
        max_iterations: level
            .settings
            .max_iterations
            .min(GLOBAL_SEED_REFINE_ITERATIONS),
        ..*level.settings
    };
    for candidate in candidates {
        if level.cancel.is_cancelled() {
            break;
        }
        let local_level = Level {
            moving: level.moving,
            normals: level.normals,
            fixed: level.fixed,
            moving_surface: level.moving_surface,
            fixed_samples: level.fixed_samples,
            samples: level.samples,
            settings: &seed_settings,
            cancel: level.cancel,
            start: candidate.rigid,
        };
        let Ok(local) = run_level(&local_level) else {
            continue;
        };
        if !local.summary.geometric_rms.is_finite()
            || local.summary.geometric_rms > candidate.summary.geometric_rms
        {
            continue;
        }
        let reciprocal = reciprocal_evidence(level, local.pose, level.settings.influence_radius_mm);
        if !reciprocal_evidence_is_usable(level, reciprocal) {
            continue;
        }
        let mut local_summary = local.summary;
        local_summary.support_coverage =
            common_support_coverage(level, local_summary.coverage, reciprocal);
        if !support_coverage_is_sufficient(local_summary.support_coverage) {
            continue;
        }
        candidate.rigid = local.pose;
        candidate.summary = local_summary;
        candidate.reciprocal = reciprocal;
        candidate.shift =
            (local.pose.apply(moving_center) - level.start.apply(moving_center)).length();
    }
}

/// Score a pose against the forward surface and, when possible, the bounded
/// reverse surface. Keeping this in one function prevents the global seed pass
/// from accidentally becoming an acceptance path with weaker evidence.
fn score_candidate(level: &Level<'_>, pose: Rigid) -> Option<(Summary, Option<ReciprocalSummary>)> {
    let mut summary = forward_summary(level, pose)?;
    let reciprocal = reciprocal_evidence(level, pose, level.settings.influence_radius_mm);
    if !reciprocal_evidence_is_usable(level, reciprocal) {
        return None;
    }
    summary.support_coverage = common_support_coverage(level, summary.coverage, reciprocal);
    support_coverage_is_sufficient(summary.support_coverage).then_some((summary, reciprocal))
}

/// Calculate only the forward objective for a coarse seed or a full candidate.
fn forward_summary(level: &Level<'_>, pose: Rigid) -> Option<Summary> {
    let found = correspondences(level, pose, level.settings.influence_radius_mm);
    let matched = found.iter().flatten().count();
    if !directional_forward_evidence_is_sufficient(level, matched, level.samples.len()) {
        return None;
    }
    let kept = trim(&found, level.settings.matching_ratio);
    if kept.len() < MIN_CORRESPONDENCES {
        return None;
    }
    let (matrix, _, _) = accumulate(&kept);
    Some(summarize(
        &found,
        &kept,
        matched,
        level.samples.len(),
        &matrix,
    ))
}

/// Orientation hypotheses from the two surfaces' principal frames.
///
/// The 24 cube rotations recover a scan turned about a coordinate axis. A scan
/// turned about a tilted axis -- a hand placement turned 45 degrees across the
/// occlusal plane, say -- stays outside every one of them, and the local
/// refine cannot leave the basin it starts in. Matching the principal frames
/// of the two surfaces covers those turns directly, at 24 cheap scores. They
/// are only hypotheses: the same forward and reciprocal evidence as every
/// other coarse candidate, and the same ambiguity guard, decide whether one
/// wins. They are tried only when no coarse candidate already carries a
/// seatable overlap, so a case the cube sweep can solve keeps its exact result.
fn principal_frame_candidates(
    level: &Level<'_>,
    moving_center: DVec3,
    max_shift: f64,
) -> Vec<CoarseCandidate> {
    // `Inverted` is an explicit winding-repair escape hatch: it must not
    // manufacture a turned pose just because that would make the surfaces face
    // each other. The identity candidate remains available to it.
    if level.settings.orientation == Orientation::Inverted {
        return Vec::new();
    }
    let seed_samples = sample_vertices(level.moving, GLOBAL_SEED_SAMPLE_BUDGET);
    let moving_points = sampled_points(level.moving, &seed_samples);
    let Some((moving_centre, moving_axes)) = principal_axes(&moving_points) else {
        return Vec::new();
    };
    let mut fixed_by_component: BTreeMap<usize, Vec<DVec3>> = BTreeMap::new();
    for sample in level.fixed.representative_samples(GLOBAL_ANCHOR_BUDGET) {
        fixed_by_component
            .entry(sample.component)
            .or_default()
            .push(sample.point);
    }
    let mut candidates = Vec::new();
    for (component, points) in fixed_by_component {
        if level.cancel.is_cancelled() {
            break;
        }
        let Some((fixed_centre, fixed_axes)) = principal_axes(&points) else {
            continue;
        };
        for rotation in principal_frame_matches(moving_axes, fixed_axes) {
            let candidate = Rigid::new(rotation, fixed_centre - rotation * moving_centre);
            let shift =
                (candidate.apply(moving_center) - level.start.apply(moving_center)).length();
            if !shift.is_finite() || shift > max_shift {
                continue;
            }
            let Some((summary, reciprocal)) = score_candidate(level, candidate) else {
                continue;
            };
            candidates.push(CoarseCandidate {
                rigid: candidate,
                summary,
                reciprocal,
                shift,
                component: Some(component),
            });
        }
    }
    candidates
}

/// The right-handed rotations that carry one principal frame onto another:
/// every axis permutation with every sign combination that keeps the frame
/// proper. The axes of a power iteration carry an arbitrary sign, and a nearly
/// equal spread can leave their order open, so all 24 are tried rather than
/// trusting one.
pub(super) fn principal_frame_matches(moving: [DVec3; 3], fixed: [DVec3; 3]) -> Vec<DQuat> {
    const PERMUTATIONS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let moving_frame = DMat3::from_cols(moving[0], moving[1], moving[2]);
    let mut matches: Vec<DQuat> = Vec::with_capacity(24);
    for permutation in PERMUTATIONS {
        for signs in 0..8u8 {
            let sign = |bit: u8| if signs & (1 << bit) == 0 { 1.0 } else { -1.0 };
            let target = DMat3::from_cols(
                fixed[permutation[0]] * sign(0),
                fixed[permutation[1]] * sign(1),
                fixed[permutation[2]] * sign(2),
            );
            if target.determinant() <= 0.0 {
                continue;
            }
            let rotation = DQuat::from_mat3(&(target * moving_frame.transpose())).normalize();
            if rotation.is_finite() && !matches.contains(&rotation) {
                matches.push(rotation);
            }
        }
    }
    matches
}

/// A surface crop's bounding-box centre can sit well above its actual surface
/// when curvature is strong. Use the deterministic sample centroid for global
/// anchoring, while retaining the bounds centre for displacement bookkeeping.
fn sampled_points(soup: Soup<'_>, samples: &[u32]) -> Vec<DVec3> {
    samples
        .iter()
        .filter_map(|&raw| {
            usize::try_from(raw)
                .ok()
                .and_then(|vertex| vertex_at(soup.positions, vertex))
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn sampled_centroid(soup: Soup<'_>, samples: &[u32]) -> Option<DVec3> {
    let mut sum = DVec3::ZERO;
    let mut count = 0usize;
    for &raw in samples {
        let vertex = usize::try_from(raw).ok()?;
        let point = vertex_at(soup.positions, vertex)?;
        sum += point;
        count += 1;
    }
    (count > 0).then(|| sum / count as f64)
}

fn nearest_component_index(level: &Level<'_>, pose: Rigid, moving_center: DVec3) -> Option<usize> {
    let point = pose.apply(moving_center);
    level
        .fixed
        .component_bounds()
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_center = (left.0 + left.1) * 0.5;
            let right_center = (right.0 + right.1) * 0.5;
            point
                .distance_squared(left_center)
                .total_cmp(&point.distance_squared(right_center))
        })
        .map(|(index, _)| index)
}

/// Whether a candidate keeps enough of the incumbent's forward moving-side
/// coverage. This directional witness stays separate from common support.
fn coarse_keeps_coverage(candidate: f64, current: f64) -> bool {
    candidate >= current * COARSE_COVERAGE_KEEP_FRACTION
}

/// Whether a candidate retains enough of the incumbent's common smaller-side
/// support, whichever input role supplies that side.
fn coarse_keeps_support(candidate: f64, current: f64) -> bool {
    candidate >= current * COARSE_COVERAGE_KEEP_FRACTION
}

pub(super) fn coarse_candidate_is_better(
    candidate: &CoarseCandidate,
    current: &CoarseCandidate,
) -> bool {
    // Compare overlap on the smaller indexed surface before any directional
    // moving-side seating or residual. A large-side seated fraction cannot
    // compensate for losing most of a small fixed crop.
    if candidate.summary.support_coverage > current.summary.support_coverage + COARSE_TIE_COVERAGE {
        return true;
    }
    if candidate.summary.support_coverage + COARSE_TIE_COVERAGE < current.summary.support_coverage {
        return false;
    }
    // Seating comes before every residual comparison. A candidate that puts
    // materially more of the surface inside the seated band is the better
    // answer even when its trimmed residual is worse, because the residual is
    // what a deformed majority can drive down. Without this the search walks
    // from the operator's own placement into the basin that best hides a
    // preparation.
    if candidate.summary.seated_fraction > current.summary.seated_fraction + COARSE_TIE_SEATED {
        return true;
    }
    if candidate.summary.seated_fraction + COARSE_TIE_SEATED < current.summary.seated_fraction {
        return false;
    }
    // Keep the two directional measures independent during arbitration. A
    // candidate may be supported by the smaller-side fraction and still only
    // match a small smooth patch on the other surface; reciprocal growth can
    // break a residual tie when forward support is retained.
    if coarse_reciprocal_advantage(candidate, current) {
        return true;
    }
    // Residual alone can hide a crop or deformation. Preserve the same common
    // support in either input role before replacing a lower-residual seating.
    if candidate.summary.geometric_rms < current.summary.geometric_rms * STALL_IMPROVEMENT {
        // A lower residual is only an improvement if it still explains the same
        // surface. A small smooth patch can beat the true seating on residual
        // alone while covering almost none of the moving scan, and the search
        // floor admits a candidate at 1% support. Keep the incumbent unless
        // the candidate keeps that common support; the trust gate still decides
        // whether anything may be committed.
        return coarse_keeps_support(
            candidate.summary.support_coverage,
            current.summary.support_coverage,
        ) || coarse_explains_more_fixed(candidate.reciprocal, current.reciprocal);
    }
    if candidate.summary.geometric_rms > current.summary.geometric_rms * (1.0 / STALL_IMPROVEMENT) {
        return false;
    }
    let reciprocal_better = match (candidate.reciprocal, current.reciprocal) {
        (Some(candidate), Some(current)) => {
            candidate.coverage > current.coverage + COARSE_TIE_COVERAGE
        }
        (Some(_), None) => true,
        _ => false,
    };
    reciprocal_better || candidate.shift + COARSE_TIE_SHIFT_MM < current.shift
}

fn coarse_reciprocal_advantage(candidate: &CoarseCandidate, current: &CoarseCandidate) -> bool {
    match (candidate.reciprocal, current.reciprocal) {
        (Some(candidate_reciprocal), Some(current_reciprocal)) => {
            candidate_reciprocal.coverage
                > current_reciprocal.coverage + COARSE_RECIPROCAL_ADVANTAGE
                && candidate.summary.geometric_rms
                    <= current.summary.geometric_rms * COARSE_RECIPROCAL_RMS_FACTOR
                && coarse_keeps_coverage(candidate.summary.coverage, current.summary.coverage)
        }
        _ => false,
    }
}

fn coarse_explains_more_fixed(
    candidate: Option<ReciprocalSummary>,
    current: Option<ReciprocalSummary>,
) -> bool {
    match (candidate, current) {
        (Some(candidate), Some(current)) => {
            candidate.coverage > current.coverage + COARSE_RECIPROCAL_ADVANTAGE
        }
        (Some(_), None) => true,
        _ => false,
    }
}

fn coarse_seed_order(left: &CoarseCandidate, right: &CoarseCandidate) -> std::cmp::Ordering {
    right
        .summary
        .support_coverage
        .total_cmp(&left.summary.support_coverage)
        .then_with(|| {
            left.summary
                .geometric_rms
                .total_cmp(&right.summary.geometric_rms)
        })
        .then_with(|| left.shift.total_cmp(&right.shift))
        .then_with(|| {
            left.rigid
                .translation
                .x
                .total_cmp(&right.rigid.translation.x)
        })
        .then_with(|| {
            left.rigid
                .translation
                .y
                .total_cmp(&right.rigid.translation.y)
        })
        .then_with(|| {
            left.rigid
                .translation
                .z
                .total_cmp(&right.rigid.translation.z)
        })
}

/// Whether a competing coarse hypothesis is a rival answer to the best one.
///
/// A rival has to be at least as good on both axes. The guard below stops the
/// tool picking one of two equally supported seatings by component id; it is
/// not a reason to give up on a pair the search can seat.
///
/// A candidate that is merely close on both axes is not a rival: a hypothesis
/// worse in residual and worse in coverage explains the surface less well by
/// both measures and does not refuse the better pose.
fn coarse_candidates_are_equivalent(candidate: &CoarseCandidate, best: &CoarseCandidate) -> bool {
    let worse_on_residual = candidate.summary.geometric_rms > best.summary.geometric_rms;
    let worse_on_support = candidate.summary.support_coverage < best.summary.support_coverage;
    if worse_on_residual && worse_on_support {
        return false;
    }
    let rms_scale = candidate
        .summary
        .geometric_rms
        .max(best.summary.geometric_rms)
        .max(f64::MIN_POSITIVE);
    if (candidate.summary.geometric_rms - best.summary.geometric_rms).abs()
        > rms_scale * COARSE_TIE_RELATIVE_RMS + 1e-6
        || (candidate.summary.support_coverage - best.summary.support_coverage).abs()
            > COARSE_TIE_COVERAGE
    {
        return false;
    }
    true
}

/// How far a coarse hypothesis may turn the scan and still be answering the
/// operator's question.
///
/// A hypothesis that flips or rolls the jaw is not a rival answer to the same
/// question, it is a different question. The coarse search tries 24 cube
/// orientations, and on a real arch pair an upside-down pose covers a patch
/// comparable to the upright seating at every starting distance from touching
/// to 40 mm apart; counting it as a rival would refuse every such fit as
/// `Ambiguous`. `Orientation` is how an operator asks for a flipped answer;
/// the ambiguity guard is not.
const COARSE_SAME_QUESTION_RAD: f64 = std::f64::consts::FRAC_PI_2;

pub(super) fn coarse_candidates_are_ambiguous(
    candidate: &CoarseCandidate,
    best: &CoarseCandidate,
    extent: f64,
    tolerance: f64,
) -> bool {
    // Connected-component identity is useful for ranking hypotheses, but it
    // is not evidence that two poses are different answers. Repeated cusps or
    // symmetric windows can live in one component, and choosing one of them
    // deterministically would authorize a misleading heatmap. Treat every
    // distinct, equally supported nearby pose as ambiguous.
    //
    // "Distinct" means distinct as an answer, so a hypothesis that turns the
    // scan onto a different face of the cube is not a rival: see
    // `COARSE_SAME_QUESTION_RAD`.
    candidate.shift <= best.shift + COARSE_TIE_SHIFT_MM
        && turn_between(candidate.rigid, best.rigid) <= COARSE_SAME_QUESTION_RAD
        && poses_are_distinct(candidate.rigid, best.rigid, extent, tolerance)
        && coarse_candidates_are_equivalent(candidate, best)
}

/// Whether two coarse poses are different answers, judged at the scan's scale.
///
/// Comparing a translation against one epsilon and a rotation against another
/// treats the two independently and refuses real arch pairs: two hypotheses
/// six micrometres apart in translation and 0.62 degrees apart in rotation —
/// one seating, parameterised twice — clear a 0.57-degree rotation epsilon and
/// would be declared two rival answers.
///
/// What matters is how far the two poses actually move the geometry. A rotation
/// of `dr` about the scan's centre moves its rim by `dr * extent/2`, so the
/// displacement at the rim is the sum, and it is compared against a fraction of
/// the correspondence radius: hypotheses nearer than that place the surface
/// within the distance the search itself treats as the same place.
fn poses_are_distinct(left: Rigid, right: Rigid, extent: f64, tolerance: f64) -> bool {
    let turned = (left.rotation * right.rotation.inverse())
        .to_scaled_axis()
        .length();
    let rim_shift =
        (left.translation - right.translation).length() + turned * extent.max(0.0) * 0.5;
    rim_shift > tolerance
}

/// Bounded global orientation probes used before local point-to-plane ICP.
///
/// Importers and manual placement commonly leave a scan quarter-turned or
/// upside down while its centre is already near the target. Comparing only the
/// current rotation then lets the first tangent patch win and can return a
/// sideways pose. These 24 cube orientations cover the principal dental-CAD
/// axis conventions at a fixed, deterministic cost; the subsequent ICP remains
/// responsible for fine arbitrary-angle correction.
fn coarse_orientation_deltas() -> [DQuat; 24] {
    let quarter = std::f64::consts::FRAC_PI_2;
    let turns = [0.0, quarter, std::f64::consts::PI, -quarter];
    core::array::from_fn(|index| {
        let face = index / 4;
        let turn = turns[index % 4];
        match face {
            0 => DQuat::from_euler(EulerRot::XYZ, 0.0, 0.0, turn),
            1 => DQuat::from_euler(EulerRot::XYZ, quarter, 0.0, turn),
            2 => DQuat::from_euler(EulerRot::XYZ, -quarter, 0.0, turn),
            3 => DQuat::from_euler(EulerRot::XYZ, std::f64::consts::PI, 0.0, turn),
            4 => DQuat::from_euler(EulerRot::XYZ, 0.0, quarter, turn),
            _ => DQuat::from_euler(EulerRot::XYZ, 0.0, -quarter, turn),
        }
    })
}
