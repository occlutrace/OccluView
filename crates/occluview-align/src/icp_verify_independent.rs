//! Independent common-region evidence, derived from Gelfand et al. (2003),
//! <https://pixl.cs.princeton.edu/pubs/Gelfand_2003_GSS/stabicp.pdf>, and Phillips,
//! Liu and Tomasi (2006), <https://arxiv.org/abs/cs/0606098>.
//! The undamped Jacobian is centred and radius normalized. Residuals include
//! every complete non-border holdout hit within .5 mm, without a second trim.
//! Disjoint spatial populations are never extrapolated to unqueried area.

use crate::{
    CandidateEvidence, Metric, MissingReason, NormalPolicy, PreparedSurface, ResidualSummary,
    Rigid, SampleBatch,
};
use glam::DVec3;
use occluview_geometry::surface::{
    GeometryControl, GeometryStop, QueryOutcome, SurfaceQueryScratch,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(super) struct EvidencePair {
    pub point: DVec3,
    pub normal: Option<DVec3>,
    pub weight: f64,
}

pub(crate) struct VerificationPass {
    pub evidence: CandidateEvidence,
    pub center: DVec3,
    pub radius: f64,
}

struct Direction {
    tight: f64,
    common: f64,
    soft: f64,
    compatible: f64,
    reciprocal: f64,
    normals_complete: bool,
    distances: Vec<(f64, f64)>,
    planes: Vec<(f64, f64)>,
    pairs: Vec<EvidencePair>,
    cells: BTreeSet<[i64; 3]>,
}

/// Complete both independent directions before publishing any measurement.
/// An interrupted exact query discards the entire pass, including its best hit.
/// Returned whole-area coverage is a lower bound on the disjoint role's support.
///
/// # Errors
/// Returns the actual cancellation, deadline, resource, work or numeric stop.
pub(crate) fn verify_candidate(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    policy: NormalPolicy,
    control: &GeometryControl,
) -> Result<VerificationPass, GeometryStop> {
    verify_at_resolution(moving, fixed, pose, policy, Resolution::Full, control)
}

/// Measure a reserved rival on 2,048 representatives of each disjoint role.
/// Weights retain the full role denominator. This screening pass cannot supply
/// full-resolution final authority; close rivals must be fully re-evaluated.
///
/// # Errors
/// Returns the actual control or arithmetic interruption without partial evidence.
pub(crate) fn verify_rival_candidate(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    policy: NormalPolicy,
    control: &GeometryControl,
) -> Result<VerificationPass, GeometryStop> {
    verify_at_resolution(moving, fixed, pose, policy, Resolution::Rival, control)
}

#[derive(Clone, Copy)]
enum Resolution {
    Full,
    Rival,
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one common reduction over explicitly bounded training and holdout populations"
)]
fn verify_at_resolution(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    policy: NormalPolicy,
    resolution: Resolution,
    control: &GeometryControl,
) -> Result<VerificationPass, GeometryStop> {
    let (holdout_cap, training_cap) = match resolution {
        Resolution::Full => (8_192, 16_384),
        Resolution::Rival => (2_048, 2_048),
    };
    if !pose.is_finite() {
        return Err(GeometryStop::Numerical);
    }
    let count = moving.samples[3]
        .samples
        .len()
        .min(holdout_cap)
        .saturating_add(fixed.samples[3].samples.len().min(holdout_cap))
        .saturating_add(moving.samples[2].samples.len().min(training_cap))
        .saturating_add(fixed.samples[2].samples.len().min(training_cap));
    let _memory = control.reserve(count.saturating_mul(512).saturating_add(4096))?;
    let forward = direction(
        moving,
        fixed,
        &moving.samples[3],
        holdout_cap,
        pose,
        false,
        policy,
        control,
    )?;
    let reverse = direction(
        fixed,
        moving,
        &fixed.samples[3],
        holdout_cap,
        pose.inverse(),
        true,
        policy,
        control,
    )?;
    let train_forward = direction(
        moving,
        fixed,
        &moving.samples[2],
        training_cap,
        pose,
        false,
        policy,
        control,
    )?;
    let train_reverse = direction(
        fixed,
        moving,
        &fixed.samples[2],
        training_cap,
        pose.inverse(),
        true,
        policy,
        control,
    )?;
    let areas = [moving.eligible_area_mm2, fixed.eligible_area_mm2];
    if areas.iter().any(|a| !a.is_finite() || *a <= 0.) {
        return Err(GeometryStop::Numerical);
    }
    let euclidean = lesser_summary(&forward.distances, &reverse.distances, control)?;
    let training = lesser_summary(&train_forward.distances, &train_reverse.distances, control)?;
    let normals = forward.normals_complete && reverse.normals_complete;
    let plane = if normals {
        lesser_summary(&forward.planes, &reverse.planes, control)?
    } else {
        Metric::Missing(MissingReason::Degenerate)
    };
    let common = forward.tight.min(reverse.tight);
    let common_region = forward.common.min(reverse.common);
    let smaller = areas[0].min(areas[1]);
    let ratio = |numerator: f64, denominator: f64| {
        if denominator > 0. {
            Metric::Measured((numerator / denominator).clamp(0., 1.))
        } else {
            Metric::Missing(MissingReason::NoSupport)
        }
    };
    let inlier = match (
        ratio(forward.tight, forward.common),
        ratio(reverse.tight, reverse.common),
    ) {
        (Metric::Measured(a), Metric::Measured(b)) => Metric::Measured(a.min(b)),
        _ => Metric::Missing(MissingReason::NoSupport),
    };
    let orientation = if normals {
        match (
            ratio(forward.compatible, forward.common),
            ratio(reverse.compatible, reverse.common),
        ) {
            (Metric::Measured(a), Metric::Measured(b)) => Metric::Measured(a.min(b)),
            _ => Metric::Missing(MissingReason::NoSupport),
        }
    } else {
        Metric::Missing(MissingReason::Degenerate)
    };
    let reciprocal = match (
        ratio(forward.reciprocal, forward.common),
        ratio(reverse.reciprocal, reverse.common),
    ) {
        (Metric::Measured(a), Metric::Measured(b)) => Metric::Measured(a.min(b)),
        _ => Metric::Missing(MissingReason::NoSupport),
    };
    let effective = u32::try_from(forward.cells.len().min(reverse.cells.len()))
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut pairs = forward.pairs;
    pairs
        .try_reserve(reverse.pairs.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    pairs.extend(reverse.pairs);
    let info = information_on_common_region(&pairs, effective, control)?;
    let mut evidence = CandidateEvidence {
        eligible_area_mm2: Metric::Measured(areas),
        representation_area_mm2: Metric::Measured([
            moving.represented_area_mm2,
            fixed.represented_area_mm2,
        ]),
        original_surface_exact: [moving.exact_original, fixed.exact_original],
        queried_population_area_mm2: Metric::Measured([
            moving.samples[3].represented_area_mm2,
            fixed.samples[3].represented_area_mm2,
        ]),
        policy_compatible_area_mm2: Metric::Measured([
            normals.then_some(forward.compatible),
            normals.then_some(reverse.compatible),
        ]),
        coverage_02: Metric::Measured([forward.tight / areas[0], reverse.tight / areas[1]]),
        coverage_05: Metric::Measured([forward.common / areas[0], reverse.common / areas[1]]),
        common_area_mm2: Metric::Measured(common),
        overlap_smaller: Metric::Measured(common / smaller),
        common_region_area_mm2: Metric::Measured(common_region),
        euclidean_mm: euclidean,
        plane_mm: plane,
        inlier_ratio: inlier,
        effective_cells: Metric::Measured(effective),
        orientation_fraction: orientation,
        reciprocal_fraction: reciprocal,
        holdout_ratio: match (euclidean, training) {
            (Metric::Measured(h), Metric::Measured(t)) => Metric::Measured(h.rms / t.rms.max(0.02)),
            _ => Metric::Missing(MissingReason::NoSupport),
        },
        holdout_complete: true,
        population_coverage_complete: moving.samples[3].unqueried_area_mm2 <= areas[0] * 1e-12
            && fixed.samples[3].unqueried_area_mm2 <= areas[1] * 1e-12,
        ..CandidateEvidence::default()
    };
    // Both cost and LCP are evaluated on the same declared holdout population.
    if let Metric::Measured(residual) = euclidean {
        let fraction = common_region / smaller;
        if fraction > 0. {
            let cost = (residual.rms.powi(2) + 0.02f64.powi(2)).sqrt() / fraction.sqrt();
            evidence.score =
                Metric::Measured((forward.soft.min(reverse.soft) / smaller) / (1. + cost / 0.20));
            evidence.trim_fraction = Metric::Measured(fraction);
        }
    } else {
        // A completely queried sentinel with no support has a measured zero.
        evidence.score = Metric::Measured(0.);
    }
    let (center, radius) = if let Some(info) = info {
        evidence.info_eigenvalues = Metric::Measured(info.values);
        evidence.info_eigenvectors = Metric::Measured(info.vectors);
        evidence.weak_twists = info.weak;
        (info.center, info.radius)
    } else {
        evidence.info_eigenvalues = Metric::Missing(MissingReason::Degenerate);
        (DVec3::ZERO, 0.)
    };
    Ok(VerificationPass {
        evidence,
        center,
        radius,
    })
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one serial complete-query pass with explicit directional denominators"
)]
fn direction(
    source: &PreparedSurface,
    target: &PreparedSurface,
    batch: &SampleBatch,
    capacity: usize,
    transform: Rigid,
    reverse: bool,
    policy: NormalPolicy,
    control: &GeometryControl,
) -> Result<Direction, GeometryStop> {
    let count = capacity.min(batch.samples.len());
    let reliable = source.quality.orientation_coherent
        && target.quality.orientation_coherent
        && source.exact_original
        && target.exact_original;
    let mut result = Direction {
        tight: 0.,
        common: 0.,
        soft: 0.,
        compatible: 0.,
        reciprocal: 0.,
        normals_complete: reliable,
        distances: Vec::new(),
        planes: Vec::new(),
        pairs: Vec::new(),
        cells: BTreeSet::new(),
    };
    for vector in [&mut result.distances, &mut result.planes] {
        vector
            .try_reserve_exact(count)
            .map_err(|_| GeometryStop::ResourceLimit)?;
    }
    result
        .pairs
        .try_reserve_exact(count)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut target_scratch = SurfaceQueryScratch::new(control)?;
    let mut source_scratch = SurfaceQueryScratch::new(control)?;
    for i in 0..count {
        let start = i * batch.samples.len() / count;
        let end = (i + 1) * batch.samples.len() / count;
        let sample = &batch.samples[(start + end - 1) / 2];
        control.charge_operations(1)?;
        if !sample.point.is_finite()
            || !sample.area_weight_mm2.is_finite()
            || sample.area_weight_mm2 <= 0.
        {
            return Err(GeometryStop::Numerical);
        }
        let point = transform.apply(sample.point);
        let hit = if super::super::icp_overlap::outside_query_bounds(
            &target.original_index,
            point,
            0.5,
        ) {
            None
        } else {
            match target
                .original_index
                .nearest_with_scratch(point, 0.5, &mut target_scratch)
            {
                QueryOutcome::Complete(hit) => hit,
                QueryOutcome::Interrupted { reason, .. } => return Err(reason),
            }
        };
        let Some(hit) = hit.filter(|h| !h.on_border) else {
            continue;
        };
        let distance = point.distance(hit.point);
        // Each deterministic representative carries its entire equal-area
        // block. Missing correspondences retain their population denominator.
        #[allow(clippy::cast_precision_loss)]
        let weight = sample.area_weight_mm2 * (end - start) as f64;
        if !distance.is_finite() || distance > 0.5 {
            continue;
        }
        result.common += weight;
        result.distances.push((distance, weight));
        if distance <= 0.2 {
            result.tight += weight;
            result.soft += weight * (1. - (distance / 0.2).powi(2));
            result.cells.insert(cell(sample.point)?);
        }
        let normal = reliable
            .then_some(hit.normal.normalize_or_zero())
            .filter(|n| n.length_squared() > 0.5);
        let source_normal = sample
            .normal
            .filter(|n| reliable && n.is_finite() && n.length_squared() > 0.5);
        if let (Some(n), Some(source_n)) = (normal, source_normal) {
            let dot = (transform.rotation * source_n).dot(n);
            let compatible = match policy {
                NormalPolicy::Match => dot > 0.5,
                NormalPolicy::Opposed => dot < -0.5,
                NormalPolicy::Unsigned => dot.abs() > 0.5,
            };
            if compatible {
                result.compatible += weight;
            }
            result
                .planes
                .push(((point - hit.point).dot(n).abs(), weight));
        } else {
            result.normals_complete = false;
        }
        if distance <= 0.2 {
            result.pairs.push(EvidencePair {
                point: if reverse { sample.point } else { point },
                normal: if reverse { source_normal } else { normal },
                weight,
            });
        }
        let back_point = transform.inverse().apply(hit.point);
        let back =
            match source
                .original_index
                .nearest_with_scratch(back_point, 0.3, &mut source_scratch)
            {
                QueryOutcome::Complete(hit) => hit,
                QueryOutcome::Interrupted { reason, .. } => return Err(reason),
            };
        if back.is_some_and(|h| !h.on_border && h.point.distance(sample.point) <= 0.3) {
            result.reciprocal += weight;
        }
    }
    Ok(result)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite bounded floor checked before conversion"
)]
fn cell(point: DVec3) -> Result<[i64; 3], GeometryStop> {
    let value = point.floor();
    if !value.is_finite() || value.abs().max_element() >= 9_223_372_036_854_775_808. {
        return Err(GeometryStop::Numerical);
    }
    Ok(value.to_array().map(|x| x as i64))
}

fn lesser_summary(
    a: &[(f64, f64)],
    b: &[(f64, f64)],
    control: &GeometryControl,
) -> Result<Metric<ResidualSummary>, GeometryStop> {
    match (summary(a, control)?, summary(b, control)?) {
        (Metric::Measured(a), Metric::Measured(b)) => Ok(Metric::Measured(ResidualSummary {
            median: a.median.max(b.median),
            rms: a.rms.max(b.rms),
            p95: a.p95.max(b.p95),
        })),
        _ => Ok(Metric::Missing(MissingReason::NoSupport)),
    }
}

fn summary(
    values: &[(f64, f64)],
    control: &GeometryControl,
) -> Result<Metric<ResidualSummary>, GeometryStop> {
    if values.is_empty() {
        return Ok(Metric::Missing(MissingReason::NoSupport));
    }
    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(values.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    sorted.extend_from_slice(values);
    let mut weight = 0.;
    let mut squared = 0.;
    for &(d, w) in values {
        control.charge_operations(1)?;
        weight += w;
        squared += d * d * w;
    }
    let median = super::super::icp_surface_step::quantile(&mut sorted, 0.5, control)?;
    let p95 = super::super::icp_surface_step::quantile(&mut sorted, 0.95, control)?;
    let rms = (squared / weight).sqrt();
    if !rms.is_finite() {
        return Err(GeometryStop::Numerical);
    }
    Ok(Metric::Measured(ResidualSummary { median, rms, p95 }))
}

pub(super) struct Information {
    pub values: [f64; 6],
    pub vectors: [[f64; 6]; 6],
    pub weak: Vec<[f64; 6]>,
    pub center: DVec3,
    pub radius: f64,
}

/// Undamped mean of unit-normal outer products on independent tight support.
/// Missing normals, <64 cells or radius <=.5 mm supplies no information.
///
/// # Errors
/// Returns the control stop or non-finite arithmetic; never regularizes rank.
pub(super) fn information_on_common_region(
    pairs: &[EvidencePair],
    cells: u32,
    control: &GeometryControl,
) -> Result<Option<Information>, GeometryStop> {
    if cells < 64 || pairs.is_empty() || pairs.iter().any(|p| p.normal.is_none()) {
        return Ok(None);
    }
    let mut center = DVec3::ZERO;
    let mut total = 0.;
    for p in pairs {
        control.charge_operations(1)?;
        if !p.point.is_finite() || !p.weight.is_finite() || p.weight <= 0. {
            return Err(GeometryStop::Numerical);
        }
        let next = total + p.weight;
        center += (p.point - center) * (p.weight / next);
        total = next;
    }
    let mut radius2 = 0.;
    for p in pairs {
        control.charge_operations(1)?;
        radius2 += (p.weight / total) * p.point.distance_squared(center);
    }
    let radius = radius2.sqrt();
    if !radius.is_finite() || !center.is_finite() || !total.is_finite() {
        return Err(GeometryStop::Numerical);
    }
    if radius <= 0.5 {
        return Ok(None);
    }
    let mut matrix = [[0.; 6]; 6];
    for p in pairs {
        control.charge_operations(1)?;
        let n = p.normal.unwrap_or(DVec3::ZERO).normalize_or_zero();
        if !n.is_finite() || n.length_squared() < 0.5 {
            return Ok(None);
        }
        let torque = (p.point - center).cross(n) / radius;
        let j = [torque.x, torque.y, torque.z, n.x, n.y, n.z];
        for (i, row) in matrix.iter_mut().enumerate() {
            for (k, entry) in row.iter_mut().enumerate() {
                *entry += (p.weight / total) * j[i] * j[k];
            }
        }
    }
    if matrix.iter().flatten().any(|v| !v.is_finite()) {
        return Err(GeometryStop::Numerical);
    }
    control.charge_operations(64 * 36)?;
    let (raw_values, raw_vectors) = super::super::icp_solve::symmetric_eigendecomposition(matrix);
    let mut order = [0, 1, 2, 3, 4, 5];
    order.sort_by(|&a, &b| raw_values[a].total_cmp(&raw_values[b]).then(a.cmp(&b)));
    let values = order.map(|i| raw_values[i].max(0.));
    let vectors = order.map(|i| std::array::from_fn(|row| raw_vectors[row][i]));
    let weak = vectors
        .iter()
        .zip(values)
        .filter(|(_, v)| *v <= values[5] * 1e-6)
        .map(|(vector, _)| *vector)
        .collect();
    Ok(Some(Information {
        values,
        vectors,
        weak,
        center,
        radius,
    }))
}

/// Omit each spatial stratum, solve at most three dense iterations on at most
/// 2,048 independent points per direction and measure maximum patch-centre
/// drift. Missing support stops the pass instead of reporting zero drift.
///
/// # Errors
/// Returns actual control/numeric stops; deficient omitted support is missing.
#[expect(
    clippy::too_many_arguments,
    reason = "immutable surfaces and one omitted-stratum sensitivity pass"
)]
pub(crate) fn spatial_jackknife(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    policy: NormalPolicy,
    center: DVec3,
    control: &GeometryControl,
) -> Result<Metric<[f64; 2]>, GeometryStop> {
    let _memory = control.reserve(4096 * 256)?;
    let local_center = pose.inverse().apply(center);
    let mut drift = [0f64; 2];
    for omitted in 0..8 {
        let Some(trial) = perturb_refine(moving, fixed, pose, policy, Some(omitted), 3, control)?
        else {
            return Ok(Metric::Missing(MissingReason::TooFewSamples));
        };
        drift[0] = drift[0].max(trial.apply(local_center).distance(pose.apply(local_center)));
        drift[1] = drift[1].max(
            2. * trial
                .rotation
                .dot(pose.rotation)
                .abs()
                .clamp(0., 1.)
                .acos()
                .to_degrees(),
        );
    }
    Ok(Metric::Measured(drift))
}

/// Bounded dense rerun used by omitted-stratum and rival checks; it cannot
/// change a training checkpoint or manufacture confidence.
#[expect(
    clippy::too_many_arguments,
    reason = "immutable surfaces, one omitted stratum and bounded perturbation iterations"
)]
pub(crate) fn perturb_refine(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    mut pose: Rigid,
    policy: NormalPolicy,
    omitted: Option<u8>,
    iterations: usize,
    control: &GeometryControl,
) -> Result<Option<Rigid>, GeometryStop> {
    let _memory = control.reserve(4096 * 256)?;
    for _ in 0..iterations.min(5) {
        let mut pairs = Vec::new();
        pairs
            .try_reserve_exact(4096)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for (source, target, transform, reverse) in [
            (moving, fixed, pose, false),
            (fixed, moving, pose.inverse(), true),
        ] {
            let samples = &source.samples[3].samples;
            let stride = samples.len().div_ceil(2048).max(1);
            let mut scratch = SurfaceQueryScratch::new(control)?;
            for sample in samples.iter().step_by(stride).take(2048) {
                control.charge_operations(1)?;
                let point = transform.apply(sample.point);
                let common_point = if reverse { sample.point } else { point };
                if omitted == Some(crate::spatial_stratum(common_point)?) {
                    continue;
                }
                let hit = if super::super::icp_overlap::outside_query_bounds(
                    &target.original_index,
                    point,
                    0.3,
                ) {
                    None
                } else {
                    match target
                        .original_index
                        .nearest_with_scratch(point, 0.3, &mut scratch)
                    {
                        QueryOutcome::Complete(hit) => hit,
                        QueryOutcome::Interrupted { reason, .. } => return Err(reason),
                    }
                };
                let Some(hit) = hit.filter(|h| !h.on_border) else {
                    continue;
                };
                let reliable = source.exact_original
                    && target.exact_original
                    && source.quality.orientation_coherent
                    && target.quality.orientation_coherent;
                let normal = reliable.then_some(hit.normal.normalize_or_zero());
                if let (Some(a), Some(b)) = (sample.normal, normal) {
                    let dot = (transform.rotation * a).dot(b);
                    if (policy == NormalPolicy::Match && dot <= 0.5)
                        || (policy == NormalPolicy::Opposed && dot >= -0.5)
                    {
                        continue;
                    }
                }
                pairs.push(super::super::icp_surface_step::SurfacePair {
                    moving: if reverse { hit.point } else { sample.point },
                    fixed: if reverse { sample.point } else { hit.point },
                    normal: if reverse {
                        sample.normal.filter(|_| reliable)
                    } else {
                        normal
                    },
                    weight: sample.area_weight_mm2,
                });
            }
        }
        if pairs.len() < 64 {
            return Ok(None);
        }
        let Some(model) = super::super::icp_surface_step::accumulate_robust(&pairs, pose, control)?
        else {
            return Ok(None);
        };
        let (next, termination) =
            super::super::icp_surface_step::line_search(&pairs, pose, &model, true, control)?;
        pose = next;
        if termination != crate::RefinementTermination::NotStarted {
            break;
        }
    }
    Ok(Some(pose))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_rival_verification_bounds_queries_and_preserves_area() {
        let mesh = crate::proposal_test_support::arch::plane();
        let prepare = |side| {
            crate::prepare_alignment_surface(
                crate::MeshInput {
                    soup: mesh.soup(),
                    world_from_local: glam::DAffine3::IDENTITY,
                    revision: 1,
                },
                side,
                crate::RegionPolicy::AllEligible,
                &GeometryControl::unlimited(),
            )
            .unwrap()
            .surface
            .unwrap()
        };
        let moving = prepare(crate::SurfaceSide::Moving);
        let fixed = prepare(crate::SurfaceSide::Fixed);
        let control = GeometryControl::unlimited();
        let pass = verify_rival_candidate(
            &moving,
            &fixed,
            Rigid::IDENTITY,
            NormalPolicy::Unsigned,
            &control,
        )
        .unwrap();
        assert!(
            control.counters().query_calls <= 16_384,
            "{} queries",
            control.counters().query_calls
        );
        let areas = match pass.evidence.queried_population_area_mm2 {
            Metric::Measured(areas) => Some(areas),
            Metric::Missing(_) => None,
        }
        .unwrap();
        assert!((areas[0] - moving.samples[3].population_area_mm2).abs() <= 1e-10);
        assert!((areas[1] - fixed.samples[3].population_area_mm2).abs() <= 1e-10);
        assert!(pass.evidence.holdout_complete);
        assert!(!pass.evidence.verification_complete);
    }
}
