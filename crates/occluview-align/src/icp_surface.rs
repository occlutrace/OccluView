//! Controlled multiscale surface refinement, independently derived from
//! Chetverikov et al., The Trimmed Iterative Closest Point algorithm (2002),
//! <https://doi.org/10.1109/ICPR.2002.1047997>, and
//! Phillips, Liu and Tomasi (2006), <https://arxiv.org/abs/cs/0606098>.
//! Area prefixes replace vertex counts. A frozen robust local objective chooses
//! steps; the same bidirectional fractional score compares every checkpoint.
//! Two common-region trajectories share a basin's coarse iteration allowance;
//! each receives half, while a single trajectory keeps the full allowance.
//! Local convergence supplies no independent confidence or uniqueness.

use super::icp_surface_step::{accumulate_robust, line_search, PlaneInformation, SurfacePair};
use crate::candidate_score::{
    pose_distance, proposal_order, score_common_region, weighted_trim_sweep, CoarseScoring,
    Proposal, WeightedDistance,
};
use crate::{NormalPolicy, PreparedSurface, RefinementTermination, Rigid, SearchSettings};
use occluview_geometry::surface::{
    GeometryControl, GeometryStop, QueryOutcome, SurfaceQueryHint, SurfaceQueryScratch,
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
    cursor: Option<RefinementCursor>,
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

fn trajectory_scale(mut scale: Scale, paired: bool) -> Scale {
    if scale.slot == 0 && paired {
        scale.iterations /= 2;
    }
    scale
}

/// Pending work retains its iteration state across local admission stops.
struct RefinementTask {
    id: crate::CandidateId,
    scale: Scale,
    fraction: Option<f64>,
    counted: bool,
    started: bool,
    state: Option<ScaleState>,
}

#[derive(Default)]
struct RefinementCursor {
    scores: Vec<crate::candidate_score::CoarseScore>,
    initialized: bool,
    phase: u8,
    tasks: std::collections::VecDeque<RefinementTask>,
    memory: Option<occluview_geometry::surface::GeometryMemory>,
}

struct ScaleState {
    pose: Rigid,
    fraction: Option<f64>,
    stable: u32,
    iteration: usize,
    hints: [Vec<Option<SurfaceQueryHint>>; 2],
    _memory: occluview_geometry::surface::GeometryMemory,
}

/// Refine the same internal pool independently of public top-k. Completed
/// iterations and comparable checkpoints survive local work admission stops.
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
        cursor: Some(RefinementCursor::default()),
    };
    resume_multiscale(moving, fixed, &mut batch, settings, control, true);
    batch
}

/// Continue pending numerical work after independent evidence releases its
/// reserved headroom. Global caps, cancellation and the wall deadline remain
/// shared. No completed iteration or scale is restarted.
#[expect(
    clippy::too_many_arguments,
    reason = "immutable populations, explicit cursor and evidence reservation"
)]
pub(crate) fn resume_multiscale(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    batch: &mut RefinementBatch,
    settings: &SearchSettings,
    control: &GeometryControl,
    reserve_evidence: bool,
) {
    let Some(mut cursor) = batch.cursor.take() else {
        return;
    };
    batch.stop = None;
    let scratch = (moving.samples[2].samples.len() + fixed.samples[2].samples.len())
        .saturating_mul(512)
        .saturating_add(4096);
    let outcome = if cursor.memory.is_none() {
        control
            .reserve(scratch)
            .map(|memory| cursor.memory = Some(memory))
    } else {
        Ok(())
    }
    .and_then(|()| {
        advance_itinerary(
            moving,
            fixed,
            batch,
            settings,
            control,
            reserve_evidence,
            &mut cursor,
        )
    });
    if let Err(stop) = outcome {
        batch.stop = Some(stop);
        batch.cursor = Some(cursor);
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "bounded deterministic itinerary and resumable transactional state"
)]
fn advance_itinerary(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    batch: &mut RefinementBatch,
    settings: &SearchSettings,
    control: &GeometryControl,
    reserve_evidence: bool,
    cursor: &mut RefinementCursor,
) -> Result<(), GeometryStop> {
    if !cursor.initialized {
        cursor
            .scores
            .try_reserve(batch.candidates.len().saturating_sub(cursor.scores.len()))
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for held in batch.candidates.iter().skip(cursor.scores.len()) {
            cursor.scores.push(rescore(
                moving,
                fixed,
                held.proposal.pose,
                settings,
                control,
            )?);
            batch.scored += 1;
        }
        for (held, score) in batch.candidates.iter_mut().zip(cursor.scores.drain(..)) {
            held.proposal.score = score;
        }
        batch
            .candidates
            .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
        if let Some(leading) = batch.candidates.first().cloned() {
            for scale in SCALES {
                enqueue(
                    cursor,
                    leading.proposal.id,
                    trajectory_scale(scale, leading.proposal.score.supported_prefix.is_some()),
                    None,
                    true,
                )?;
            }
            enqueue_alternative(batch, cursor, leading, SCALES[0])?;
        }
        cursor.initialized = true;
    }
    loop {
        if let Some(mut task) = cursor.tasks.pop_front() {
            if let Some(stop) = refinement_admission(control, reserve_evidence) {
                cursor.tasks.push_front(task);
                return Err(stop);
            }
            let Some(index) = batch
                .candidates
                .iter()
                .position(|p| p.proposal.id == task.id)
            else {
                return Err(GeometryStop::Numerical);
            };
            if !task.started {
                if task.counted {
                    batch.basins[task.scale.slot] += 1;
                }
                task.started = true;
            }
            let held = &mut batch.candidates[index];
            let outcome = refine_candidate(
                moving,
                fixed,
                held,
                task.scale,
                settings,
                control,
                &mut batch.iterations,
                &mut batch.scored,
                task.fraction,
                &mut task.state,
                reserve_evidence,
            );
            if let Err(stop) = outcome {
                held.termination = stop_termination(stop);
                cursor.tasks.push_front(task);
                return Err(stop);
            }
            held.attempted_scales |= 1 << task.scale.slot;
            if cursor.phase == 0
                && matches!(
                    held.termination,
                    RefinementTermination::NoCorrespondences
                        | RefinementTermination::Singular
                        | RefinementTermination::NumericalTrialRejected
                )
            {
                cursor
                    .tasks
                    .retain(|later| later.id != task.id || later.scale.slot == 0);
            }
            continue;
        }
        if cursor.phase != 0 {
            coalesce_basins(&mut batch.candidates, moving, control)?;
        }
        if cursor.phase >= 3 {
            batch
                .candidates
                .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
            return Ok(());
        }
        let scale = SCALES[usize::from(cursor.phase)];
        batch
            .candidates
            .sort_by(|a, b| proposal_order(&a.proposal, &b.proposal));
        let selected: Vec<_> = batch
            .candidates
            .iter()
            .filter(|held| held.attempted_scales & (1 << scale.slot) == 0)
            .take(
                scale
                    .basins
                    .saturating_sub(batch.basins[scale.slot] as usize),
            )
            .cloned()
            .collect();
        for held in selected {
            enqueue(
                cursor,
                held.proposal.id,
                trajectory_scale(scale, held.proposal.score.supported_prefix.is_some()),
                None,
                true,
            )?;
            if scale.slot == 0 {
                enqueue_alternative(batch, cursor, held, scale)?;
            }
        }
        cursor.phase += 1;
    }
}

fn enqueue(
    cursor: &mut RefinementCursor,
    id: crate::CandidateId,
    scale: Scale,
    fraction: Option<f64>,
    counted: bool,
) -> Result<(), GeometryStop> {
    cursor
        .tasks
        .try_reserve(1)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    cursor.tasks.push_back(RefinementTask {
        id,
        scale,
        fraction,
        counted,
        started: false,
        state: None,
    });
    Ok(())
}

fn enqueue_alternative(
    batch: &mut RefinementBatch,
    cursor: &mut RefinementCursor,
    mut held: RefinedProposal,
    scale: Scale,
) -> Result<(), GeometryStop> {
    if let Some((fraction, _)) = held.proposal.score.supported_prefix {
        if let Some(id) = held.proposal.id.proposal.checked_add(1 << 31) {
            held.proposal.id.proposal = id;
            batch
                .candidates
                .try_reserve(1)
                .map_err(|_| GeometryStop::ResourceLimit)?;
            enqueue(
                cursor,
                held.proposal.id,
                trajectory_scale(scale, true),
                Some(fraction),
                false,
            )?;
            batch.candidates.push(held);
        }
    }
    Ok(())
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

fn refinement_admission(control: &GeometryControl, reserve_evidence: bool) -> Option<GeometryStop> {
    let counters = control.counters();
    let limits = control.limits();
    control.checkpoint().or_else(|| {
        (reserve_evidence
            && (counters.query_calls >= limits.query_calls - limits.query_calls.div_ceil(4)
                || counters.triangle_tests
                    >= limits.triangle_tests - limits.triangle_tests.div_ceil(4)))
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
    clippy::too_many_lines,
    reason = "transactional solve with explicit resumable iteration state"
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
    pending: &mut Option<ScaleState>,
    reserve_evidence: bool,
) -> Result<(), GeometryStop> {
    let phase = match scale.slot {
        0 => crate::search_probe::Phase::Coarse,
        1 => crate::search_probe::Phase::Middle,
        _ => crate::search_probe::Phase::Dense,
    };
    let _phase = crate::search_probe::Span::new(phase, control);
    if pending.is_none() {
        let count = moving.samples[scale.slot]
            .samples
            .len()
            .saturating_add(fixed.samples[scale.slot].samples.len());
        let memory =
            control.reserve(count.saturating_mul(size_of::<Option<SurfaceQueryHint>>()))?;
        let mut hints = [Vec::new(), Vec::new()];
        for (surface, hints) in [moving, fixed].into_iter().zip(&mut hints) {
            hints
                .try_reserve_exact(surface.samples[scale.slot].samples.len())
                .map_err(|_| GeometryStop::ResourceLimit)?;
            hints.resize(surface.samples[scale.slot].samples.len(), None);
        }
        *pending = Some(ScaleState {
            pose: held.proposal.pose,
            fraction: initial_fraction,
            stable: 0,
            iteration: 0,
            hints,
            _memory: memory,
        });
    }
    let Some(state) = pending.as_mut() else {
        return Err(GeometryStop::Numerical);
    };
    let reach = scale.reach * (settings.influence_radius_mm / 2.).clamp(0.25, 2.);
    held.termination = RefinementTermination::IterationLimit;
    while state.iteration < scale.iterations {
        if let Some(stop) = refinement_admission(control, reserve_evidence) {
            return Err(stop);
        }
        let (pairs, fallback) = correspondences(
            moving,
            fixed,
            state.pose,
            scale.slot,
            reach,
            settings,
            &mut state.fraction,
            state.iteration.is_multiple_of(3)
                && (state.iteration != 0 || initial_fraction.is_none()),
            &mut state.hints,
            control,
        )?;
        if pairs.len() < 6 {
            held.termination = RefinementTermination::NoCorrespondences;
            break;
        }
        let Some(model) = accumulate_robust(&pairs, state.pose, control)? else {
            held.termination = RefinementTermination::Singular;
            break;
        };
        *iterations += 1;
        if state.pose == held.proposal.pose {
            held.information.clone_from(&model.information);
            held.unsigned_fallback |= fallback;
        }
        let (next, termination, objectives) =
            line_search(&pairs, state.pose, &model, scale.slot == 2, control)?;
        if termination != RefinementTermination::NotStarted {
            held.termination = termination;
            break;
        }
        let Some([before, after]) = objectives else {
            held.termination = RefinementTermination::NumericalTrialRejected;
            break;
        };
        control.charge_point_pairs(moving.samples[0].samples.len().min(256) as u64)?;
        let displacement = pose_distance(state.pose, next, &moving.samples[0].samples)[0];
        let score = rescore(moving, fixed, next, settings, control)?;
        *scored += 1;
        let next_proposal = Proposal {
            pose: next,
            score,
            ..held.proposal.clone()
        };
        if proposal_order(&next_proposal, &held.proposal).is_lt() {
            held.proposal = next_proposal;
            held.information = None;
            held.unsigned_fallback |= fallback;
        }
        state.pose = next;
        state.iteration += 1;
        let relative = (before - after).abs() / before.max(1e-20);
        state.stable = if relative < 1e-4 && displacement < 0.01 {
            state.stable + 1
        } else {
            0
        };
        if state.stable >= 3 {
            held.termination = RefinementTermination::Stationary;
            break;
        }
    }
    *pending = None;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "directed population and optional per-sample facet hints"
)]
fn gather(
    source: &PreparedSurface,
    target: &PreparedSurface,
    pose: Rigid,
    slot: usize,
    reach: f64,
    reverse: bool,
    policy: NormalPolicy,
    hints: &mut [Option<SurfaceQueryHint>],
    control: &GeometryControl,
) -> Result<Vec<Gathered>, GeometryStop> {
    let samples = &source.samples[slot].samples;
    let mut result = Vec::new();
    result
        .try_reserve_exact(samples.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut scratch = SurfaceQueryScratch::new(control)?;
    for (ordinal, sample) in samples.iter().enumerate() {
        control.charge_operations(1)?;
        scratch.set_facet_hint(hints.get(ordinal).copied().flatten());
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
        if let Some(hint) = hints.get_mut(ordinal) {
            *hint = scratch.facet_hint();
        }
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
    hints: &mut [Vec<Option<SurfaceQueryHint>>; 2],
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
        &mut hints[0],
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
        &mut hints[1],
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
    crate::search_probe::population(slot, !refresh, smaller.len() + other.len(), pairs.len());
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
            &mut [],
            &control,
        )
        .unwrap();
        assert_eq!(gathered.len(), surface.samples[2].samples.len());
        assert!(gathered.iter().all(|pair| pair.pair.is_none()));
        assert_eq!(control.counters().query_calls, before);
    }

    #[test]
    fn local_admission_resume_preserves_completed_iterations_and_scores() {
        let control = GeometryControl::unlimited();
        let surface = surface(&control);
        let proposals: Vec<_> = (0..4)
            .map(|i| {
                proposal(
                    &surface,
                    Rigid::new(
                        DQuat::from_rotation_x(0.01 * f64::from(i)),
                        DVec3::Z * 0.05 * f64::from(i),
                    ),
                    i,
                    &control,
                )
            })
            .collect();
        let settings = SearchSettings::default();
        let before = control.counters().operations;
        let uninterrupted =
            run_multiscale(&surface, &surface, proposals.clone(), &settings, &control);
        let resumed_control = GeometryControl::unlimited();
        let allowance = (control.counters().operations - before) / 2;
        let limited = resumed_control.with_operation_allowance(allowance);
        let mut resumed = run_multiscale(&surface, &surface, proposals, &settings, &limited);
        assert_eq!(resumed.stop, Some(GeometryStop::WorkLimit));
        assert!(resumed.iterations > 0);
        let completed = resumed.iterations;
        resume_multiscale(
            &surface,
            &surface,
            &mut resumed,
            &settings,
            &resumed_control,
            false,
        );
        assert!(resumed.stop.is_none());
        assert!(resumed.cursor.is_none());
        assert_eq!(resumed.basins, uninterrupted.basins);
        assert!(resumed.iterations >= completed);
        assert!(resumed.iterations <= uninterrupted.iterations + 1);
        let signature = |batch: &RefinementBatch| {
            batch
                .candidates
                .iter()
                .map(|p| {
                    (
                        p.proposal.id,
                        p.proposal.pose,
                        p.proposal.score.score,
                        p.termination,
                        p.attempted_scales,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&resumed), signature(&uninterrupted));
        assert_eq!(resumed_control.counters().memory_bytes, 0);
    }

    #[test]
    fn overlap_trajectories_share_the_coarse_basin_iteration_cap() {
        let control = GeometryControl::unlimited();
        let mesh = crate::proposal_test_support::arch::dental_arch(
            &crate::proposal_test_support::arch::ArchSpec::default(),
        );
        let mut surface = crate::prepare_alignment_surface(
            MeshInput {
                soup: mesh.soup(),
                world_from_local: DAffine3::IDENTITY,
                revision: 1,
            },
            SurfaceSide::Moving,
            RegionPolicy::AllEligible,
            &control,
        )
        .unwrap()
        .surface
        .unwrap();
        let pose = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 5f64.to_radians()),
            DVec3::new(3., -2., 1.),
        );
        let mut seed = proposal(&surface, pose, 0, &control);
        // Deliberately keep a second supported common-region hypothesis.
        seed.score.supported_prefix = Some((0.2, 1.));
        surface.samples[1].samples.clear();
        surface.samples[2].samples.clear();
        let result = run_multiscale(
            &surface,
            &surface,
            vec![seed],
            &SearchSettings::default(),
            &control,
        );
        assert!(result.stop.is_none());
        assert_eq!(result.basins[0], 1);
        assert!(
            result.iterations <= 12,
            "one coarse basin spent {} iterations across its overlap hypotheses",
            result.iterations
        );
        assert!(result
            .candidates
            .iter()
            .all(|p| p.proposal.pose.is_finite()));
    }

    #[test]
    fn temporal_population_bounds_preserve_pairs_and_reduce_traversal() {
        let unlimited = GeometryControl::unlimited();
        let surface = surface(&unlimited);
        let cold = GeometryControl::unlimited();
        let warm = GeometryControl::unlimited();
        let mut hints = vec![None; surface.samples[2].samples.len()];
        // Initial nearest facets precede a new pose; no old point/distance is
        // allowed to stand in for its recomputed exact surface answer.
        gather(
            &surface,
            &surface,
            Rigid::IDENTITY,
            2,
            4.,
            false,
            NormalPolicy::Unsigned,
            &mut hints,
            &unlimited,
        )
        .unwrap();
        let pose = Rigid::new(DQuat::from_rotation_x(0.001), DVec3::Z * 0.02);
        let expected = gather(
            &surface,
            &surface,
            pose,
            2,
            4.,
            false,
            NormalPolicy::Unsigned,
            &mut [],
            &cold,
        )
        .unwrap();
        let actual = gather(
            &surface,
            &surface,
            pose,
            2,
            4.,
            false,
            NormalPolicy::Unsigned,
            &mut hints,
            &warm,
        )
        .unwrap();
        for (a, b) in actual.iter().zip(&expected) {
            assert_eq!(a.distance.distance, b.distance.distance);
            assert_eq!(a.distance.weight, b.distance.weight);
            assert_eq!(a.distance.compatible, b.distance.compatible);
            assert_eq!(a.distance.cell, b.distance.cell);
            assert_eq!(
                a.pair.map(|p| (p.moving, p.fixed, p.normal, p.weight)),
                b.pair.map(|p| (p.moving, p.fixed, p.normal, p.weight))
            );
        }
        assert_eq!(cold.counters().query_calls, warm.counters().query_calls);
        assert!(
            warm.counters().operations < cold.counters().operations,
            "temporal={} preceding-sample={}",
            warm.counters().operations,
            cold.counters().operations
        );
        assert_eq!(warm.counters().memory_bytes, 0);
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
