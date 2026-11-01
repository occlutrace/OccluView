//! What a pose is worth: how much surface the two scans share under it and
//! how well that surface agrees.
//!
//! A reading takes every probe of one scan to the other scan's surface. The
//! surface within half a millimetre that faces the right way is the common
//! region; the rest of either scan has no counterpart and is left out of the
//! residuals instead of being counted as error. Both directions are read, so
//! surface that only one scan has cannot hide in either.

use super::seat::{pairs, seat, Meet, Probe, Schedule, Target};
use super::solve::{eigen, positive, solve_symmetric};
use crate::{
    CandidateEvidence, Metric, MissingReason, RefinementTermination, ResidualSummary, Rigid,
};
use glam::DVec3;
use rayon::prelude::*;
use std::collections::HashSet;

/// Surface closer than this to the other scan belongs to the common region.
pub(super) const COMMON_MM: f64 = 0.5;
/// Common surface closer than this is seated.
pub(super) const SEATED_MM: f64 = 0.2;
/// Least cosine between the normals of surfaces that face the same way.
pub(super) const FACING: f64 = 0.5;
/// Fewer pairs than this do not tell a size.
const SIZE_PAIRS: usize = 200;
/// A common region no wider than this holds no rotation worth the name.
const MIN_RADIUS_MM: f64 = 0.5;
/// A weak direction of motion has less than this share of the strongest.
const WEAK_SHARE: f64 = 1e-6;

/// One probe of the common region under the pose that was read.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shared {
    /// Where the probe stands, in the target's frame.
    pub at: DVec3,
    /// Normal of the target surface there.
    pub normal: DVec3,
    pub distance: f64,
    pub area: f64,
    /// The probe took part in seating the pose.
    pub trained: bool,
}

/// One direction's reading of a pose.
#[derive(Default)]
pub(super) struct Reading {
    /// Area of all probes.
    pub queried: f64,
    /// Area within the common distance, whichever way it faces.
    pub near: f64,
    /// Area of the common region.
    pub common: f64,
    /// Area of the common region that is seated.
    pub seated: f64,
    /// Common area, each probe counted less the farther it stands off.
    pub score: f64,
    /// Occupied one-millimetre cells of the common region.
    pub cells: u32,
    pub shared: Vec<Shared>,
}

/// Read `probes` under `pose` against `target`. `trained` tells which probes
/// seated the pose.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn read<T: Target>(
    probes: &[Probe],
    trained: &(dyn Fn(usize) -> bool + Sync),
    target: &T,
    pose: Rigid,
    meet: Meet,
) -> Reading {
    let found: Vec<Option<(DVec3, DVec3, f64, bool)>> = probes
        .par_iter()
        .map_init(
            || target.scratch(),
            |scratch, probe| {
                let at = pose.apply(probe.position);
                let (distance, normal) = target.distance(scratch, at, COMMON_MM)?;
                let facing = meet.allows(pose.rotation * probe.normal, normal);
                Some((at, normal, distance, facing))
            },
        )
        .collect();
    let mut reading = Reading::default();
    let mut cells = HashSet::new();
    for (ordinal, (probe, hit)) in probes.iter().zip(&found).enumerate() {
        reading.queried += probe.area;
        let Some((at, normal, distance, facing)) = *hit else {
            continue;
        };
        reading.near += probe.area;
        if !facing {
            continue;
        }
        reading.common += probe.area;
        if distance <= SEATED_MM {
            reading.seated += probe.area;
        }
        let ratio = distance / COMMON_MM;
        reading.score += probe.area * (1. - ratio * ratio).max(0.).powi(2);
        cells.insert([
            at.x.floor() as i64,
            at.y.floor() as i64,
            at.z.floor() as i64,
        ]);
        reading.shared.push(Shared {
            at,
            normal,
            distance,
            area: probe.area,
            trained: trained(ordinal),
        });
    }
    reading.cells = u32::try_from(cells.len()).unwrap_or(u32::MAX);
    reading
}

/// Area-weighted median, root mean square and 95th percentile of distances.
fn summary<'a>(shared: impl Iterator<Item = &'a Shared>) -> Option<ResidualSummary> {
    let mut values: Vec<(f64, f64)> = shared.map(|s| (s.distance, s.area)).collect();
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = values.iter().map(|v| v.1).sum();
    if !positive(total) {
        return None;
    }
    let quantile = |share: f64| {
        let mut passed = 0.;
        values
            .iter()
            .find(|(_, area)| {
                passed += area;
                passed >= total * share
            })
            .map_or(0., |v| v.0)
    };
    let squares: f64 = values.iter().map(|(d, area)| d * d * area).sum();
    Some(ResidualSummary {
        median: quantile(0.5),
        rms: (squares / total).sqrt(),
        p95: quantile(0.95),
    })
}

/// Which motions the common region holds and which it lets slide: the
/// area-weighted mean of the plane constraints of its probes, rotations taken
/// about the region's centre and scaled by its radius, as rising eigenvalues
/// with their vectors (Gelfand et al., Geometrically Stable Sampling for the
/// ICP Algorithm, 3DIM 2003, <https://doi.org/10.1109/IM.2003.1240258>).
pub(super) struct Hold {
    pub values: [f64; 6],
    pub vectors: [[f64; 6]; 6],
    pub weak: Vec<[f64; 6]>,
    pub centre: DVec3,
}

pub(super) fn hold(shared: &[Shared]) -> Option<Hold> {
    let (mut centre, mut total) = (DVec3::ZERO, 0.);
    for s in shared {
        total += s.area;
        centre += (s.at - centre) * (s.area / total);
    }
    if !positive(total) {
        return None;
    }
    let radius = (shared
        .iter()
        .map(|s| s.area * s.at.distance_squared(centre))
        .sum::<f64>()
        / total)
        .sqrt();
    if !positive(radius - MIN_RADIUS_MM) {
        return None;
    }
    let mut matrix = [[0f64; 6]; 6];
    for s in shared {
        let torque = (s.at - centre).cross(s.normal) / radius;
        let row = [
            torque.x, torque.y, torque.z, s.normal.x, s.normal.y, s.normal.z,
        ];
        for (i, line) in matrix.iter_mut().enumerate() {
            for (k, value) in line.iter_mut().enumerate() {
                *value += s.area / total * row[i] * row[k];
            }
        }
    }
    if matrix.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let (values, vectors) = eigen(matrix);
    let values = values.map(|v| v.max(0.));
    let weak = vectors
        .iter()
        .zip(values)
        .filter(|(_, value)| *value <= values[5] * WEAK_SHARE)
        .map(|(vector, _)| *vector)
        .collect();
    Some(Hold {
        values,
        vectors,
        weak,
        centre,
    })
}

/// How much the moving surface would have to grow, as a share of its size,
/// to lie better on the fixed one around `pose`: the size term of a
/// point-to-plane step that may also turn and shift. Two scans of one object
/// read zero; a scan in other units or of a deformed object does not, and no
/// rigid pose removes that. Absent when the surfaces do not determine it.
pub(super) fn size_trend<T: Target>(
    probes: &[Probe],
    target: &T,
    pose: Rigid,
    reach: f64,
    meet: Meet,
) -> Option<f64> {
    let rows = pairs(probes, target, pose, reach, meet);
    let (mut centre, mut total, mut count) = (DVec3::ZERO, 0., 0usize);
    for (at, _, _, weight) in rows.iter().flatten() {
        total += weight;
        centre += (*at - centre) * (weight / total);
        count += 1;
    }
    if count < SIZE_PAIRS {
        return None;
    }
    let radius = (rows
        .iter()
        .flatten()
        .map(|(at, _, _, weight)| weight * at.distance_squared(centre))
        .sum::<f64>()
        / total)
        .sqrt();
    if !positive(radius - MIN_RADIUS_MM) {
        return None;
    }
    let (mut matrix, mut right) = ([[0f64; 7]; 7], [0f64; 7]);
    for (at, normal, offset, weight) in rows.iter().flatten() {
        let arm = (*at - centre) / radius;
        let torque = arm.cross(*normal);
        let row = [
            torque.x,
            torque.y,
            torque.z,
            normal.x,
            normal.y,
            normal.z,
            arm.dot(*normal),
        ];
        for (i, line) in matrix.iter_mut().enumerate() {
            right[i] -= weight / total * row[i] * offset;
            for (k, value) in line.iter_mut().enumerate() {
                *value += weight / total * row[i] * row[k];
            }
        }
    }
    // The size term is in units of the radius: a share of the surface's size.
    solve_symmetric(&matrix, &right).map(|found| found[6] / radius)
}

/// Largest drift of `centre`, in millimetres, and largest turn, in degrees,
/// when `pose` is seated again with each eighth of the surface around
/// `centre` left out. A pose that rests on one part of the common surface
/// moves when that part is taken away. Absent when a seating fails.
pub(super) fn left_out_drift<T: Target>(
    probes: &[Probe],
    target: &T,
    pose: Rigid,
    centre: DVec3,
    schedule: &Schedule<'_>,
) -> Option<[f64; 2]> {
    let part = |probe: &Probe| {
        let offset = pose.apply(probe.position) - centre;
        usize::from(offset.x >= 0.)
            | (usize::from(offset.y >= 0.) << 1)
            | (usize::from(offset.z >= 0.) << 2)
    };
    let home = pose.inverse().apply(centre);
    let mut drift = [0f64; 2];
    for left_out in 0..8 {
        let rest: Vec<Probe> = probes
            .iter()
            .filter(|probe| part(probe) != left_out)
            .copied()
            .collect();
        if rest.len() == probes.len() {
            continue;
        }
        let again = seat(&rest, target, pose, schedule);
        if !matches!(
            again.termination,
            RefinementTermination::StepSmall | RefinementTermination::IterationLimit
        ) {
            return None;
        }
        let turn = (again.pose.rotation * pose.rotation.conjugate())
            .to_axis_angle()
            .1;
        drift[0] = drift[0].max(again.pose.apply(home).distance(centre));
        drift[1] = drift[1].max(turn.to_degrees().abs());
    }
    Some(drift)
}

/// Join the two directions into the evidence of one pose. `areas` are the
/// eligible areas of the moving and the fixed scan.
pub(super) fn join(forward: &Reading, backward: &Reading, areas: [f64; 2]) -> CandidateEvidence {
    let mut evidence = CandidateEvidence {
        eligible_area_mm2: Metric::Measured(areas),
        coverage_02: Metric::Measured([forward.seated / areas[0], backward.seated / areas[1]]),
        coverage_05: Metric::Measured([forward.common / areas[0], backward.common / areas[1]]),
        score: Metric::Measured(forward.score + backward.score),
        ..CandidateEvidence::default()
    };
    let common = forward.common.min(backward.common);
    let smaller = areas[0].min(areas[1]);
    evidence.common_area_mm2 = Metric::Measured(common);
    evidence.overlap_smaller = Metric::Measured((common / smaller).min(1.));
    evidence.effective_cells = Metric::Measured(forward.cells.min(backward.cells));
    let both = || forward.shared.iter().chain(&backward.shared);
    let missing = Metric::Missing(MissingReason::NoSupport);
    evidence.euclidean_mm = summary(both()).map_or(missing, Metric::Measured);
    let (near, shared) = (
        forward.near + backward.near,
        forward.common + backward.common,
    );
    if near > 0. {
        evidence.orientation_fraction = Metric::Measured(shared / near);
    }
    if shared > 0. {
        evidence.inlier_ratio = Metric::Measured((forward.seated + backward.seated) / shared);
        evidence.reciprocal_fraction =
            Metric::Measured(common / forward.common.max(backward.common));
    }
    // Probes that seated the pose against probes that did not.
    let rms = |trained: bool| summary(both().filter(|s| s.trained == trained)).map(|s| s.rms);
    if let (Some(training), Some(holdout)) = (rms(true), rms(false)) {
        evidence.holdout_ratio = Metric::Measured(holdout / training.max(0.02));
        evidence.holdout_complete = true;
    }
    match hold(&forward.shared) {
        Some(held) => {
            evidence.info_eigenvalues = Metric::Measured(held.values);
            evidence.info_eigenvectors = Metric::Measured(held.vectors);
            evidence.weak_twists = held.weak;
        }
        None => evidence.info_eigenvalues = Metric::Missing(MissingReason::Degenerate),
    }
    evidence
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]
    use super::super::cloud::{Cloud, Gather};
    use super::*;

    const SAME: Meet = Meet {
        sign: 1.,
        facing: FACING,
    };

    fn sheet(width: f64, lift: impl Fn(f64, f64) -> f64) -> Cloud {
        let mut gather = Gather::new(0.25);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = (width / 0.25) as usize;
        let at = |i: usize, j: usize| {
            #[allow(clippy::cast_precision_loss)]
            let (x, y) = (i as f64 * 0.25, j as f64 * 0.25);
            DVec3::new(x, y, lift(x, y))
        };
        for i in 0..steps {
            for j in 0..40 {
                gather.add_triangle([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                gather.add_triangle([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        gather.finish()
    }

    fn probes(cloud: &Cloud) -> Vec<Probe> {
        cloud
            .points
            .iter()
            .map(|point| Probe {
                position: point.position,
                normal: point.normal,
                area: point.area,
            })
            .collect()
    }

    #[test]
    fn half_a_sheet_on_the_whole_shares_its_own_area_both_ways() {
        let bumps = |x: f64, y: f64| (x * 0.8).sin() + (y * 0.5).cos();
        let whole = sheet(20., bumps);
        let half = sheet(10., bumps);
        let forward = read(
            &probes(&half),
            &|i| i % 2 == 0,
            &whole,
            Rigid::IDENTITY,
            SAME,
        );
        let backward = read(&probes(&whole), &|_| false, &half, Rigid::IDENTITY, SAME);
        let evidence = join(&forward, &backward, [half.area(), whole.area()]);
        let Metric::Measured(overlap) = evidence.overlap_smaller else {
            panic!("overlap is measured");
        };
        assert!(overlap > 0.9, "{overlap}");
        let Metric::Measured([of_moving, of_fixed]) = evidence.coverage_05 else {
            panic!("coverage is measured");
        };
        assert!(
            of_moving > 0.9 && (0.4..0.6).contains(&of_fixed),
            "{of_moving} {of_fixed}"
        );
        let Metric::Measured(residual) = evidence.euclidean_mm else {
            panic!("residuals are measured");
        };
        assert!(residual.p95 < 0.02, "{residual:?}");
        assert!(evidence.holdout_complete);
        let Metric::Measured(values) = evidence.info_eigenvalues else {
            panic!("the bumpy sheet holds every motion");
        };
        assert!(values[0] > 1e-3, "{values:?}");
    }

    #[test]
    fn a_flat_sheet_lets_three_motions_slide() {
        let flat = sheet(10., |_, _| 0.);
        let reading = read(&probes(&flat), &|_| true, &flat, Rigid::IDENTITY, SAME);
        let held = hold(&reading.shared).unwrap();
        assert_eq!(held.weak.len(), 3, "{:?}", held.values);
    }

    #[test]
    fn a_lifted_sheet_shares_nothing_beyond_the_common_distance() {
        let flat = sheet(10., |_, _| 0.);
        let lifted = Rigid::new(glam::DQuat::IDENTITY, DVec3::new(0., 0., 0.8));
        let reading = read(&probes(&flat), &|_| true, &flat, lifted, SAME);
        assert_eq!(reading.common, 0.);
        let seated = Rigid::new(glam::DQuat::IDENTITY, DVec3::new(0., 0., 0.1));
        let reading = read(&probes(&flat), &|_| true, &flat, seated, SAME);
        assert!(reading.seated > 0.9 * reading.queried);
    }
}
