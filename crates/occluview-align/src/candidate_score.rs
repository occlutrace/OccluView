//! Common-area proposal ranking, independently derived from Phillips, Liu and
//! Tomasi (2006), <https://arxiv.org/abs/cs/0606098>. The engineering objective
//! adds a 0.02 mm floor and uses the lesser bidirectional eligible area; it is
//! not the paper's fractional RMSD exponent or a confidence certificate.

use crate::icp::{common_area, directional_support};
use crate::{
    CandidateId, NormalPolicy, PreparedSurface, Rigid, SeedOrigin, SurfaceIndex, SurfaceSample,
};
use occluview_geometry::surface::{GeometryControl, GeometryStop};
use std::{cmp::Ordering, collections::BTreeSet};

const FRACTIONS: [f64; 10] = [0.01, 0.03, 0.05, 0.10, 0.20, 0.30, 0.50, 0.70, 0.90, 1.];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CoarseScore {
    pub score: f64,
    pub common_area: f64,
    pub overlap: f64,
    pub coverage_02: [f64; 2],
    pub coverage_05: [f64; 2],
    pub fraction: f64,
    pub rms: Option<f64>,
    pub orientation: Option<f64>,
    /// Actual queried role populations, never doubled into full eligible area.
    pub population_area: [f64; 2],
    /// Policy-compatible area per direction; absent normal quality stays absent.
    pub policy_support: [Option<f64>; 2],
    /// Larger-area prefix retained for refinement when its occupied-cell set
    /// differs by more than 20% from the minimum-cost prefix.
    #[allow(
        dead_code,
        reason = "refinement consumes both retained overlap hypotheses"
    )]
    pub supported_prefix: Option<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Proposal {
    pub id: CandidateId,
    pub pose: Rigid,
    pub origins: Vec<SeedOrigin>,
    pub score: CoarseScore,
}

/// Cheap proxy indices are used only to rank translations for exact rescoring.
#[derive(Clone, Copy)]
pub(crate) struct CoarseScoring<'a> {
    pub(crate) policy: NormalPolicy,
    pub(crate) samples_per_side: usize,
    pub(crate) ceiling: f64,
    pub(crate) proxies: Option<(&'a SurfaceIndex, &'a SurfaceIndex)>,
}

#[derive(Clone, Copy)]
pub(crate) struct WeightedDistance {
    pub distance: Option<f64>,
    pub weight: f64,
    pub compatible: Option<bool>,
    pub cell: [u64; 3],
}

/// Prefix of the full eligible population; unavailable area cannot be trimmed
/// out of the denominator. The last sample can contribute a fractional weight.
pub(crate) fn weighted_trim_sweep(
    distances: &mut [WeightedDistance],
    area: f64,
    ceiling: f64,
    control: &GeometryControl,
) -> Result<Option<(f64, f64, f64)>, GeometryStop> {
    if let Some(stop) = control.checkpoint() {
        return Err(stop);
    }
    if !area.is_finite() || area <= 0. {
        return Ok(None);
    }
    let mut stopped = None;
    distances.sort_by(|a, b| {
        if stopped.is_none() {
            stopped = control.charge_operations(1).err();
        }
        a.distance
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.distance.unwrap_or(f64::INFINITY))
    });
    if let Some(stop) = stopped {
        return Err(stop);
    }
    let mut available = 0.;
    for sample in distances.iter() {
        control.charge_operations(1)?;
        if sample.distance.is_some() {
            available += sample.weight;
        }
    }
    let available = available.min(area) / area;
    let endpoint = available.min(ceiling);
    let mut best = None;
    for fraction in FRACTIONS.into_iter().chain([endpoint]) {
        if fraction <= 0. || fraction > endpoint + 1e-12 {
            continue;
        }
        let target = fraction * area;
        let mut weight = 0.;
        let mut squared = 0.;
        for sample in distances.iter() {
            control.charge_operations(1)?;
            let Some(distance) = sample.distance else {
                break;
            };
            let w = sample.weight.min((target - weight).max(0.));
            squared += distance * distance * w;
            weight += w;
            if weight >= target - area * 1e-12 {
                break;
            }
        }
        if weight < target - area * 1e-12 {
            continue;
        }
        let rms = (squared / target).sqrt();
        let cost = (rms * rms + 0.02f64.powi(2)).sqrt() / fraction.sqrt();
        if cost.is_finite() && best.is_none_or(|(_, _, held)| cost < held) {
            best = Some((fraction, rms, cost));
        }
    }
    Ok(best)
}

/// Compare cell populations rather than densely repeated sample counts.
fn supported_trim_alternative(
    distances: &[WeightedDistance],
    area: f64,
    best_fraction: f64,
    ceiling: f64,
) -> Option<(f64, f64)> {
    let available = distances
        .iter()
        .filter(|s| s.distance.is_some())
        .map(|s| s.weight)
        .sum::<f64>()
        .min(area);
    let target = available.min(ceiling * area);
    let mut best_cells = BTreeSet::new();
    let mut supported_cells = BTreeSet::new();
    let mut weight = 0.;
    let mut squared = 0.;
    for sample in distances {
        let Some(distance) = sample.distance else {
            break;
        };
        if weight >= target {
            break;
        }
        if weight < best_fraction * area {
            best_cells.insert(sample.cell);
        }
        supported_cells.insert(sample.cell);
        let w = sample.weight.min(target - weight);
        squared += w * distance * distance;
        weight += w;
    }
    if target <= 0. || supported_cells.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let difference =
        supported_cells.difference(&best_cells).count() as f64 / supported_cells.len() as f64;
    (difference > 0.20).then(|| (target / area, (squared / target).sqrt()))
}

/// Complete controlled queries on each declared representation; interrupted
/// upper bounds never become distances. Training evidence remains unverified.
pub(crate) fn score_common_region(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    config: CoarseScoring<'_>,
    control: &GeometryControl,
) -> Result<CoarseScore, GeometryStop> {
    let policy = config.policy;
    let ceiling = config.ceiling;
    let capacity = config.samples_per_side.min(1_024);
    let (moving_index, fixed_index) = config
        .proxies
        .unwrap_or((&moving.original_index, &fixed.original_index));
    let exclude_border = config.proxies.is_none();
    let _memory = control.reserve(capacity * (2 * size_of::<WeightedDistance>() + 128) + 128)?;
    let mut forward = directional_support(
        moving,
        fixed_index,
        pose,
        policy,
        capacity,
        exclude_border && fixed.exact_original,
        control,
    )?;
    let mut reverse = directional_support(
        fixed,
        moving_index,
        pose.inverse(),
        policy,
        capacity,
        exclude_border && moving.exact_original,
        control,
    )?;
    if config.proxies.is_some()
        || !moving.quality.orientation_coherent
        || !fixed.quality.orientation_coherent
    {
        for sample in forward.iter_mut().chain(&mut reverse) {
            control.charge_operations(1)?;
            sample.compatible = None;
        }
    }
    let areas = [moving.eligible_area_mm2, fixed.eligible_area_mm2];
    let smaller = areas[0].min(areas[1]);
    let coverage = |band| {
        [
            covered(&forward, band) / areas[0],
            covered(&reverse, band) / areas[1],
        ]
        .map(|v| v.clamp(0., 1.))
    };
    let coverage_02 = coverage(0.2);
    let coverage_05 = coverage(0.5);
    let common_area = common_area(areas, coverage_02);
    let soft = |distances: &[WeightedDistance]| {
        distances
            .iter()
            .map(|s| s.distance.map_or(0., |d| (1. - d * d).max(0.) * s.weight))
            .sum::<f64>()
    };
    let lcp = (soft(&forward).min(soft(&reverse)) / smaller).clamp(0., 1.);
    let policy_support = [&forward, &reverse].map(|samples| policy_covered_area(samples));
    let (distances, area) = if areas[0] <= areas[1] {
        (&mut forward, areas[0])
    } else {
        (&mut reverse, areas[1])
    };
    let prefix = weighted_trim_sweep(distances, area, ceiling, control)?;
    let supported_prefix = if config.proxies.is_none() {
        prefix.and_then(|p| supported_trim_alternative(distances, area, p.0, ceiling))
    } else {
        None
    };
    let oriented_area: f64 = distances
        .iter()
        .filter(|s| s.distance.is_some_and(|d| d <= 0.5) && s.compatible.is_some())
        .map(|s| s.weight)
        .sum();
    let compatible_area: f64 = distances
        .iter()
        .filter(|s| s.distance.is_some_and(|d| d <= 0.5) && s.compatible == Some(true))
        .map(|s| s.weight)
        .sum();
    Ok(CoarseScore {
        score: prefix.map_or(0., |(_, _, cost)| lcp / (1. + cost / 0.20)),
        common_area,
        overlap: (common_area / smaller).clamp(0., 1.),
        coverage_02,
        coverage_05,
        fraction: prefix.map_or(0., |p| p.0),
        rms: prefix.map(|p| p.1),
        orientation: (oriented_area > 0.).then(|| compatible_area / oriented_area),
        supported_prefix,
        population_area: [
            moving.samples[0].population_area_mm2,
            fixed.samples[0].population_area_mm2,
        ],
        policy_support,
    })
}

fn policy_covered_area(samples: &[WeightedDistance]) -> Option<f64> {
    let oriented: f64 = samples
        .iter()
        .filter(|s| s.distance.is_some_and(|d| d <= 0.5) && s.compatible.is_some())
        .map(|s| s.weight)
        .sum();
    (oriented > 0.).then(|| {
        samples
            .iter()
            .filter(|s| s.distance.is_some_and(|d| d <= 0.5) && s.compatible == Some(true))
            .map(|s| s.weight)
            .sum()
    })
}

fn covered(samples: &[WeightedDistance], band: f64) -> f64 {
    samples
        .iter()
        .filter(|s| s.distance.is_some_and(|d| d <= band))
        .map(|s| s.weight)
        .sum()
}

/// RMS displacement on at most 256 fixed probes, together with rotation angle.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn pose_distance(left: Rigid, right: Rigid, probes: &[SurfaceSample]) -> [f64; 2] {
    let count = probes.len().min(256);
    let mut squared = 0.;
    for i in 0..count {
        let p = probes[i * probes.len() / count].point;
        squared += left.apply(p).distance_squared(right.apply(p));
    }
    [
        (squared / count.max(1) as f64).sqrt(),
        2. * left
            .rotation
            .dot(right.rotation)
            .abs()
            .clamp(0., 1.)
            .acos()
            .to_degrees(),
    ]
}

pub(crate) fn proposal_order(left: &Proposal, right: &Proposal) -> Ordering {
    right
        .score
        .score
        .total_cmp(&left.score.score)
        .then_with(|| right.score.common_area.total_cmp(&left.score.common_area))
        .then_with(|| {
            left.score
                .rms
                .unwrap_or(f64::INFINITY)
                .total_cmp(&right.score.rms.unwrap_or(f64::INFINITY))
        })
        .then_with(|| left.id.cmp(&right.id))
}

/// Only a completed, comparable training pass may replace the independent
/// global checkpoint. Interrupted or worse passes preserve it unchanged.
pub(crate) fn retain_best(best: &mut Option<Proposal>, completed: Option<&Proposal>) {
    if let Some(proposal) = completed {
        if best
            .as_ref()
            .is_none_or(|held| proposal_order(proposal, held).is_lt())
        {
            *best = Some(proposal.clone());
        }
    }
}

/// Select 12 scored basins plus four family/rival alternatives. Publication's
/// top-k is deliberately absent: every caller refines the same internal pool.
/// The best examined checkpoint remains separately owned by `ProposalSearch`.
pub(crate) fn refinement_candidates(pool: &[Proposal], policy: NormalPolicy) -> Vec<Proposal> {
    let mut order: Vec<_> = pool.iter().collect();
    order.sort_by(|a, b| proposal_order(a, b));
    let mut selected: Vec<_> = order.iter().take(12).map(|p| (*p).clone()).collect();
    if policy != NormalPolicy::Unsigned {
        for p in order
            .iter()
            .filter(|p| {
                p.score.orientation.is_some_and(|f| f >= 0.75)
                    && p.score
                        .policy_support
                        .iter()
                        .all(|a| a.is_some_and(|a| a > 0.))
            })
            .take(2)
        {
            if !selected.iter().any(|held| held.id == p.id) {
                selected.push((*p).clone());
            }
        }
    }
    let mut groups = [false; 6];
    for p in &selected {
        groups[quota_group(p.id.family)] = true;
    }
    for p in &order {
        let group = quota_group(p.id.family);
        if selected.len() >= 16 {
            break;
        }
        if !groups[group] && !selected.iter().any(|held| held.id == p.id) {
            selected.push((*p).clone());
            groups[group] = true;
        }
    }
    for p in order {
        if selected.len() >= 16 {
            break;
        }
        if !selected.iter().any(|held| held.id == p.id) {
            selected.push(p.clone());
        }
    }
    selected
}

fn quota_group(family: u8) -> usize {
    match family {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        5 => 3,
        6 => 4,
        _ => 5,
    }
}
const QUOTAS: [usize; 6] = [1, 4, 8, 4, 8, 4];

/// One bounded 32-basin pool. Quota shortages can evict only surplus groups;
/// duplicate proposals accumulate origins without growing a second pose pool.
pub(crate) fn insert_candidate(
    pool: &mut Vec<Proposal>,
    mut candidate: Proposal,
    probes: &[SurfaceSample],
    control: &GeometryControl,
    policy: NormalPolicy,
) -> Result<(), GeometryStop> {
    if !candidate.pose.is_finite() {
        return Ok(());
    }
    for held in pool.iter_mut() {
        // Both conditions are required for merging. A distinct rotation proves
        // separation without evaluating hundreds of transformed probes.
        let angle = 2.
            * held
                .pose
                .rotation
                .dot(candidate.pose.rotation)
                .abs()
                .clamp(0., 1.)
                .acos()
                .to_degrees();
        if angle >= 1. || probes.is_empty() {
            continue;
        }
        control.charge_point_pairs(probes.len().min(256) as u64)?;
        let distance = pose_distance(held.pose, candidate.pose, probes);
        if distance[0] < 0.2 && distance[1] < 1. {
            if proposal_order(&candidate, held).is_lt() {
                std::mem::swap(held, &mut candidate);
            }
            for origin in candidate.origins {
                if !held.origins.contains(&origin) {
                    held.origins.push(origin);
                }
            }
            pool.sort_by(proposal_order);
            return Ok(());
        }
    }
    pool.push(candidate);
    pool.sort_by(proposal_order);
    if pool.len() > 32 {
        let mut counts = [0; 6];
        for proposal in pool.iter() {
            counts[quota_group(proposal.id.family)] += 1;
        }
        let signed = |p: &Proposal| {
            policy != NormalPolicy::Unsigned
                && p.score.orientation.is_some_and(|f| f >= 0.75)
                && p.score
                    .policy_support
                    .iter()
                    .all(|a| a.is_some_and(|a| a > 0.))
        };
        let compatible = pool.iter().filter(|p| signed(p)).count();
        let unsigned = pool.len() - compatible;
        let eligible = |p: &Proposal| {
            if signed(p) {
                compatible > 4
            } else {
                unsigned > 4
            }
        };
        let drop = pool
            .iter()
            .rposition(|p| {
                let g = quota_group(p.id.family);
                counts[g] > QUOTAS[g] && eligible(p)
            })
            .or_else(|| pool.iter().rposition(eligible))
            .unwrap_or(pool.len() - 1);
        pool.remove(drop);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_area_cannot_shrink_fraction_denominator() {
        let mut d = [
            WeightedDistance {
                distance: Some(0.01),
                weight: 3.,
                compatible: None,
                cell: [0; 3],
            },
            WeightedDistance {
                distance: None,
                weight: 97.,
                compatible: None,
                cell: [1; 3],
            },
        ];
        let control = GeometryControl::unlimited();
        let p = weighted_trim_sweep(&mut d, 100., 1., &control)
            .unwrap()
            .unwrap();
        assert!((p.0 - 0.03).abs() < 1e-12);
        assert!(p.2 > 0.1);
        d[0].distance = None;
        assert!(weighted_trim_sweep(&mut d, 100., 1., &control)
            .unwrap()
            .is_none());
        assert!((common_area([100., 1_000.], [1., 0.1]) - 100.).abs() < 1e-12);
    }

    #[test]
    fn trim_sweep_counts_sorting_and_stops_without_a_prefix() {
        let mut distances: Vec<_> = (0..128u32)
            .map(|i| WeightedDistance {
                distance: Some(f64::from(127 - i)),
                weight: 1.,
                compatible: None,
                cell: [u64::from(i), 0, 0],
            })
            .collect();
        let control = GeometryControl::new(
            crate::CancelFlag::new(),
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits {
                operations: 3,
                ..occluview_geometry::surface::GeometryLimits::default()
            },
        );
        assert_eq!(
            weighted_trim_sweep(&mut distances, 128., 1., &control),
            Err(GeometryStop::WorkLimit)
        );
        assert_eq!(control.counters().operations, 3);
        let cancel = crate::CancelFlag::new();
        cancel.cancel();
        let cancelled = GeometryControl::new(
            cancel,
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits::default(),
        );
        assert_eq!(
            weighted_trim_sweep(&mut distances, 128., 1., &cancelled),
            Err(GeometryStop::Cancelled)
        );
    }

    #[test]
    fn proxy_target_cannot_supply_original_orientation_evidence() {
        let mesh = crate::proposal_test_support::arch::plane();
        let control = GeometryControl::unlimited();
        let prepare = |side| {
            crate::prepare_alignment_surface(
                crate::MeshInput {
                    soup: mesh.soup(),
                    world_from_local: glam::DAffine3::IDENTITY,
                    revision: 1,
                },
                side,
                crate::RegionPolicy::AllEligible,
                &control,
            )
            .unwrap()
            .surface
            .unwrap()
        };
        let moving = prepare(crate::SurfaceSide::Moving);
        let mut fixed = prepare(crate::SurfaceSide::Fixed);
        fixed.exact_original = false;
        fixed.quality.orientation_coherent = false;
        for batch in &mut fixed.samples {
            for sample in &mut batch.samples {
                sample.normal = None;
            }
        }
        let score = score_common_region(
            &moving,
            &fixed,
            Rigid::IDENTITY,
            CoarseScoring {
                policy: NormalPolicy::Match,
                samples_per_side: 128,
                ceiling: 1.,
                proxies: None,
            },
            &control,
        )
        .unwrap();
        assert!(score.rms.is_some());
        assert_eq!(score.orientation, None);
        assert_eq!(score.policy_support, [None; 2]);
    }

    #[test]
    fn larger_prefix_is_retained_only_for_distinct_common_cells() {
        let mut distances = (0..100u64)
            .map(|i| WeightedDistance {
                distance: Some(if i < 10 { 0. } else { 0.5 }),
                weight: 1.,
                compatible: None,
                cell: [i, 0, 0],
            })
            .collect::<Vec<_>>();
        let best = weighted_trim_sweep(&mut distances, 100., 1., &GeometryControl::unlimited())
            .unwrap()
            .unwrap();
        assert!((best.0 - 0.10).abs() < 1e-12);
        let alternative = supported_trim_alternative(&distances, 100., best.0, 1.).unwrap();
        assert!((alternative.0 - 1.).abs() < 1e-12);
        assert!((alternative.1 - 0.9f64.sqrt() * 0.5).abs() < 1e-12);
        for distance in &mut distances {
            distance.cell = [0; 3];
        }
        assert!(supported_trim_alternative(&distances, 100., best.0, 1.).is_none());
    }

    #[test]
    fn quota_pool_keeps_sparse_families_and_distinct_rivals() {
        use glam::{DQuat, DVec3};
        let probe = SurfaceSample {
            id: 0,
            point: DVec3::new(5., 7., 11.),
            normal: Some(DVec3::Z),
            triangle: 0,
            barycentric: [1. / 3.; 3],
            area_weight_mm2: 100.,
            region_id: 0,
        };
        let mut pool = Vec::with_capacity(33);
        let mut ordinal = 0u32;
        for (family, origin, count) in [
            (0, SeedOrigin::Start, 1),
            (1, SeedOrigin::Landmarks, 4),
            (3, SeedOrigin::FeatureSame, 8),
            (5, SeedOrigin::PrincipalFrame, 4),
            (7, SeedOrigin::CongruentBasis, 4),
            (6, SeedOrigin::GridPatch, 100),
        ] {
            for i in 0..count {
                let score = CoarseScore {
                    score: if family == 6 { 0.9 } else { 0.2 },
                    common_area: 100.,
                    overlap: 1.,
                    coverage_02: [1.; 2],
                    coverage_05: [1.; 2],
                    fraction: 1.,
                    rms: Some(0.01),
                    orientation: Some(1.),
                    supported_prefix: None,
                    population_area: [100.; 2],
                    policy_support: [Some(100.); 2],
                };
                let candidate = Proposal {
                    id: CandidateId {
                        family,
                        proposal: i,
                    },
                    pose: Rigid::new(DQuat::IDENTITY, DVec3::X * f64::from(ordinal) * 2.),
                    origins: vec![origin],
                    score,
                };
                insert_candidate(
                    &mut pool,
                    candidate,
                    &[probe],
                    &GeometryControl::unlimited(),
                    NormalPolicy::Unsigned,
                )
                .unwrap();
                ordinal += 1;
            }
        }
        assert_eq!(pool.len(), 32);
        for (group, quota) in QUOTAS.into_iter().enumerate() {
            assert!(
                pool.iter()
                    .filter(|p| quota_group(p.id.family) == group)
                    .count()
                    >= quota
            );
        }
        let mut repeated = pool[0].clone();
        repeated.id = CandidateId {
            family: 7,
            proposal: 999,
        };
        repeated.origins = vec![SeedOrigin::CongruentBasis];
        insert_candidate(
            &mut pool,
            repeated,
            &[probe],
            &GeometryControl::unlimited(),
            NormalPolicy::Unsigned,
        )
        .unwrap();
        assert_eq!(pool.len(), 32);
        assert!(pool[0].origins.contains(&SeedOrigin::CongruentBasis));
    }
    #[test]
    fn signed_pool_retains_policy_support_and_unsigned_rivals() {
        use glam::{DQuat, DVec3};
        let control = GeometryControl::unlimited();
        let mut pool = Vec::new();
        for i in 0..64u32 {
            let supported = i >= 60;
            let score = CoarseScore {
                score: if supported { 0.3 } else { 0.9 },
                common_area: 100.,
                overlap: 1.,
                coverage_02: [1.; 2],
                coverage_05: [1.; 2],
                fraction: 1.,
                rms: Some(0.01),
                orientation: Some(if supported { 1. } else { 0. }),
                population_area: [100.; 2],
                policy_support: [Some(if supported { 100. } else { 0. }); 2],
                supported_prefix: None,
            };
            let candidate = Proposal {
                id: CandidateId {
                    family: 6,
                    proposal: i,
                },
                pose: Rigid::new(DQuat::IDENTITY, DVec3::X * f64::from(i)),
                origins: vec![SeedOrigin::GridPatch],
                score,
            };
            insert_candidate(&mut pool, candidate, &[], &control, NormalPolicy::Opposed).unwrap();
        }
        assert_eq!(pool.len(), 32);
        assert_eq!(
            pool.iter()
                .filter(|p| p.score.orientation == Some(1.))
                .count(),
            4
        );
        assert!(pool.iter().any(|p| p.score.orientation == Some(0.)));
        let refinement = refinement_candidates(&pool, NormalPolicy::Opposed);
        assert_eq!(refinement.len(), 16);
        assert!(refinement.iter().any(|p| p.score.orientation == Some(1.)));
    }

    fn candidate(ordinal: u32, score: f64) -> Proposal {
        Proposal {
            id: CandidateId {
                family: 6,
                proposal: ordinal,
            },
            pose: Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::X * f64::from(ordinal)),
            origins: vec![SeedOrigin::GridPatch],
            score: CoarseScore {
                score,
                common_area: 100.,
                overlap: 1.,
                coverage_02: [1.; 2],
                coverage_05: [1.; 2],
                fraction: 1.,
                rms: Some(0.01),
                orientation: None,
                population_area: [100.; 2],
                policy_support: [None; 2],
                supported_prefix: None,
            },
        }
    }

    #[test]
    fn internal_shortlist_survives_publication_caps() {
        let pool: Vec<_> = (0..32)
            .map(|i| candidate(i, 0.9 - f64::from(i) * 0.01))
            .collect();
        let mut refinement = refinement_candidates(&pool, NormalPolicy::Unsigned);
        assert_eq!(refinement.len(), 16);
        // Rank seven before numerical work becomes the best completed result.
        let rank_seven = refinement[6].id;
        refinement[6].score.score = 1.;
        refinement.sort_by(proposal_order);
        for public_top_k in 1..=5 {
            assert_eq!(
                refinement.iter().take(public_top_k).next().unwrap().id,
                rank_seven
            );
        }
    }

    #[test]
    fn completed_checkpoint_survives_worse_and_interrupted_passes() {
        let initial = candidate(0, 0.9);
        let worse = candidate(1, 0.3);
        let better = candidate(2, 1.);
        let mut best = None;
        retain_best(&mut best, Some(&initial));
        retain_best(&mut best, Some(&worse));
        retain_best(&mut best, None);
        assert_eq!(best.as_ref().unwrap().id, initial.id);
        assert_eq!(best.as_ref().unwrap().score.score, initial.score.score);
        retain_best(&mut best, Some(&better));
        assert_eq!(best.as_ref().unwrap().id, better.id);
    }
}
