//! Normal-histogram consensus proposals, independently derived from Rusu,
//! Blodow and Beetz, FPFH (2009), <https://doi.org/10.1109/ROBOT.2009.5152473>.
//! Two physical radii and both relative normal signs share a fixed round-robin
//! trial schedule. Small clouds remain eligible; consensus supplies proposals,
//! never confidence. No reference implementation is transcribed.

use crate::{
    area_samples, CancelFlag, PreparedSurface, Rigid, SeedOrigin, SurfaceIndex, SurfaceSample,
};
use glam::DVec3;
use kdtree::{distance::squared_euclidean, KdTree};
use occluview_geometry::surface::{GeometryControl, GeometryStop};
use std::collections::{BTreeMap, BTreeSet};

const SIZE: usize = 33;
#[derive(Clone, Copy, Debug)]
pub(super) struct FeatureSeed {
    pub(super) rigid: Rigid,
}
#[derive(Clone, Copy)]
struct Feature {
    point: DVec3,
    normal: DVec3,
}
#[derive(Clone, Copy)]
struct Match {
    source: usize,
    target: usize,
    ratio: f64,
}
struct Schedule {
    matches: Vec<Match>,
    state: u64,
    origin: SeedOrigin,
}
#[derive(Clone, Copy)]
struct Consensus {
    pose: Rigid,
    cells: usize,
    residual: f64,
    schedule: usize,
}

/// Legacy pair-fit consumer uses the same bounded feature proposal producer.
pub(super) fn find_feature_seed(
    moving: Option<&SurfaceIndex>,
    fixed: &SurfaceIndex,
    cancel: &CancelFlag,
) -> Option<FeatureSeed> {
    let moving = moving?;
    let unrestricted = GeometryControl::new(
        cancel.clone(),
        std::time::Duration::MAX,
        occluview_geometry::surface::GeometryLimits::default(),
    );
    let control = fixed.query_control().unwrap_or(&unrestricted);
    if cancel.is_cancelled() {
        return None;
    }
    let source = area_samples(
        moving,
        4_096,
        0x4f56_5f41_4c52_3103,
        moving.orientation_coherent(),
        control,
    )
    .ok()?;
    let target = area_samples(
        fixed,
        4_096,
        0x4f56_5f41_4c52_3103,
        fixed.orientation_coherent(),
        control,
    )
    .ok()?;
    hypotheses(&source.samples, &target.samples, control)
        .ok()?
        .first()
        .map(|&(rigid, _)| FeatureSeed { rigid })
}

pub(crate) fn feature_hypotheses(
    moving: &PreparedSurface,
    fixed: &PreparedSurface,
    control: &GeometryControl,
) -> Result<Vec<(Rigid, SeedOrigin)>, GeometryStop> {
    hypotheses(
        &moving.samples[1].samples,
        &fixed.samples[1].samples,
        control,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "four fixed consensus schedules share bounded clouds and descriptors"
)]
fn hypotheses(
    source: &[SurfaceSample],
    target: &[SurfaceSample],
    control: &GeometryControl,
) -> Result<Vec<(Rigid, SeedOrigin)>, GeometryStop> {
    let _phase = crate::search_probe::Span::new(crate::search_probe::Phase::Descriptors, control);
    let _memory = control.reserve(24 * 1024 * 1024)?;
    let source = cloud(source, control)?;
    let target = cloud(target, control)?;
    if source.len() < 32 || target.len() < 32 {
        return Ok(Vec::new());
    }
    let mut schedules = Vec::new();
    schedules
        .try_reserve_exact(4)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let source_descriptors = descriptor_variants(&source, &[false], control)?;
    let target_descriptors = descriptor_variants(&target, &[false, true], control)?;
    for (radius, a) in source_descriptors.iter().enumerate() {
        for (sign, reversed) in [false, true].into_iter().enumerate() {
            let matches = match_features(a, &target_descriptors[radius * 2 + sign], control)?;
            let ordinal = schedules.len() as u64;
            schedules.push(Schedule {
                matches,
                state: crate::sample::mix_seed(0x4f56_5f41_4c52_3100 ^ (3 + ordinal)),
                origin: if reversed {
                    SeedOrigin::FeatureOpposed
                } else {
                    SeedOrigin::FeatureSame
                },
            });
        }
    }
    let mut retained: Vec<Consensus> = Vec::with_capacity(8);
    for trial in 0..4_096 {
        control.charge_operations(1)?;
        let which = trial % 4;
        let schedule = &mut schedules[which];
        if schedule.matches.len() < 3 {
            continue;
        }
        let mut slots = [0; 3];
        for slot in &mut slots {
            schedule.state = schedule.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            *slot = usize::try_from(
                crate::sample::mix_seed(schedule.state) % schedule.matches.len() as u64,
            )
            .map_err(|_| GeometryStop::ResourceLimit)?;
        }
        if slots[0] == slots[1] || slots[0] == slots[2] || slots[1] == slots[2] {
            continue;
        }
        let triplet = slots.map(|i| schedule.matches[i]);
        let from = triplet.map(|m| source[m.source].point);
        let to = triplet.map(|m| target[m.target].point);
        let mut consistent = true;
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            control.charge_point_pairs(1)?;
            let length = from[i].distance(from[j]);
            consistent &= (length - to[i].distance(to[j])).abs() <= (0.10 * length).max(0.15);
        }
        if !consistent {
            continue;
        }
        let Some(pose) = crate::proposal_geometry::fit_geometry_pairs(&from, &to) else {
            continue;
        };
        let (cells, residual) = consensus(pose, &schedule.matches, &source, &target, 512, control)?;
        let candidate = Consensus {
            pose,
            cells,
            residual,
            schedule: which,
        };
        let duplicate = retained.iter().position(|p| {
            p.pose.translation.distance(pose.translation) < 0.2
                && p.pose.rotation.dot(pose.rotation).abs() > (0.5f64.to_radians()).cos()
        });
        if let Some(i) = duplicate {
            if consensus_order(&candidate, &retained[i]).is_lt() {
                retained[i] = candidate;
            }
        } else {
            retained.push(candidate);
        }
        retained.sort_by(consensus_order);
        if retained.len() > 8 {
            let mut counts = [0; 2];
            for held in &retained {
                counts[held.schedule % 2] += 1;
            }
            let drop = retained
                .iter()
                .rposition(|p| counts[p.schedule % 2] > 4)
                .unwrap_or(retained.len() - 1);
            retained.remove(drop);
        }
    }
    let mut result = Vec::with_capacity(8);
    for mut held in retained {
        let schedule = &schedules[held.schedule];
        // Fit only a geometrically coherent consensus, not all descriptor matches.
        let mut from = Vec::new();
        let mut to = Vec::new();
        from.try_reserve_exact(schedule.matches.len())
            .map_err(|_| GeometryStop::ResourceLimit)?;
        to.try_reserve_exact(schedule.matches.len())
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for m in &schedule.matches {
            control.charge_point_pairs(1)?;
            if held
                .pose
                .apply(source[m.source].point)
                .distance(target[m.target].point)
                <= 0.8
            {
                from.push(source[m.source].point);
                to.push(target[m.target].point);
            }
        }
        if let Some(polished) = crate::proposal_geometry::fit_geometry_pairs(&from, &to) {
            held.pose = polished;
        }
        let (cells, _) = consensus(
            held.pose,
            &schedule.matches,
            &source,
            &target,
            4_096,
            control,
        )?;
        if cells >= 3 {
            result.push((held.pose, schedule.origin));
        }
    }
    Ok(result)
}
fn consensus_order(a: &Consensus, b: &Consensus) -> std::cmp::Ordering {
    b.cells
        .cmp(&a.cells)
        .then_with(|| a.residual.total_cmp(&b.residual))
        .then_with(|| a.schedule.cmp(&b.schedule))
}

#[allow(clippy::cast_possible_truncation)]
fn cloud(
    samples: &[SurfaceSample],
    control: &GeometryControl,
) -> Result<Vec<Feature>, GeometryStop> {
    let mut voxels = BTreeMap::new();
    for sample in samples.iter().take(4_096) {
        control.charge_operations(1)?;
        let Some(normal) = sample.normal else {
            continue;
        };
        let p = (sample.point / 0.6).floor();
        if !p.is_finite() || p.abs().max_element() >= 9_223_372_036_854_775_808. {
            continue;
        }
        let key = p.to_array().map(|v| v as i64);
        voxels.entry(key).or_insert(Feature {
            point: sample.point,
            normal,
        });
    }
    let mut cloud = Vec::new();
    cloud
        .try_reserve_exact(voxels.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    cloud.extend(voxels.into_values());
    Ok(cloud)
}

fn descriptor_variants(
    cloud: &[Feature],
    signs: &[bool],
    control: &GeometryControl,
) -> Result<Vec<Vec<[f64; SIZE]>>, GeometryStop> {
    let neighbors = geometric_neighbors(cloud, control)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(signs.len().saturating_mul(2))
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for radius in [2., 4.] {
        for &reversed in signs {
            result.push(descriptors_from_neighbors(
                cloud, &neighbors, radius, reversed, control,
            )?);
        }
    }
    Ok(result)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
fn sign_descriptors(
    cloud: &[Feature],
    radius: f64,
    reversed: bool,
    control: &GeometryControl,
) -> Result<Vec<[f64; SIZE]>, GeometryStop> {
    let mut tree = KdTree::new(3);
    for (i, p) in cloud.iter().enumerate() {
        control.charge_operations(1)?;
        tree.add(p.point.to_array(), i)
            .map_err(|_| GeometryStop::Numerical)?;
    }
    let mut neighbors = Vec::new();
    neighbors
        .try_reserve_exact(cloud.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut spfh = Vec::new();
    spfh.try_reserve_exact(cloud.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, p) in cloud.iter().enumerate() {
        let metric = |a: &[f64], b: &[f64]| {
            if control.charge_point_pairs(1).is_err() {
                f64::MAX
            } else {
                squared_euclidean(a, b)
            }
        };
        let nearest = tree
            .nearest(&p.point.to_array(), 65, &metric)
            .map_err(|_| GeometryStop::Numerical)?;
        if let Some(stop) = control.checkpoint() {
            return Err(stop);
        }
        let mut local = Vec::new();
        local
            .try_reserve_exact(64)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for (distance, &other) in nearest {
            if other != i && distance > 1e-12 && distance <= radius * radius {
                local.push((other, distance.sqrt()));
            }
        }
        local.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut histogram = [0.; SIZE];
        for &(other, distance) in &local {
            control.charge_operations(1)?;
            let mut delta = (cloud[other].point - p.point) / distance;
            let sign = if reversed { -1. } else { 1. };
            let mut first = p.normal * sign;
            let mut second = cloud[other].normal * sign;
            let a = first.dot(delta);
            let b = second.dot(delta);
            let phi = if a.abs() < b.abs() {
                std::mem::swap(&mut first, &mut second);
                delta = -delta;
                -b
            } else {
                a
            };
            let tangent = delta.cross(first).normalize_or_zero();
            if tangent.length_squared() < 0.5 {
                continue;
            }
            let theta = first.cross(tangent).dot(second).atan2(first.dot(second));
            histogram[bin(theta / std::f64::consts::PI)] += 1.;
            histogram[11 + bin(tangent.dot(second))] += 1.;
            histogram[22 + bin(phi)] += 1.;
        }
        normalize(&mut histogram);
        spfh.push(histogram);
        neighbors.push(local);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(spfh.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, local) in neighbors.iter().enumerate() {
        let mut histogram = spfh[i];
        let mut weighted = [0.; SIZE];
        for &(other, distance) in local {
            control.charge_operations(1)?;
            for (slot, value) in weighted.iter_mut().enumerate() {
                *value += spfh[other][slot] / distance;
            }
        }
        normalize(&mut weighted);
        for (value, extra) in histogram.iter_mut().zip(weighted) {
            *value += extra;
        }
        normalize(&mut histogram);
        result.push(histogram);
    }
    Ok(result)
}
/// Radius and normal sign do not change the nearest-65 geometric population.
/// Keep its stable distance/index order once for all descriptor variants.
fn geometric_neighbors(
    cloud: &[Feature],
    control: &GeometryControl,
) -> Result<Vec<Vec<(usize, f64)>>, GeometryStop> {
    let mut tree = KdTree::new(3);
    for (i, p) in cloud.iter().enumerate() {
        control.charge_operations(1)?;
        tree.add(p.point.to_array(), i)
            .map_err(|_| GeometryStop::Numerical)?;
    }
    let mut neighbors = Vec::new();
    neighbors
        .try_reserve_exact(cloud.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, p) in cloud.iter().enumerate() {
        let metric = |a: &[f64], b: &[f64]| {
            if control.charge_point_pairs(1).is_err() {
                f64::MAX
            } else {
                squared_euclidean(a, b)
            }
        };
        let nearest = tree
            .nearest(&p.point.to_array(), 65, &metric)
            .map_err(|_| GeometryStop::Numerical)?;
        if let Some(stop) = control.checkpoint() {
            return Err(stop);
        }
        let mut local = Vec::new();
        local
            .try_reserve_exact(64)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for (distance, &other) in nearest {
            if other != i && distance > 1e-12 && distance <= 16. {
                local.push((other, distance));
            }
        }
        local.sort_by(|a, b| a.1.sqrt().total_cmp(&b.1.sqrt()).then(a.0.cmp(&b.0)));
        neighbors.push(local);
    }
    Ok(neighbors)
}

fn descriptors_from_neighbors(
    cloud: &[Feature],
    neighbors: &[Vec<(usize, f64)>],
    radius: f64,
    reversed: bool,
    control: &GeometryControl,
) -> Result<Vec<[f64; SIZE]>, GeometryStop> {
    let mut spfh = Vec::new();
    spfh.try_reserve_exact(cloud.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, p) in cloud.iter().enumerate() {
        let local = &neighbors[i];
        let mut histogram = [0.; SIZE];
        for &(other, distance) in local
            .iter()
            .filter(|(_, distance)| *distance <= radius * radius)
        {
            control.charge_operations(1)?;
            let distance = distance.sqrt();
            let mut delta = (cloud[other].point - p.point) / distance;
            let sign = if reversed { -1. } else { 1. };
            let mut first = p.normal * sign;
            let mut second = cloud[other].normal * sign;
            let a = first.dot(delta);
            let b = second.dot(delta);
            let phi = if a.abs() < b.abs() {
                std::mem::swap(&mut first, &mut second);
                delta = -delta;
                -b
            } else {
                a
            };
            let tangent = delta.cross(first).normalize_or_zero();
            if tangent.length_squared() < 0.5 {
                continue;
            }
            let theta = first.cross(tangent).dot(second).atan2(first.dot(second));
            histogram[bin(theta / std::f64::consts::PI)] += 1.;
            histogram[11 + bin(tangent.dot(second))] += 1.;
            histogram[22 + bin(phi)] += 1.;
        }
        normalize(&mut histogram);
        spfh.push(histogram);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(spfh.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, local) in neighbors.iter().enumerate() {
        let mut histogram = spfh[i];
        let mut weighted = [0.; SIZE];
        for &(other, distance) in local
            .iter()
            .filter(|(_, distance)| *distance <= radius * radius)
        {
            control.charge_operations(1)?;
            let distance = distance.sqrt();
            for (slot, value) in weighted.iter_mut().enumerate() {
                *value += spfh[other][slot] / distance;
            }
        }
        normalize(&mut weighted);
        for (value, extra) in histogram.iter_mut().zip(weighted) {
            *value += extra;
        }
        normalize(&mut histogram);
        result.push(histogram);
    }
    Ok(result)
}
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn bin(value: f64) -> usize {
    (((value.clamp(-1., 1.) + 1.) * 5.5).floor() as usize).min(10)
}
fn normalize(histogram: &mut [f64; SIZE]) {
    for block in histogram.chunks_mut(11) {
        let sum: f64 = block.iter().sum();
        if sum > 0. {
            for value in block {
                *value /= sum;
            }
        }
    }
}
fn match_features(
    source: &[[f64; SIZE]],
    target: &[[f64; SIZE]],
    control: &GeometryControl,
) -> Result<Vec<Match>, GeometryStop> {
    // Two empty histogram populations have distance zero everywhere. The
    // fixed tie order selects one target for every source (ratio zero), so
    // no nondegenerate rigid triplet can be produced by this schedule.
    let empty =
        |descriptors: &[[f64; SIZE]]| descriptors.iter().all(|d| d.iter().all(|v| *v == 0.));
    control.charge_operations(
        u64::try_from(source.len().saturating_add(target.len()))
            .map_err(|_| GeometryStop::ResourceLimit)?,
    )?;
    if empty(source) && empty(target) {
        return Ok(Vec::new());
    }
    let mut tree = KdTree::new(SIZE);
    for (i, descriptor) in target.iter().enumerate() {
        control.charge_operations(1)?;
        tree.add(*descriptor, i)
            .map_err(|_| GeometryStop::Numerical)?;
    }
    let mut matches = Vec::new();
    matches
        .try_reserve_exact(4_096)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for (i, descriptor) in source.iter().enumerate() {
        let metric = |a: &[f64], b: &[f64]| {
            if control.charge_point_pairs(1).is_err() {
                f64::MAX
            } else {
                squared_euclidean(a, b)
            }
        };
        let mut nearest = tree
            .nearest(descriptor, 3, &metric)
            .map_err(|_| GeometryStop::Numerical)?;
        if let Some(stop) = control.checkpoint() {
            return Err(stop);
        }
        nearest.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        let ratio = if nearest.len() > 1 {
            nearest[0].0 / nearest[1].0.max(f64::MIN_POSITIVE)
        } else {
            1.
        };
        for (_, &other) in nearest
            .into_iter()
            .take(if ratio < 0.9f64.powi(2) { 1 } else { 3 })
        {
            if matches.len() < 4_096 {
                matches.push(Match {
                    source: i,
                    target: other,
                    ratio,
                });
            }
        }
    }
    matches.sort_by(|a, b| {
        a.ratio
            .total_cmp(&b.ratio)
            .then(a.source.cmp(&b.source))
            .then(a.target.cmp(&b.target))
    });
    Ok(matches)
}
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
#[expect(
    clippy::too_many_arguments,
    reason = "paired clouds, match population, pose and work control are explicit"
)]
fn consensus(
    pose: Rigid,
    matches: &[Match],
    source: &[Feature],
    target: &[Feature],
    count: usize,
    control: &GeometryControl,
) -> Result<(usize, f64), GeometryStop> {
    let mut cells = BTreeSet::new();
    let mut residual = 0.;
    let mut inliers = 0;
    let count = count.min(matches.len());
    for i in 0..count {
        control.charge_point_pairs(1)?;
        let m = matches[i * matches.len() / count];
        let distance = pose
            .apply(source[m.source].point)
            .distance(target[m.target].point);
        if distance <= 0.8 {
            cells.insert(source[m.source].point.floor().to_array().map(|v| v as i64));
            residual += distance;
            inliers += 1;
        }
    }
    Ok((cells.len(), residual / f64::from(inliers.max(1))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    #[test]
    fn empty_feature_histograms_cannot_supply_a_rigid_triplet() {
        let control = GeometryControl::unlimited();
        let empty = vec![[0.; SIZE]; 128];
        let matches = match_features(&empty, &empty, &control).unwrap();
        assert!(matches.is_empty());
        assert_eq!(control.counters().point_pair_tests, 0);
    }

    #[test]
    fn descriptor_variants_share_one_geometric_neighborhood_search() {
        let cloud = (0..256u32)
            .map(|i| Feature {
                point: DVec3::new(
                    f64::from(i % 16) * 0.47,
                    f64::from(i / 16) * 0.51,
                    f64::from(i % 7) * 0.03,
                ),
                normal: DVec3::new(0.02 * f64::from(i % 16), 0.03 * f64::from(i / 16), 1.)
                    .normalize(),
            })
            .collect::<Vec<_>>();
        let single = GeometryControl::unlimited();
        let expected = [2., 4.]
            .into_iter()
            .flat_map(|radius| {
                [false, true].map(|sign| sign_descriptors(&cloud, radius, sign, &single).unwrap())
            })
            .collect::<Vec<_>>();
        let shared = GeometryControl::unlimited();
        let actual = descriptor_variants(&cloud, &[false, true], &shared).unwrap();
        assert_eq!(actual, expected);
        assert!(
            shared.counters().point_pair_tests * 4 <= single.counters().point_pair_tests,
            "repeated spatial searches: {} vs {}",
            shared.counters().point_pair_tests,
            single.counters().point_pair_tests
        );
    }

    #[test]
    fn histograms_preserve_rigid_geometry_and_both_normal_signs() {
        let source = (0..64u32)
            .map(|i| {
                let x = f64::from(i % 8) * 0.71;
                let y = f64::from(i / 8) * 0.83;
                let point = DVec3::new(x, y, 0.031 * x * x + 0.067 * y * y + 0.017 * x * y);
                let normal =
                    DVec3::new(-0.062 * x - 0.017 * y, -0.134 * y - 0.017 * x, 1.).normalize();
                Feature { point, normal }
            })
            .collect::<Vec<_>>();
        let rotation = DQuat::from_axis_angle(DVec3::new(2., 3., 5.).normalize(), 1.137);
        let target = source
            .iter()
            .map(|s| Feature {
                point: rotation * s.point + DVec3::new(7., 11., 13.),
                normal: -(rotation * s.normal),
            })
            .collect::<Vec<_>>();
        for radius in [2., 4.] {
            let a =
                sign_descriptors(&source, radius, false, &GeometryControl::unlimited()).unwrap();
            let b = sign_descriptors(&target, radius, true, &GeometryControl::unlimited()).unwrap();
            assert!(a
                .iter()
                .flatten()
                .zip(b.iter().flatten())
                .all(|(x, y)| (x - y).abs() < 1e-10));
        }
        assert!(hypotheses(&[], &[], &GeometryControl::unlimited())
            .unwrap()
            .is_empty());
    }
}
