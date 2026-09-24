//! Global surface proposal for scans with changed anatomy.
//!
//! The 33-bin local normal histogram follows the FPFH construction used by
//! `Open3D` (MIT, Copyright 2018-2024 www.open3d.org;
//! <https://github.com/isl-org/Open3D/blob/v0.19.0/cpp/open3d/pipelines/registration/Feature.cpp>).
//! The independent matches
//! are only proposals: a rigid pose needs spatially extended geometric
//! consensus before the existing surface ICP can refine it.

use std::collections::BTreeMap;

use glam::DVec3;
use kdtree::{distance::squared_euclidean, KdTree};

use crate::pairs::{fit_pairs, horn_fit, FitBounds};
use crate::surface::{feature_voxel_key, FeaturePoint, FEATURE_VOXEL_MM};
use crate::{CancelFlag, Rigid, SurfaceIndex};

const FEATURE_RADIUS_MM: f64 = FEATURE_VOXEL_MM * 5.0;
const FEATURE_RADIUS_SQ: f64 = FEATURE_RADIUS_MM * FEATURE_RADIUS_MM;
const MAX_NEIGHBORS: usize = 99; // Open3D's 100 includes the query itself.
                                 // A handful of synthetic facets has no distinctive local histogram. Keep the
                                 // established geometry path for such surfaces, including cropped tooth tests.
const MIN_CLOUD_POINTS: usize = 2_000;
const BINS: usize = 11;
const DESCRIPTOR_SIZE: usize = BINS * 3;
const MATCH_RATIO_SQUARED: f64 = 0.8 * 0.8;
const CONSENSUS_MM: f64 = 0.5;
/// The band that separates a seating from an illusion, in millimetres.
///
/// Two agreements can both reach hundreds of matches within the coarse
/// consensus distance and still be different answers: a prepared model's
/// operated region slides onto the original within half a millimetre almost
/// everywhere, while only the unchanged region seats exactly. Counting the
/// matches inside this tighter band is what tells those two apart, and it is
/// the same band the ICP refinement treats as seated.
const TIGHT_CONSENSUS_MM: f64 = 0.05;
/// Independent matches one agreement must carry before it can seed a fit.
///
/// This is a floor on EVIDENCE, not on the fraction of the cloud: a prepared
/// model keeps only its unchanged region rigid, and that region can be a small
/// minority of the surface. On a real prepared arch pair the true seating
/// carried 12 spatially extended agreements, so a floor of 24 refused the one
/// correct hypothesis and left the search to a coarse orientation sweep that
/// landed 2.5 mm away. The floor is only safe because the seed is not trusted
/// on its own: `refine` refuses any pose more than a millimetre from it, so a
/// coincidental twelve-point agreement produces a refusal, not a wrong pose.
const MIN_SUPPORT: usize = 12;
const MIN_SPAN_MM: f64 = 4.0;
const TRIAL_BUDGET: usize = 15_000;

#[derive(Clone, Copy, Debug)]
pub(super) struct FeatureSeed {
    pub(super) rigid: Rigid,
}

#[derive(Clone, Copy)]
struct Match {
    moving: usize,
    fixed: usize,
    ratio: f64,
}

#[derive(Clone, Copy)]
struct Consensus {
    rigid: Rigid,
    inliers: usize,
    /// Matches inside [`TIGHT_CONSENSUS_MM`]: the matches that are actually
    /// seated rather than merely near.
    tight: usize,
    residual: f64,
    span: f64,
}

pub(super) fn find_feature_seed(
    moving: Option<&SurfaceIndex>,
    fixed: &SurfaceIndex,
    cancel: &CancelFlag,
) -> Option<FeatureSeed> {
    let moving = moving?;
    if cancel.is_cancelled() {
        return None;
    }
    let moving_cloud = moving.feature_cloud();
    let fixed_cloud = fixed.feature_cloud();
    if moving_cloud.len() < MIN_CLOUD_POINTS {
        return None;
    }
    if fixed_cloud.len() < MIN_CLOUD_POINTS {
        return None;
    }
    let moving_descriptors = descriptors(&moving_cloud, cancel)?;
    let fixed_descriptors = descriptors(&fixed_cloud, cancel)?;
    let matches = match_features(&moving_descriptors, &fixed_descriptors, cancel)?;
    if matches.len() < MIN_SUPPORT {
        return None;
    }
    let bounds = fit_bounds(moving, fixed);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let modulus = u64::try_from(matches.len()).ok()?;
    let mut best: Option<Consensus> = None;
    let mut rival: Option<Consensus> = None;
    for trial in 0..TRIAL_BUDGET {
        if trial % 64 == 0 && cancel.is_cancelled() {
            return None;
        }
        let mut slots = [0usize; 3];
        for slot in &mut slots {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *slot = usize::try_from(state % modulus).ok()?;
        }
        if slots[0] == slots[1] || slots[0] == slots[2] || slots[1] == slots[2] {
            continue;
        }
        let triplet = slots.map(|slot| matches[slot]);
        let source = triplet.map(|pair| moving_cloud[pair.moving].position);
        let target = triplet.map(|pair| fixed_cloud[pair.fixed].position);
        if !triplet_is_consistent(&source, &target) {
            continue;
        }
        let Ok(fit) = fit_pairs(&source, &target, None, &bounds) else {
            continue;
        };
        let candidate = consensus(fit.rigid, &matches, &moving_cloud, &fixed_cloud);
        if candidate.inliers < MIN_SUPPORT || candidate.span < MIN_SPAN_MM {
            continue;
        }
        if best.is_none_or(|current| better(candidate, current)) {
            if let Some(current) = best {
                if distinct(candidate.rigid, current.rigid, bounds.moving_center) {
                    rival = Some(current);
                }
            }
            best = Some(candidate);
        } else if best
            .is_some_and(|current| distinct(candidate.rigid, current.rigid, bounds.moving_center))
            && rival.is_none_or(|current| better(candidate, current))
        {
            rival = Some(candidate);
        }
    }
    let mut best = best?;
    // Not a fixed fraction of the matched cloud: a changed arch supplies most
    // of its features from the region that no longer matches, so requiring a
    // share of them would refuse exactly the case this seed exists for. What
    // matters is that no rival explains as much, and that the support is
    // spatially extended rather than a coincidental cluster.
    // A rival is only a rival if it seats as well. Two hypotheses that both
    // explain the coarse distance are not equally supported when one of them
    // seats hundreds of matches exactly and the other seats a handful.
    if rival.is_some_and(|other| {
        other.tight * 10 >= best.tight * 9 && other.inliers * 10 >= best.inliers * 9
    }) {
        return None;
    }
    for _ in 0..4 {
        let mut source = Vec::new();
        let mut target = Vec::new();
        for pair in &matches {
            let moving_point = moving_cloud[pair.moving].position;
            let fixed_point = fixed_cloud[pair.fixed].position;
            if best.rigid.apply(moving_point).distance(fixed_point) < 0.6 {
                source.push(moving_point);
                target.push(fixed_point);
            }
        }
        if source.len() < MIN_SUPPORT {
            return None;
        }
        let slots: Vec<usize> = (0..source.len()).collect();
        let fit = horn_fit(&source, &target, &slots).ok()?;
        best = consensus(fit, &matches, &moving_cloud, &fixed_cloud);
    }
    if best.inliers < MIN_SUPPORT || best.span < MIN_SPAN_MM {
        return None;
    }
    Some(FeatureSeed { rigid: best.rigid })
}

fn fit_bounds(moving: &SurfaceIndex, fixed: &SurfaceIndex) -> FitBounds {
    let (moving_min, moving_max) = moving.bounds();
    let (fixed_min, fixed_max) = fixed.bounds();
    FitBounds {
        moving_center: (moving_min + moving_max) * 0.5,
        moving_extent: moving_min.distance(moving_max),
        fixed_center: (fixed_min + fixed_max) * 0.5,
        fixed_extent: fixed_min.distance(fixed_max),
    }
}

fn triplet_is_consistent(source: &[DVec3; 3], target: &[DVec3; 3]) -> bool {
    let mut span = 0.0_f64;
    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        let a = source[left].distance(source[right]);
        let b = target[left].distance(target[right]);
        if (a - b).abs() > 0.4 {
            return false;
        }
        span = span.max(a);
    }
    span >= MIN_SPAN_MM
}

#[allow(clippy::cast_precision_loss)]
fn consensus(
    rigid: Rigid,
    matches: &[Match],
    moving: &[FeaturePoint],
    fixed: &[FeaturePoint],
) -> Consensus {
    let mut inliers = 0;
    let mut tight = 0;
    let mut residual = 0.0;
    let mut minimum = DVec3::splat(f64::INFINITY);
    let mut maximum = DVec3::splat(f64::NEG_INFINITY);
    for pair in matches {
        let point = moving[pair.moving].position;
        let distance = rigid.apply(point).distance(fixed[pair.fixed].position);
        if distance < TIGHT_CONSENSUS_MM {
            tight += 1;
        }
        if distance < CONSENSUS_MM {
            inliers += 1;
            residual += distance;
            minimum = minimum.min(point);
            maximum = maximum.max(point);
        }
    }
    Consensus {
        rigid,
        inliers,
        tight,
        residual: residual / inliers.max(1) as f64,
        span: if inliers == 0 {
            0.0
        } else {
            minimum.distance(maximum)
        },
    }
}

fn better(left: Consensus, right: Consensus) -> bool {
    // Exact seating first. A prepared model's operated region can reach as many
    // coarse matches as the unchanged one, so ranking on the coarse count picks
    // between them by luck; the count inside the tight band is what says which
    // agreement actually seats a surface.
    if left.tight != right.tight {
        return left.tight > right.tight;
    }
    left.inliers > right.inliers
        || (left.inliers == right.inliers && left.residual < right.residual)
}

fn distinct(left: Rigid, right: Rigid, center: DVec3) -> bool {
    left.apply(center).distance(right.apply(center)) > 1.0
        || (left.rotation * right.rotation.inverse())
            .to_scaled_axis()
            .length()
            > 0.1
}

fn match_features(
    moving: &[[f64; DESCRIPTOR_SIZE]],
    fixed: &[[f64; DESCRIPTOR_SIZE]],
    cancel: &CancelFlag,
) -> Option<Vec<Match>> {
    let mut tree = KdTree::new(DESCRIPTOR_SIZE);
    for (index, descriptor) in fixed.iter().enumerate() {
        tree.add(*descriptor, index).ok()?;
    }
    let mut matches = Vec::new();
    for (index, descriptor) in moving.iter().enumerate() {
        if index % 64 == 0 && cancel.is_cancelled() {
            return None;
        }
        let nearest = tree.nearest(descriptor, 2, &squared_euclidean).ok()?;
        if nearest.len() < 2 || nearest[1].0 <= f64::EPSILON {
            continue;
        }
        let ratio = nearest[0].0 / nearest[1].0;
        if ratio < MATCH_RATIO_SQUARED {
            matches.push(Match {
                moving: index,
                fixed: *nearest[0].1,
                ratio,
            });
        }
    }
    matches.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
    Some(matches)
}

#[allow(clippy::cast_precision_loss)]
fn descriptors(cloud: &[FeaturePoint], cancel: &CancelFlag) -> Option<Vec<[f64; DESCRIPTOR_SIZE]>> {
    let neighbors = neighbor_lists(cloud, cancel)?;
    let mut spfh = vec![[0.0; DESCRIPTOR_SIZE]; cloud.len()];
    for (index, local) in neighbors.iter().enumerate() {
        if index % 64 == 0 && cancel.is_cancelled() {
            return None;
        }
        if local.is_empty() {
            continue;
        }
        let increment = 100.0 / local.len() as f64;
        for &(other, distance_sq) in local {
            let source = cloud[index];
            let target = cloud[other];
            let mut delta = target.position - source.position;
            let distance = distance_sq.sqrt();
            let mut source_normal = source.normal;
            let mut target_normal = target.normal;
            let angle1 = source_normal.dot(delta) / distance;
            let angle2 = target_normal.dot(delta) / distance;
            let phi = if angle1.abs() < angle2.abs() {
                std::mem::swap(&mut source_normal, &mut target_normal);
                delta = -delta;
                -angle2
            } else {
                angle1
            };
            let cross = delta.cross(source_normal);
            let (theta, alpha, phi) = if cross.length_squared() <= f64::EPSILON {
                (0.0, 0.0, 0.0)
            } else {
                let side = cross.normalize();
                let third = source_normal.cross(side);
                (
                    third
                        .dot(target_normal)
                        .atan2(source_normal.dot(target_normal)),
                    side.dot(target_normal),
                    phi,
                )
            };
            spfh[index][bin(theta, -std::f64::consts::PI, std::f64::consts::PI)] += increment;
            spfh[index][BINS + bin(alpha, -1.0, 1.0)] += increment;
            spfh[index][2 * BINS + bin(phi, -1.0, 1.0)] += increment;
        }
    }
    let mut result = spfh.clone();
    for (index, local) in neighbors.iter().enumerate() {
        if index % 64 == 0 && cancel.is_cancelled() {
            return None;
        }
        if local.is_empty() {
            continue;
        }
        let mut weighted = [0.0; DESCRIPTOR_SIZE];
        for &(other, distance_sq) in local {
            for (slot, value) in weighted.iter_mut().enumerate() {
                *value += spfh[other][slot] / distance_sq;
            }
        }
        for part in 0..3 {
            let range = part * BINS..(part + 1) * BINS;
            let sum: f64 = weighted[range.clone()].iter().sum();
            if sum > 0.0 {
                for slot in range {
                    result[index][slot] += weighted[slot] * 100.0 / sum;
                }
            }
        }
    }
    Some(result)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn bin(value: f64, low: f64, high: f64) -> usize {
    (((value - low) / (high - low) * BINS as f64).floor() as usize).min(BINS - 1)
}

fn neighbor_lists(cloud: &[FeaturePoint], cancel: &CancelFlag) -> Option<Vec<Vec<(usize, f64)>>> {
    let mut cells: BTreeMap<(i32, i32, i32), Vec<usize>> = BTreeMap::new();
    for (index, point) in cloud.iter().enumerate() {
        cells
            .entry(feature_voxel_key(point.position)?)
            .or_default()
            .push(index);
    }
    let mut lists = Vec::with_capacity(cloud.len());
    for (index, point) in cloud.iter().enumerate() {
        if index % 64 == 0 && cancel.is_cancelled() {
            return None;
        }
        let (x, y, z) = feature_voxel_key(point.position)?;
        let mut local = Vec::new();
        for dx in -5..=5 {
            for dy in -5..=5 {
                for dz in -5..=5 {
                    if let Some(indices) = cells.get(&(x + dx, y + dy, z + dz)) {
                        for &other in indices {
                            if other == index {
                                continue;
                            }
                            let distance_sq =
                                point.position.distance_squared(cloud[other].position);
                            if distance_sq > 1e-12 && distance_sq <= FEATURE_RADIUS_SQ {
                                local.push((other, distance_sq));
                            }
                        }
                    }
                }
            }
        }
        local.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        local.truncate(MAX_NEIGHBORS);
        lists.push(local);
    }
    Some(lists)
}
