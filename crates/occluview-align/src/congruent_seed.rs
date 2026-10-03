//! Descriptor-free congruent four-point proposals, independently derived from
//! Aiger, Mitra and Cohen-Or (2008), <https://doi.org/10.1145/1360612.1360684>,
//! and Mellado, Aiger and Mitra (2014), <https://doi.org/10.1111/cgf.12446>.
//! Intersecting base segments retain their affine ratios under rigid motion.
//! Length bins select candidate segments; ratio-point cells join segment pairs.
//! Work/output caps replace exhaustive largest-common-pointset optimization.

use crate::{PreparedSurface, Rigid};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryStop};
use std::collections::BTreeMap;

/// Up to 256 finite proper transforms from 64 fixed-seed near-planar bases.
/// No normals, descriptors, centroid equality or confidence floor is required.
#[expect(
    clippy::too_many_lines,
    reason = "bounded distance-bin and affine-ratio joins share one reservation"
)]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn congruent_basis_hypotheses(
    source: &PreparedSurface,
    target: &PreparedSurface,
    control: &GeometryControl,
) -> Result<Vec<Rigid>, GeometryStop> {
    let _memory = control.reserve(12 * 1024 * 1024)?;
    let points = |surface: &PreparedSurface| {
        let samples = &surface.samples[0].samples;
        let count = samples.len().min(256);
        (0..count)
            .map(|i| samples[i * samples.len() / count].point)
            .collect::<Vec<_>>()
    };
    let source = points(source);
    let target = points(target);
    if source.len() < 4 || target.len() < 4 {
        return Ok(Vec::new());
    }
    let mut bins: BTreeMap<i64, Vec<(usize, usize)>> = BTreeMap::new();
    for i in 0..target.len() {
        for j in i + 1..target.len() {
            control.charge_point_pairs(1)?;
            let d = target[i].distance(target[j]);
            if !d.is_finite() || d >= 1e12 {
                continue;
            }
            let entries = bins.entry((d / 0.25).floor() as i64).or_default();
            entries
                .try_reserve(2)
                .map_err(|_| GeometryStop::ResourceLimit)?;
            entries.extend([(i, j), (j, i)]);
        }
    }
    let mut state = 0x4f56_5f41_4c52_3107u64;
    let mut output = Vec::new();
    output
        .try_reserve_exact(256)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut quads = 0;
    for _ in 0..64 {
        control.charge_operations(1)?;
        let ids: [usize; 4] = std::array::from_fn(|_| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            (crate::sample::mix_seed(state) % source.len() as u64) as usize
        });
        if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
            continue;
        }
        let initial = ids.map(|i| source[i]);
        let base = [[0, 1, 2, 3], [0, 2, 1, 3], [0, 3, 1, 2]]
            .into_iter()
            .find_map(|order| {
                let corners = order.map(|i| initial[i]);
                crossing(corners).map(|ratios| (corners, ratios))
            });
        let Some((base, ratios)) = base else {
            continue;
        };
        let lengths = [base[0].distance(base[1]), base[2].distance(base[3])];
        if lengths[0].max(lengths[1]) < 4. {
            continue;
        }
        let pair_lists: [Vec<(usize, usize)>; 2] = lengths.map(|length| {
            let key = (length / 0.25).floor() as i64;
            bins.range(key - 2..=key + 2)
                .flat_map(|(_, pairs)| pairs.iter().copied())
                .collect()
        });
        let mut intersections: BTreeMap<[i64; 3], Vec<(usize, usize, DVec3)>> = BTreeMap::new();
        for &(i, j) in &pair_lists[0] {
            control.charge_point_pairs(1)?;
            if (target[i].distance(target[j]) - lengths[0]).abs() > 0.5 {
                continue;
            }
            let p = target[i].lerp(target[j], ratios[0]);
            let Some(key) = cell(p) else {
                continue;
            };
            let records = intersections.entry(key).or_default();
            records
                .try_reserve(1)
                .map_err(|_| GeometryStop::ResourceLimit)?;
            records.push((i, j, p));
        }
        for &(k, l) in &pair_lists[1] {
            control.charge_point_pairs(1)?;
            if (target[k].distance(target[l]) - lengths[1]).abs() > 0.5 {
                continue;
            }
            let p = target[k].lerp(target[l], ratios[1]);
            let Some(key) = cell(p) else {
                continue;
            };
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        control.charge_operations(1)?;
                        let near = [key[0] + dx, key[1] + dy, key[2] + dz];
                        if let Some(records) = intersections.get(&near) {
                            for &(i, j, q) in records {
                                control.charge_point_pairs(1)?;
                                if i == k || i == l || j == k || j == l || p.distance(q) > 0.5 {
                                    continue;
                                }
                                if quads >= 4_096 || output.len() >= 256 {
                                    return Ok(output);
                                }
                                quads += 1;
                                let other = [target[i], target[j], target[k], target[l]];
                                let Some(other_ratios) = crossing(other) else {
                                    continue;
                                };
                                if (ratios[0] - other_ratios[0]).abs() > 0.05
                                    || (ratios[1] - other_ratios[1]).abs() > 0.05
                                {
                                    continue;
                                }
                                let mut consistent = true;
                                for a in 0..4 {
                                    for b in a + 1..4 {
                                        control.charge_point_pairs(1)?;
                                        consistent &= (base[a].distance(base[b])
                                            - other[a].distance(other[b]))
                                        .abs()
                                            <= 0.5;
                                    }
                                }
                                if consistent {
                                    if let Some(pose) =
                                        crate::proposal_geometry::fit_geometry_pairs(&base, &other)
                                    {
                                        output.push(pose);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(output)
}

/// Closest points of two supporting lines; crossing parameters and plane gap
/// are invariant under all proper rigid transforms.
fn crossing(p: [DVec3; 4]) -> Option<[f64; 2]> {
    let a = p[1] - p[0];
    let b = p[3] - p[2];
    let delta = p[2] - p[0];
    let cross = a.cross(b);
    let denominator = cross.length_squared();
    if !denominator.is_finite() || denominator <= 0.04 {
        return None;
    }
    let first = delta.cross(b).dot(cross) / denominator;
    let second = delta.cross(a).dot(cross) / denominator;
    if !(0.05..=0.95).contains(&first) || !(0.05..=0.95).contains(&second) {
        return None;
    }
    (p[0].lerp(p[1], first).distance(p[2].lerp(p[3], second)) <= 0.3).then_some([first, second])
}
#[allow(clippy::cast_possible_truncation)]
fn cell(p: DVec3) -> Option<[i64; 3]> {
    let q = (p / 0.5).floor();
    (q.is_finite() && q.abs().max_element() < 1e12).then(|| q.to_array().map(|v| v as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn congruent_ratios_are_rigid_invariants() {
        let base = [
            DVec3::new(-4., 0., 0.),
            DVec3::new(6., 0., 0.),
            DVec3::new(0., -3., 0.),
            DVec3::new(0., 7., 0.),
        ];
        let pose = Rigid::new(
            glam::DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 2.1),
            DVec3::new(73., -21., 15.),
        );
        let a = crossing(base).unwrap();
        let b = crossing(base.map(|p| pose.apply(p))).unwrap();
        assert!((a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12);
        assert!(crossing([DVec3::ZERO; 4]).is_none());
    }
}
