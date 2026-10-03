//! Controlled multiscale surface refinement, independently derived from
//! Chetverikov et al., The Trimmed Iterative Closest Point algorithm (2002),
//! <https://doi.org/10.1109/ICPR.2002.1047997>, and
//! Phillips, Liu and Tomasi (2006), <https://arxiv.org/abs/cs/0606098>.
//! Area prefixes replace vertex counts. A frozen robust local objective chooses
//! steps; the same bidirectional fractional score compares every checkpoint.
//! Local convergence supplies no independent confidence or uniqueness.

use super::icp_surface_step::{
    accumulate_robust, frozen_objective, line_search, PlaneInformation, SurfacePair,
};
use crate::candidate_score::{
    pose_distance, proposal_order, score_common_region, weighted_trim_sweep, CoarseScoring,
    Proposal, WeightedDistance,
};
use crate::{NormalPolicy, PreparedSurface, RefinementTermination, Rigid, SearchSettings};
use occluview_geometry::surface::{
    GeometryControl, GeometryStop, QueryOutcome, SurfaceQueryScratch,
};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(crate) struct RefinedProposal {
    pub proposal: Proposal,
    pub termination: RefinementTermination,
    pub information: Option<PlaneInformation>,
    pub unsigned_fallback: bool,
    /// Resolutions already attempted in this basin, independent of which
    /// completed checkpoint wins. This never certifies its residuals.
    pub attempted_scales: u8,
}

pub(crate) struct RefinementBatch {
    pub candidates: Vec<RefinedProposal>,
    pub iterations: u64,
    pub scored: u64,
    pub basins: [u32; 3],
    pub stop: Option<GeometryStop>,
}

#[derive(Clone, Copy)]
struct Gathered {
    pair: Option<SurfacePair>,
    distance: WeightedDistance,
}

#[derive(Clone, Copy)]
struct Scale {
    slot: usize,
    reach: f64,
    iterations: usize,
    basins: usize,
}
const SCALES: [Scale; 3] = [
    Scale {
        slot: 0,
        reach: 4.,
        iterations: 12,
        basins: 16,
    },
    Scale {
        slot: 1,
        reach: 1.,
        iterations: 15,
        basins: 8,
    },
    Scale {
        slot: 2,
        reach: 0.3,
        iterations: 20,
        basins: 5,
    },
];

/// Retain all completed basins until publication. The 16 -> 8 -> 5 itinerary
/// is independent of public top-k. Earlier better checkpoints survive any
/// later stall, unsupported dense pass, rejected trial or interruption.
#[expect(
    clippy::too_many_lines,
    reason = "one bounded itinerary with transactional scores and two overlap hypotheses per basin"
)]
pub(crate) fn run_multiscale(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    proposals: Vec<Proposal>,
    settings: &SearchSettings,
    control: &GeometryControl,
) -> RefinementBatch {
    let mut batch = RefinementBatch {
        candidates: proposals
            .into_iter()
            .map(|proposal| RefinedProposal {
                proposal,
                termination: RefinementTermination::NotStarted,
                information: None,
                unsigned_fallback: false,
                attempted_scales: 0,
            })
            .collect(),
        iterations: 0,
        scored: 0,
        basins: [0; 3],
        stop: None,
    };
    let scratch = (moving.samples[2].samples.len() + fixed.samples[2].samples.len())
        .saturating_mul(512)
        .saturating_add(4096);
    let _memory = match control.reserve(scratch) {
        Ok(memory) => memory,
        Err(stop) => {
            batch.stop = Some(stop);
            return batch;
        }
    };
    // Transactional rescoring: interruption must not mix proposal quadratures.
    let mut scores = Vec::with_capacity(batch.candidates.len());
    for held in &batch.candidates {
        match rescore(moving, fixed, held.proposal.pose, settings, control) {
            Ok(score) => {
                batch.scored += 1;
                scores.push(score);
            }
            Err(stop) => {
                batch.stop = Some(stop);
                return batch;
            }
        }
    }
    for (held, score) in batch.candidates.iter_mut().zip(scores) {
        held.proposal.score = score;
    }
    batch
        .candidates
        .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
    let mut leading_alternative = batch
        .candidates
        .first()
        .cloned()
        .filter(|p| p.proposal.score.supported_prefix.is_some());
    // Deliver a fully examined leading checkpoint before weaker basins can
    // consume the allowance. Each resolution is still charged once to its
    // 16/8/5 itinerary; all other basins retain their completed checkpoints.
    if !batch.candidates.is_empty() {
        for scale in SCALES {
            if let Some(stop) = refinement_admission(control) {
                batch.stop = Some(stop);
                return batch;
            }
            batch.basins[scale.slot] += 1;
            let held = &mut batch.candidates[0];
            if let Err(stop) = refine_candidate(
                moving,
                fixed,
                held,
                scale,
                settings,
                control,
                &mut batch.iterations,
                &mut batch.scored,
                None,
            ) {
                held.termination = stop_termination(stop);
                batch.stop = Some(stop);
                return batch;
            }
            held.attempted_scales |= 1 << scale.slot;
            if matches!(
                held.termination,
                RefinementTermination::NoCorrespondences
                    | RefinementTermination::Singular
                    | RefinementTermination::NumericalTrialRejected
            ) {
                break;
            }
        }
    }
    if let Some(mut alternative) = leading_alternative.take() {
        if let Some((fraction, _)) = alternative.proposal.score.supported_prefix {
            if let Some(id) = alternative.proposal.id.proposal.checked_add(1 << 31) {
                alternative.proposal.id.proposal = id;
                let outcome = refine_candidate(
                    moving,
                    fixed,
                    &mut alternative,
                    SCALES[0],
                    settings,
                    control,
                    &mut batch.iterations,
                    &mut batch.scored,
                    Some(fraction),
                );
                alternative.attempted_scales |= 1;
                batch.candidates.push(alternative);
                if let Err(stop) = outcome {
                    batch.stop = Some(stop);
                    return batch;
                }
            }
        }
    }
    for scale in SCALES {
        batch
            .candidates
            .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
        let indices: Vec<_> = batch
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, held)| held.attempted_scales & (1 << scale.slot) == 0)
            .take(
                scale
                    .basins
                    .saturating_sub(batch.basins[scale.slot] as usize),
            )
            .map(|(i, _)| i)
            .collect();
        let mut alternatives = Vec::new();
        for i in indices {
            // Final evidence owns the reserved quarter. This admission stop
            // does not poison the geometry control needed by verification.
            if let Some(stop) = refinement_admission(control) {
                batch.stop = Some(stop);
                return batch;
            }
            batch.basins[scale.slot] += 1;
            let mut alternative = (scale.slot == 0).then(|| batch.candidates[i].clone());
            let held = &mut batch.candidates[i];
            match refine_candidate(
                moving,
                fixed,
                held,
                scale,
                settings,
                control,
                &mut batch.iterations,
                &mut batch.scored,
                None,
            ) {
                Ok(()) => {}
                Err(stop) => {
                    held.termination = stop_termination(stop);
                    batch.stop = Some(stop);
                    return batch;
                }
            }
            held.attempted_scales |= 1 << scale.slot;
            if let Some(mut alternative) = alternative.take() {
                if let Some((fraction, _)) = alternative.proposal.score.supported_prefix {
                    if let Some(id) = alternative.proposal.id.proposal.checked_add(1 << 31) {
                        alternative.proposal.id.proposal = id;
                        let outcome = refine_candidate(
                            moving,
                            fixed,
                            &mut alternative,
                            scale,
                            settings,
                            control,
                            &mut batch.iterations,
                            &mut batch.scored,
                            Some(fraction),
                        );
                        alternative.attempted_scales |= 1 << scale.slot;
                        alternatives.push(alternative);
                        if let Err(stop) = outcome {
                            batch.candidates.extend(alternatives);
                            batch.stop = Some(stop);
                            return batch;
                        }
                    }
                }
            }
        }
        batch.candidates.extend(alternatives);
        if let Err(stop) = coalesce_basins(&mut batch.candidates, moving, control) {
            batch.stop = Some(stop);
            return batch;
        }
    }
    batch
        .candidates
        .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
    batch
}

fn coalesce_basins(
    candidates: &mut Vec<RefinedProposal>,
    moving: &PreparedSurface,
    control: &GeometryControl,
) -> Result<(), GeometryStop> {
    candidates.sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
    let mut drop = Vec::new();
    drop.try_reserve_exact(candidates.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    // Mark first, mutate only after the entire comparison pass completes.
    // A canceled pass retains every prior pose/evidence checkpoint.
    for i in 0..candidates.len() {
        for j in 0..i {
            if drop.iter().any(|&(slot, _)| slot == j) {
                continue;
            }
            let a = &candidates[j].proposal;
            let b = &candidates[i].proposal;
            control.charge_point_pairs(moving.samples[0].samples.len().min(256) as u64)?;
            let distance = pose_distance(a.pose, b.pose, &moving.samples[0].samples);
            if distance[0] < 0.2
                && distance[1] < 1.
                && crate::candidate_score::same_common_region(
                    &a.score.common_cells,
                    &b.score.common_cells,
                    control,
                )?
            {
                drop.push((i, j));
                break;
            }
        }
    }
    for &(from, to) in &drop {
        candidates[to].attempted_scales |= candidates[from].attempted_scales;
        let origins = candidates[from].proposal.origins.clone();
        for origin in origins {
            if !candidates[to].proposal.origins.contains(&origin) {
                candidates[to].proposal.origins.push(origin);
            }
        }
    }
    for &(from, _) in drop.iter().rev() {
        candidates.remove(from);
    }
    Ok(())
}

fn refinement_admission(control: &GeometryControl) -> Option<GeometryStop> {
    let counters = control.counters();
    let limits = control.limits();
    control.checkpoint().or_else(|| {
        (counters.query_calls >= limits.query_calls - limits.query_calls.div_ceil(4)
            || counters.triangle_tests >= limits.triangle_tests - limits.triangle_tests.div_ceil(4))
        .then_some(GeometryStop::WorkLimit)
    })
}
fn stop_termination(stop: GeometryStop) -> RefinementTermination {
    match stop {
        GeometryStop::Cancelled => RefinementTermination::Cancelled,
        GeometryStop::Deadline => RefinementTermination::Deadline,
        GeometryStop::WorkLimit | GeometryStop::ResourceLimit => {
            RefinementTermination::IterationLimit
        }
        GeometryStop::Numerical => RefinementTermination::NumericalTrialRejected,
    }
}
fn rescore(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    settings: &SearchSettings,
    control: &GeometryControl,
) -> Result<crate::candidate_score::CoarseScore, GeometryStop> {
    score_common_region(
        moving,
        fixed,
        pose,
        CoarseScoring {
            policy: settings.normal_policy,
            samples_per_side: 256,
            ceiling: settings.overlap_prior.unwrap_or(1.),
            proxies: None,
        },
        control,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "bounded per-basin solve with an independently retained comparable checkpoint"
)]
fn refine_candidate(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    held: &mut RefinedProposal,
    scale: Scale,
    settings: &SearchSettings,
    control: &GeometryControl,
    iterations: &mut u64,
    scored: &mut u64,
    initial_fraction: Option<f64>,
) -> Result<(), GeometryStop> {
    let phase = match scale.slot {
        0 => crate::search_probe::Phase::Coarse,
        1 => crate::search_probe::Phase::Middle,
        _ => crate::search_probe::Phase::Dense,
    };
    let _phase = crate::search_probe::Span::new(phase, control);
    let mut pose = held.proposal.pose;
    let mut fraction = initial_fraction;
    let reach = scale.reach * (settings.influence_radius_mm / 2.).clamp(0.25, 2.);
    let mut stable = 0;
    held.termination = RefinementTermination::IterationLimit;
    for iteration in 0..scale.iterations {
        if let Some(stop) = refinement_admission(control) {
            return Err(stop);
        }
        let (pairs, fallback) = correspondences(
            moving,
            fixed,
            pose,
            scale.slot,
            reach,
            settings,
            &mut fraction,
            iteration.is_multiple_of(3) && (iteration != 0 || initial_fraction.is_none()),
            control,
        )?;
        if pairs.len() < 6 {
            held.termination = RefinementTermination::NoCorrespondences;
            break;
        }
        let Some(model) = accumulate_robust(&pairs, pose, control)? else {
            held.termination = RefinementTermination::Singular;
            break;
        };
        *iterations += 1;
        // Information is paired with this pose, and never written into the
        // independent holdout evidence fields by this numerical pass.
        if pose == held.proposal.pose {
            held.information.clone_from(&model.information);
            held.unsigned_fallback |= fallback;
        }
        let before = frozen_objective(&pairs, pose, &model, control)?;
        let (next, termination) = line_search(&pairs, pose, &model, scale.slot == 2, control)?;
        if termination != RefinementTermination::NotStarted {
            held.termination = termination;
            break;
        }
        let after = frozen_objective(&pairs, next, &model, control)?;
        control.charge_point_pairs(moving.samples[0].samples.len().min(256) as u64)?;
        let displacement = pose_distance(pose, next, &moving.samples[0].samples)[0];
        let score = rescore(moving, fixed, next, settings, control)?;
        *scored += 1;
        let next_proposal = Proposal {
            pose: next,
            score,
            ..held.proposal.clone()
        };
        if proposal_order(&next_proposal, &held.proposal).is_lt() {
            held.proposal = next_proposal;
            // Recompute at the best checkpoint on the next iteration; no
            // pre-step matrix masquerades as post-step information.
            held.information = None;
            held.unsigned_fallback |= fallback;
        }
        pose = next;
        let relative = (before - after).abs() / before.max(1e-20);
        stable = if relative < 1e-4 && displacement < 0.01 {
            stable + 1
        } else {
            0
        };
        if stable >= 3 {
            held.termination = RefinementTermination::Stationary;
            break;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "directed immutable populations and one shared control"
)]
fn gather(
    source: &PreparedSurface,
    target: &PreparedSurface,
    pose: Rigid,
    slot: usize,
    reach: f64,
    reverse: bool,
    policy: NormalPolicy,
    control: &GeometryControl,
) -> Result<Vec<Gathered>, GeometryStop> {
    let samples = &source.samples[slot].samples;
    let mut result = Vec::new();
    result
        .try_reserve_exact(samples.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut scratch = SurfaceQueryScratch::new(control)?;
    for sample in samples {
        control.charge_operations(1)?;
        let point = pose.apply(sample.point);
        if !point.is_finite() {
            return Err(GeometryStop::Numerical);
        }
        let hit = if super::icp_overlap::outside_query_bounds(&target.original_index, point, reach)
        {
            None
        } else {
            match target
                .original_index
                .nearest_with_scratch(point, reach, &mut scratch)
            {
                QueryOutcome::Complete(hit) => {
                    hit.filter(|h| !target.exact_original || !h.on_border)
                }
                QueryOutcome::Interrupted { reason, .. } => return Err(reason),
            }
        };
        let agreement = hit.and_then(|h| {
            sample
                .normal
                .filter(|_| target.quality.orientation_coherent)
                .map(|n| pose.apply_normal(n).dot(h.normal))
        });
        let compatible = agreement.map(|dot| match policy {
            NormalPolicy::Match => dot > 0.5,
            NormalPolicy::Opposed => dot < -0.5,
            NormalPolicy::Unsigned => true,
        });
        let pair = hit.map(|h| {
            if reverse {
                SurfacePair {
                    moving: h.point,
                    fixed: sample.point,
                    normal: sample
                        .normal
                        .filter(|_| target.quality.orientation_coherent),
                    weight: sample.area_weight_mm2,
                }
            } else {
                SurfacePair {
                    moving: sample.point,
                    fixed: h.point,
                    normal: agreement.map(|_| h.normal),
                    weight: sample.area_weight_mm2,
                }
            }
        });
        result.push(Gathered {
            pair,
            distance: WeightedDistance {
                distance: hit.map(|h| point.distance(h.point)),
                weight: sample.area_weight_mm2,
                compatible,
                cell: cell(sample.point),
            },
        });
    }
    Ok(result)
}
fn cell(point: glam::DVec3) -> [u64; 3] {
    point.to_array().map(|v| (v.floor() + 0.).to_bits())
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "explicit smaller-area prefix and matched-region reverse population"
)]
fn correspondences(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    pose: Rigid,
    slot: usize,
    reach: f64,
    settings: &SearchSettings,
    fraction: &mut Option<f64>,
    refresh: bool,
    control: &GeometryControl,
) -> Result<(Vec<SurfacePair>, bool), GeometryStop> {
    let forward = gather(
        moving,
        fixed,
        pose,
        slot,
        reach,
        false,
        settings.normal_policy,
        control,
    )?;
    let reverse = gather(
        fixed,
        moving,
        pose.inverse(),
        slot,
        reach,
        true,
        settings.normal_policy,
        control,
    )?;
    let moving_smaller = moving.eligible_area_mm2 <= fixed.eligible_area_mm2;
    let (mut smaller, other) = if moving_smaller {
        (forward, reverse)
    } else {
        (reverse, forward)
    };
    let area = moving.eligible_area_mm2.min(fixed.eligible_area_mm2);
    if refresh || fraction.is_none() {
        let mut distances: Vec<_> = smaller.iter().map(|p| p.distance).collect();
        control.charge_operations(distances.len() as u64)?;
        *fraction = weighted_trim_sweep(
            &mut distances,
            area,
            settings.overlap_prior.unwrap_or(1.),
            control,
        )?
        .map(|p| p.0);
    }
    let Some(q) = *fraction else {
        return Ok((Vec::new(), false));
    };
    let mut stopped = None;
    smaller.sort_by(|a, b| {
        if stopped.is_none() {
            stopped = control.charge_operations(1).err();
        }
        a.distance
            .distance
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.distance.distance.unwrap_or(f64::INFINITY))
    });
    if let Some(stop) = stopped {
        return Err(stop);
    }
    let target = q * area;
    let mut remaining = target;
    let mut cutoff = 0.;
    let mut pairs = Vec::new();
    pairs
        .try_reserve_exact(smaller.len() + other.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut cells = BTreeSet::new();
    let mut compatible = 0;
    for entry in &smaller {
        control.charge_operations(1)?;
        if remaining <= area * 1e-12 {
            break;
        }
        let Some(mut pair) = entry.pair else {
            break;
        };
        pair.weight = pair.weight.min(remaining);
        remaining -= pair.weight;
        cutoff = entry.distance.distance.unwrap_or(0.);
        cells.insert(entry.distance.cell);
        compatible += usize::from(entry.distance.compatible != Some(false));
        pairs.push((pair, entry.distance.compatible != Some(false)));
    }
    // Missing correspondence area cannot shrink a previously selected prefix.
    if remaining > area * 1e-12 {
        return Ok((Vec::new(), false));
    }
    for entry in &other {
        control.charge_operations(1)?;
        let Some(pair) = entry.pair else {
            continue;
        };
        let smaller_point = if moving_smaller {
            pair.moving
        } else {
            pair.fixed
        };
        if cells.contains(&cell(smaller_point))
            && entry.distance.distance.is_some_and(|d| d <= cutoff + 1e-9)
        {
            compatible += usize::from(entry.distance.compatible != Some(false));
            pairs.push((pair, entry.distance.compatible != Some(false)));
        }
    }
    let fallback = settings.normal_policy != NormalPolicy::Unsigned && compatible < 6;
    Ok((
        pairs
            .into_iter()
            .filter(|(_, okay)| fallback || *okay)
            .map(|(p, _)| p)
            .collect(),
        fallback,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateId, MeshInput, RegionPolicy, SeedOrigin, SurfaceSide};
    use glam::{DAffine3, DQuat, DVec3};

    fn surface(control: &GeometryControl) -> PreparedSurface {
        let mesh = crate::proposal_test_support::arch::dental_arch(
            &crate::proposal_test_support::arch::ArchSpec {
                grid: [40, 10],
                ..crate::proposal_test_support::arch::ArchSpec::default()
            },
        );
        crate::prepare_alignment_surface(
            MeshInput {
                soup: mesh.soup(),
                world_from_local: DAffine3::IDENTITY,
                revision: 1,
            },
            SurfaceSide::Moving,
            RegionPolicy::AllEligible,
            control,
        )
        .unwrap()
        .surface
        .unwrap()
    }
    fn proposal(
        surface: &PreparedSurface,
        pose: Rigid,
        id: u32,
        control: &GeometryControl,
    ) -> Proposal {
        Proposal {
            id: CandidateId {
                family: 6,
                proposal: id,
            },
            pose,
            origins: vec![SeedOrigin::GridPatch],
            score: rescore(surface, surface, pose, &SearchSettings::default(), control).unwrap(),
        }
    }
    #[test]
    fn distant_basin_uses_exact_bounds_without_spending_nearest_calls() {
        let control = GeometryControl::unlimited();
        let surface = surface(&control);
        let before = control.counters().query_calls;
        let gathered = gather(
            &surface,
            &surface,
            Rigid::new(DQuat::IDENTITY, DVec3::splat(1000.)),
            2,
            0.3,
            false,
            NormalPolicy::Unsigned,
            &control,
        )
        .unwrap();
        assert_eq!(gathered.len(), surface.samples[2].samples.len());
        assert!(gathered.iter().all(|pair| pair.pair.is_none()));
        assert_eq!(control.counters().query_calls, before);
    }

    #[test]
    fn converged_basins_share_later_work_without_consuming_rival_slots() {
        let control = GeometryControl::unlimited();
        let surface = surface(&control);
        let proposals = (0..16)
            .map(|i| proposal(&surface, Rigid::IDENTITY, i, &control))
            .collect();
        let result = run_multiscale(
            &surface,
            &surface,
            proposals,
            &SearchSettings::default(),
            &control,
        );
        assert!(result.stop.is_none());
        assert_eq!(result.basins, [16, 1, 1]);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].proposal.pose, Rigid::IDENTITY);
        assert!(result.candidates[0].information.is_some());
    }
    #[test]
    fn leading_basin_reaches_dense_before_distant_basins_exhaust_work() {
        let unlimited = GeometryControl::unlimited();
        let surface = surface(&unlimited);
        let proposals = (0..16)
            .map(|i| {
                proposal(
                    &surface,
                    if i == 0 {
                        Rigid::IDENTITY
                    } else {
                        Rigid::new(DQuat::IDENTITY, DVec3::Y * 100. * f64::from(i))
                    },
                    i,
                    &unlimited,
                )
            })
            .collect();
        let control = GeometryControl::new(
            crate::CancelFlag::new(),
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits {
                query_calls: 100_000,
                ..occluview_geometry::surface::GeometryLimits::default()
            },
        );
        let result = run_multiscale(
            &surface,
            &surface,
            proposals,
            &SearchSettings::default(),
            &control,
        );
        // Exact box rejection can complete the distant no-support basins
        // without consuming nearest-query admission. Both outcomes retain
        // the leading dense checkpoint and stay within the same hard cap.
        assert!(result
            .stop
            .is_none_or(|stop| stop == GeometryStop::WorkLimit));
        if result.stop.is_none() {
            assert_eq!(result.basins[0], 16);
            assert!(control.counters().query_calls <= 100_000);
        }
        assert!(result.basins[2] >= 1, "dense work={:?}", result.basins);
        assert!(result.basins[0] <= 16 && result.basins[1] <= 8 && result.basins[2] <= 5);
        let leading = result
            .candidates
            .iter()
            .find(|p| p.proposal.id.proposal == 0)
            .unwrap();
        assert_eq!(leading.proposal.pose, Rigid::IDENTITY);
        assert!(leading.information.is_some());
        assert!(control.counters().query_calls <= 100_000);
    }
    #[test]
    fn common_region_and_separated_rival_survive_coalescing() {
        let control = GeometryControl::unlimited();
        let surface = surface(&control);
        let make = |pose, id| RefinedProposal {
            proposal: proposal(&surface, pose, id, &control),
            termination: RefinementTermination::Stationary,
            information: None,
            unsigned_fallback: false,
            attempted_scales: 0,
        };
        let first = make(Rigid::IDENTITY, 0);
        let mut other_region = make(Rigid::IDENTITY, 1);
        other_region.proposal.score.common_cells = vec![[u64::MAX; 3]];
        let rival = make(Rigid::new(DQuat::IDENTITY, DVec3::Z * 2.), 2);
        let duplicate = make(Rigid::IDENTITY, 3);
        let mut candidates = vec![first, other_region, rival, duplicate];
        coalesce_basins(&mut candidates, &surface, &control).unwrap();
        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().any(|p| p.proposal.id.proposal == 1));
        assert!(candidates.iter().any(|p| p.proposal.id.proposal == 2));
        let cancel = crate::CancelFlag::new();
        cancel.cancel();
        let limited = GeometryControl::new(
            cancel,
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits::default(),
        );
        let before: Vec<_> = candidates
            .iter()
            .map(|p| (p.proposal.id, p.proposal.pose))
            .collect();
        assert_eq!(
            coalesce_basins(&mut candidates, &surface, &limited),
            Err(GeometryStop::Cancelled)
        );
        assert_eq!(
            before,
            candidates
                .iter()
                .map(|p| (p.proposal.id, p.proposal.pose))
                .collect::<Vec<_>>()
        );
    }
    /// D7: a seventh coarse basin survives, and public top-k never changes work.
    #[test]
    fn seventh_basin_survives_refinement_and_public_cap() {
        let mut reference = None;
        for top_k in [1, 5] {
            let control = GeometryControl::unlimited();
            let surface = surface(&control);
            let mut proposals: Vec<_> = (0..16)
                .map(|i| {
                    let pose = if i == 6 {
                        Rigid::IDENTITY
                    } else {
                        Rigid::new(DQuat::IDENTITY, DVec3::X * (f64::from(i) + 1.) * 0.15)
                    };
                    let mut p = proposal(&surface, pose, i, &control);
                    p.score.score = (16. - f64::from(i)) / 100.;
                    p
                })
                .collect();
            proposals.sort_by(proposal_order);
            assert_eq!(proposals[6].id.proposal, 6);
            let result = run_multiscale(
                &surface,
                &surface,
                proposals,
                &SearchSettings {
                    top_k,
                    ..SearchSettings::default()
                },
                &control,
            );
            assert!(result.stop.is_none());
            assert_eq!(result.basins[0], 16);
            assert!((1..=8).contains(&result.basins[1]));
            assert!((1..=5).contains(&result.basins[2]));
            assert!(result
                .candidates
                .iter()
                .any(|p| p.proposal.id.proposal == 6 && p.proposal.pose == Rigid::IDENTITY));
            let signature = (result.iterations, result.basins, control.counters());
            if let Some(reference) = reference {
                assert_eq!(reference, signature);
            } else {
                reference = Some(signature);
            }
        }
    }
    /// D10: a dense population failure cannot discard a completed good pose.
    #[test]
    fn unsupported_dense_pass_keeps_completed_checkpoint() {
        let control = GeometryControl::unlimited();
        let mut surface = surface(&control);
        let p = proposal(&surface, Rigid::IDENTITY, 0, &control);
        let before = p.score.score;
        surface.samples[2].samples.clear();
        let result = run_multiscale(
            &surface,
            &surface,
            vec![p],
            &SearchSettings::default(),
            &control,
        );
        assert!(result.stop.is_none());
        assert_eq!(result.candidates[0].proposal.pose, Rigid::IDENTITY);
        assert!(result.candidates[0].proposal.score.score >= before);
        assert_eq!(
            result.candidates[0].termination,
            RefinementTermination::NoCorrespondences
        );
        assert_eq!(result.basins, [1, 1, 1]);
    }
    /// D10: interruption during baseline rescoring is an atomic checkpoint.
    #[test]
    fn interrupted_rescore_preserves_comparable_prior_scores() {
        let control = GeometryControl::unlimited();
        let surface = surface(&control);
        let mut proposals = vec![
            proposal(&surface, Rigid::IDENTITY, 0, &control),
            proposal(&surface, Rigid::IDENTITY, 1, &control),
        ];
        proposals[0].score.score = 0.41;
        proposals[1].score.score = 0.42;
        let limited = GeometryControl::new(
            crate::CancelFlag::new(),
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits {
                query_calls: 600,
                ..occluview_geometry::surface::GeometryLimits::default()
            },
        );
        let result = run_multiscale(
            &surface,
            &surface,
            proposals,
            &SearchSettings::default(),
            &limited,
        );
        assert_eq!(result.stop, Some(GeometryStop::WorkLimit));
        assert_eq!(result.candidates[0].proposal.score.score, 0.41);
        assert_eq!(result.candidates[1].proposal.score.score, 0.42);
        assert_eq!(limited.counters().query_calls, 600);
    }

    /// Missing normals are uncertainty, not a conflict with unsigned policy.
    #[test]
    fn unsigned_vector_fallback_does_not_claim_policy_conflict() {
        let control = GeometryControl::unlimited();
        let mut surface = surface(&control);
        let p = proposal(&surface, Rigid::IDENTITY, 0, &control);
        surface.quality.orientation_coherent = false;
        for set in &mut surface.samples {
            for sample in &mut set.samples {
                sample.normal = None;
            }
        }
        let result = run_multiscale(
            &surface,
            &surface,
            vec![p],
            &SearchSettings {
                normal_policy: NormalPolicy::Unsigned,
                ..SearchSettings::default()
            },
            &control,
        );
        assert!(result.stop.is_none());
        assert!(result.candidates[0].information.is_none());
        assert!(!result.candidates[0].unsigned_fallback);
    }

    #[test]
    fn cancelled_refinement_reservation_preserves_stop_reason() {
        let unlimited = GeometryControl::unlimited();
        let surface = surface(&unlimited);
        let p = proposal(&surface, Rigid::IDENTITY, 0, &unlimited);
        let cancel = crate::CancelFlag::new();
        cancel.cancel();
        let control = GeometryControl::new(
            cancel,
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits::default(),
        );
        let result = run_multiscale(
            &surface,
            &surface,
            vec![p],
            &SearchSettings::default(),
            &control,
        );
        assert_eq!(result.stop, Some(GeometryStop::Cancelled));
        assert_eq!(result.candidates[0].proposal.pose, Rigid::IDENTITY);
    }
}
