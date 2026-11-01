//! The motions a search starts from: the ones it is given and the ones the
//! two surfaces suggest.

use super::cloud::Cloud;
use super::consensus::{consensus, Hypothesis, Pairing};
use super::descriptor::{describe, Descriptor};
use super::matching::match_descriptions;
use super::seat::Probe;
use super::solve::eigen;
use crate::{
    AlignmentInput, FitBounds, FitRejection, MeshInput, RefinementTermination, Rigid, SeedOrigin,
};
use glam::{DMat3, DQuat, DVec3};

/// Radii of the two descriptions, in cloud spacings.
const DESCRIBE: [f64; 2] = [3.5, 7.];
/// Points of each side that ask for their look-alike, most look-alike pairs
/// taken to the consensus, and motions taken from it.
const ASKED: usize = 4_000;
const PAIRS: usize = 6_000;
const MOTIONS: usize = 12;
/// Probes over which two poses are compared.
const COMPARED: usize = 512;

/// A motion to seat, in the work frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Start {
    pub pose: Rigid,
    pub origin: SeedOrigin,
    /// Ordinal among the motions of its origin.
    pub proposal: u32,
    /// -1 when the surfaces are to meet facing opposite ways.
    pub sign: f64,
    /// Given by the operator or the current placement: seated whatever its
    /// first support.
    pub given: bool,
    /// Common area under the pose, as last read.
    pub support: f64,
    pub termination: RefinementTermination,
}

/// The starts of one search, numbered within each origin as they are added.
#[derive(Default)]
pub(super) struct Starts(pub Vec<Start>);

impl Starts {
    pub(super) fn add(&mut self, pose: Rigid, origin: SeedOrigin, sign: f64) {
        let proposal = self.count(origin);
        self.0.push(Start {
            pose,
            origin,
            proposal,
            sign,
            given: matches!(origin, SeedOrigin::Start | SeedOrigin::Landmarks),
            support: 0.,
            termination: RefinementTermination::NotStarted,
        });
    }

    /// Starts of one origin.
    pub(super) fn count(&self, origin: SeedOrigin) -> u32 {
        let count = self.0.iter().filter(|start| start.origin == origin).count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }
}

/// Root mean square distance between where two poses put the probes.
pub(super) fn apart(first: &Rigid, second: &Rigid, probes: &[Probe]) -> f64 {
    let stride = probes.len().div_ceil(COMPARED).max(1);
    let (mut sum, mut count) = (0., 0.);
    for probe in probes.iter().step_by(stride) {
        sum += first
            .apply(probe.position)
            .distance_squared(second.apply(probe.position));
        count += 1.;
    }
    if count > 0. {
        (sum / count).sqrt()
    } else {
        0.
    }
}

/// Keep the best supported `most` starts that are more than `same` apart,
/// and every given one.
pub(super) fn shortlist(starts: &mut Vec<Start>, probes: &[Probe], same: f64, most: usize) {
    starts.sort_by(|a, b| {
        b.support
            .total_cmp(&a.support)
            .then((a.origin as u8).cmp(&(b.origin as u8)))
            .then(a.proposal.cmp(&b.proposal))
    });
    let mut kept: Vec<Start> = Vec::new();
    for start in starts.iter() {
        let twin = kept.iter().any(|other| {
            other.sign.to_bits() == start.sign.to_bits()
                && apart(&other.pose, &start.pose, probes) < same
        });
        if !twin && (kept.len() < most || start.given) {
            kept.push(*start);
        }
    }
    *starts = kept;
}

/// The proper rotations that carry the principal axes of one cloud onto
/// those of the other, with the centres brought together.
pub(super) fn principal(moving: &Cloud, fixed: &Cloud) -> Vec<Rigid> {
    let frame = |cloud: &Cloud| {
        let (mut centre, mut total) = (DVec3::ZERO, 0.);
        for point in &cloud.points {
            total += point.area;
            centre += (point.position - centre) * (point.area / total);
        }
        let mut spread = [[0f64; 3]; 3];
        for point in &cloud.points {
            let offset = (point.position - centre).to_array();
            for (i, row) in spread.iter_mut().enumerate() {
                for (k, value) in row.iter_mut().enumerate() {
                    *value += point.area * offset[i] * offset[k];
                }
            }
        }
        let (_, vectors) = eigen(spread);
        let (long, middle) = (DVec3::from_array(vectors[2]), DVec3::from_array(vectors[1]));
        (centre, DMat3::from_cols(long, middle, long.cross(middle)))
    };
    let ((from, moving_axes), (to, fixed_axes)) = (frame(moving), frame(fixed));
    [
        DVec3::new(1., 1., 1.),
        DVec3::new(1., -1., -1.),
        DVec3::new(-1., 1., -1.),
        DVec3::new(-1., -1., 1.),
    ]
    .into_iter()
    .filter_map(|flip| {
        let turned = fixed_axes * DMat3::from_diagonal(flip) * moving_axes.transpose();
        let rotation = DQuat::from_mat3(&turned);
        let pose = Rigid::new(rotation, to - rotation * from);
        pose.is_finite().then_some(pose)
    })
    .collect()
}

/// A cloud with a description of the surface around each of its points.
pub(super) struct Described<'a> {
    pub cloud: &'a Cloud,
    pub descriptions: Vec<Descriptor>,
}

impl<'a> Described<'a> {
    /// `sign` is -1 when the surface is to be read facing the other way.
    pub(super) fn new(cloud: &'a Cloud, sign: f64) -> Self {
        let radii = DESCRIBE.map(|spacings| spacings * cloud.spacing);
        Self {
            cloud,
            descriptions: describe(cloud, sign, radii),
        }
    }
}

/// Motions on which look-alike places of the two clouds agree. `sign` is -1
/// when the surfaces are to meet facing opposite ways; `stop` is asked
/// between the stages.
pub(super) fn look_alike(
    moving: &Cloud,
    fixed: &Described<'_>,
    sign: f64,
    stop: &(dyn Fn() -> bool + Sync),
) -> Vec<Hypothesis> {
    let moving = Described::new(moving, sign);
    if stop() {
        return Vec::new();
    }
    let matches = match_descriptions(&moving.descriptions, &fixed.descriptions, ASKED, PAIRS);
    let pairs: Vec<Pairing> = matches
        .iter()
        .map(|found| {
            let (from, to) = (
                &moving.cloud.points[found.moving as usize],
                &fixed.cloud.points[found.fixed as usize],
            );
            Pairing {
                moving: from.position,
                moving_normal: from.normal * sign,
                fixed: to.position,
                fixed_normal: to.normal,
            }
        })
        .collect();
    if stop() {
        return Vec::new();
    }
    consensus(&pairs, fixed.cloud.spacing, MOTIONS, stop)
}

/// The operator's landmarks as a motion in the work frame, or why they give
/// none. Absent when there are no landmarks.
pub(super) fn landmark(
    input: &AlignmentInput<'_>,
    origin: DVec3,
    moving: &Cloud,
    fixed: &Cloud,
) -> Option<Result<Rigid, FitRejection>> {
    if input.landmarks.is_empty() {
        return None;
    }
    let place =
        |mesh: &MeshInput<'_>, point: DVec3| mesh.world_from_local.transform_point3(point) - origin;
    let facing = |mesh: &MeshInput<'_>, normal: DVec3| {
        (mesh.world_from_local.matrix3.inverse().transpose() * normal).normalize_or_zero()
    };
    let from: Vec<DVec3> = input
        .landmarks
        .iter()
        .map(|pair| place(&input.moving, pair.moving_local))
        .collect();
    let to: Vec<DVec3> = input
        .landmarks
        .iter()
        .map(|pair| place(&input.fixed, pair.fixed_local))
        .collect();
    let normals: Option<(Vec<DVec3>, Vec<DVec3>)> = input
        .landmarks
        .iter()
        .map(|pair| {
            pair.normals_local
                .map(|[moving, fixed]| (facing(&input.moving, moving), facing(&input.fixed, fixed)))
        })
        .collect::<Option<Vec<_>>>()
        .map(|both| both.into_iter().unzip());
    let extent = |cloud: &Cloud| {
        let (min, max) = cloud.bounds();
        (min.midpoint(max), (max - min).length())
    };
    let ((moving_center, moving_extent), (fixed_center, fixed_extent)) =
        (extent(moving), extent(fixed));
    Some(
        crate::fit_pairs(
            &from,
            &to,
            normals
                .as_ref()
                .map(|(moving, fixed)| (moving.as_slice(), fixed.as_slice())),
            &FitBounds {
                moving_center,
                moving_extent,
                fixed_center,
                fixed_extent,
            },
        )
        .map(|fit| fit.rigid),
    )
}

#[cfg(test)]
mod tests {
    use super::super::cloud::Gather;
    use super::*;

    fn probes(count: u32) -> Vec<Probe> {
        (0..count)
            .map(|i| Probe {
                position: DVec3::new(f64::from(i), f64::from(i % 7), 0.),
                normal: DVec3::Z,
                area: 1.,
            })
            .collect()
    }

    fn start(shift: f64, support: f64, origin: SeedOrigin) -> Start {
        Start {
            pose: Rigid::new(DQuat::IDENTITY, DVec3::new(shift, 0., 0.)),
            origin,
            proposal: 0,
            sign: 1.,
            given: origin == SeedOrigin::Start,
            support,
            termination: RefinementTermination::NotStarted,
        }
    }

    #[test]
    fn a_shortlist_keeps_distinct_starts_best_first_and_every_given_one() {
        let probes = probes(100);
        let mut starts = vec![
            start(0., 10., SeedOrigin::FeatureSame),
            start(0.1, 30., SeedOrigin::FeatureSame),
            start(5., 20., SeedOrigin::PrincipalFrame),
            start(9., 5., SeedOrigin::PrincipalFrame),
            start(20., 1., SeedOrigin::Start),
        ];
        shortlist(&mut starts, &probes, 1., 2);
        let shifts: Vec<f64> = starts.iter().map(|s| s.pose.translation.x).collect();
        // The twin of the best is dropped, the fourth does not fit, the given
        // one stays.
        assert_eq!(shifts, [0.1, 5., 20.]);
        assert!((apart(&starts[0].pose, &starts[1].pose, &probes) - 4.9).abs() < 1e-12);
    }

    #[test]
    fn starts_are_numbered_within_their_origin() {
        let mut starts = Starts::default();
        starts.add(Rigid::IDENTITY, SeedOrigin::Start, 1.);
        starts.add(Rigid::IDENTITY, SeedOrigin::FeatureSame, 1.);
        starts.add(Rigid::IDENTITY, SeedOrigin::FeatureSame, 1.);
        let numbers: Vec<u32> = starts.0.iter().map(|start| start.proposal).collect();
        assert_eq!(numbers, [0, 0, 1]);
        assert!(starts.0[0].given && !starts.0[1].given);
        assert_eq!(starts.count(SeedOrigin::FeatureSame), 2);
    }

    #[test]
    fn principal_frames_bring_a_turned_box_home() {
        // A 12 x 6 x 2 mm box surface, and the same box turned and moved.
        let truth = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.3, 0.8, -0.5).normalize(), 1.3),
            DVec3::new(7., -4., 2.),
        );
        let cloud = |pose: Rigid| {
            let mut gather = Gather::new(1.);
            let size = DVec3::new(12., 6., 2.);
            for axis in 0..3 {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                for side in [0., 1.] {
                    let corner = |a: f64, b: f64| {
                        let mut point = DVec3::ZERO;
                        point[axis] = side * size[axis];
                        point[u] = a * size[u];
                        point[v] = b * size[v];
                        pose.apply(point)
                    };
                    gather.add_triangle([corner(0., 0.), corner(1., 0.), corner(1., 1.)]);
                    gather.add_triangle([corner(0., 0.), corner(1., 1.), corner(0., 1.)]);
                }
            }
            gather.finish()
        };
        let (moving, fixed) = (cloud(Rigid::IDENTITY), cloud(truth));
        let found = principal(&moving, &fixed);
        assert_eq!(found.len(), 4);
        // One of the four turns is the true one; the others are the box's own
        // half turns.
        let probe = DVec3::new(12., 6., 2.);
        let best = found
            .iter()
            .map(|pose| pose.apply(probe).distance(truth.apply(probe)))
            .fold(f64::INFINITY, f64::min);
        assert!(best < 0.2, "{best}");
    }
}
