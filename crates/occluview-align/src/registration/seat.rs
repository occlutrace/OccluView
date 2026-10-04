//! Seating a moving surface on a fixed one from a nearby start.
//!
//! Each round pairs every moving point with the nearest fixed surface within
//! a reach, weighs the pair by how far the point stands off that surface
//! (Tukey's biweight, so a point with no counterpart has no say), and takes
//! the point-to-plane step. The reach narrows from round to round: a wide
//! reach finds the basin, a narrow one keeps only the surface both scans
//! share. Sums are taken in point order, so a result does not depend on how
//! the pairing was spread over threads.

use super::cloud::Cloud;
use super::solve::PlaneSystem;
use crate::{RefinementTermination, Rigid, SurfaceIndex};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, QueryOutcome, SurfaceQueryScratch};
use rayon::prelude::*;

/// A step smaller than this at the rim of the paired surface ends a reach.
const SETTLED_MM: f64 = 2e-4;
/// Fewer weighted pairs than this do not determine a motion.
const MIN_PAIRS: usize = 12;

/// A moving point with the area it stands for.
#[derive(Clone, Copy, Debug)]
pub(super) struct Probe {
    pub position: DVec3,
    pub normal: DVec3,
    pub area: f64,
}

/// A fixed surface that can be asked for its nearest point.
pub(super) trait Target: Sync {
    /// Working memory of one thread's queries.
    type Scratch;
    fn scratch(&self) -> Self::Scratch;
    /// The nearest surface point within `reach` and the surface normal there.
    fn nearest(
        &self,
        scratch: &mut Self::Scratch,
        position: DVec3,
        reach: f64,
    ) -> Option<(DVec3, DVec3)>;
    /// Distance from `position` to the surface when it is within `reach` and
    /// the surface covers the place, with the surface normal there.
    fn distance(
        &self,
        scratch: &mut Self::Scratch,
        position: DVec3,
        reach: f64,
    ) -> Option<(f64, DVec3)>;
}

impl Target for Cloud {
    type Scratch = ();
    fn scratch(&self) {}
    fn nearest(&self, (): &mut (), position: DVec3, reach: f64) -> Option<(DVec3, DVec3)> {
        let (ordinal, _) = Cloud::nearest(self, position, reach)?;
        let point = &self.points[ordinal as usize];
        Some((point.position, point.normal))
    }
    fn distance(&self, (): &mut (), position: DVec3, reach: f64) -> Option<(f64, DVec3)> {
        let (ordinal, offset) = self.surface_distance(position, reach)?;
        Some((offset, self.points[ordinal as usize].normal))
    }
}

/// The source triangles themselves. A nearest point on the open border is no
/// counterpart: the moving point lies beyond what the fixed scan covers.
pub(super) struct Exact<'a> {
    pub index: &'a SurfaceIndex,
    pub control: &'a GeometryControl,
}

impl Target for Exact<'_> {
    type Scratch = Option<SurfaceQueryScratch>;
    fn scratch(&self) -> Self::Scratch {
        SurfaceQueryScratch::new(self.control).ok()
    }
    fn nearest(
        &self,
        scratch: &mut Self::Scratch,
        position: DVec3,
        reach: f64,
    ) -> Option<(DVec3, DVec3)> {
        match self
            .index
            .nearest_with_scratch(position, reach, scratch.as_mut()?)
        {
            QueryOutcome::Complete(Some(hit)) if !hit.on_border => Some((hit.point, hit.normal)),
            _ => None,
        }
    }
    fn distance(
        &self,
        scratch: &mut Self::Scratch,
        position: DVec3,
        reach: f64,
    ) -> Option<(f64, DVec3)> {
        let (point, normal) = self.nearest(scratch, position, reach)?;
        Some((point.distance(position), normal))
    }
}

/// How the two surfaces are taken to meet.
#[derive(Clone, Copy, Debug)]
pub(super) struct Meet {
    /// -1 when the surfaces meet facing opposite ways.
    pub sign: f64,
    /// Least cosine between the normals of a pair.
    pub facing: f64,
}

impl Meet {
    /// Whether a moving normal, already turned by the pose, and a fixed
    /// normal face the way the surfaces are to meet.
    pub(super) fn allows(self, moving: DVec3, fixed: DVec3) -> bool {
        moving.dot(fixed) * self.sign >= self.facing
    }
}

/// The plan of one seating: `rounds` rounds at each of `reaches`, which
/// narrow. `stop` is asked before and after every round.
pub(super) struct Schedule<'a> {
    pub meet: Meet,
    pub reaches: &'a [f64],
    pub rounds: u32,
    pub stop: &'a (dyn Fn() -> Option<RefinementTermination> + Sync),
}

/// How one seating ended.
#[derive(Clone, Copy, Debug)]
pub(super) struct Seated {
    pub pose: Rigid,
    pub rounds: u32,
    pub termination: RefinementTermination,
}

/// A probe under a pose and the surface it pairs with: where the probe
/// stands, the surface normal there, its offset along that normal and its
/// weight.
type Pair = (DVec3, DVec3, f64, f64);

/// Pair every probe with the target under `pose`, in probe order. A probe
/// weighs its area, less the farther it stands off, down to nothing at
/// `reach` (Tukey's biweight).
pub(super) fn pairs<T: Target>(
    probes: &[Probe],
    target: &T,
    pose: Rigid,
    reach: f64,
    meet: Meet,
) -> Vec<Option<Pair>> {
    probes
        .par_iter()
        .map_init(
            || target.scratch(),
            |scratch, probe| {
                let at = pose.apply(probe.position);
                let (point, normal) = target.nearest(scratch, at, reach)?;
                if !meet.allows(pose.rotation * probe.normal, normal) {
                    return None;
                }
                let offset = (at - point).dot(normal);
                let ratio = offset / reach;
                let weight = probe.area * (1. - ratio * ratio).max(0.).powi(2);
                (weight > 0.).then_some((at, normal, offset, weight))
            },
        )
        .collect()
}

/// One round's pairs, summed in point order.
struct Round {
    system: PlaneSystem,
    pairs: usize,
    /// Weighted centre of the paired points.
    pivot: DVec3,
    /// Weighted mean squared distance of the paired points from `pivot`.
    spread: f64,
}

impl Round {
    fn of(found: &[Option<Pair>]) -> Self {
        let (mut pivot, mut total, mut pairs) = (DVec3::ZERO, 0., 0usize);
        for (at, _, _, weight) in found.iter().flatten() {
            total += weight;
            pivot += (*at - pivot) * (weight / total);
            pairs += 1;
        }
        let mut system = PlaneSystem::new(pivot);
        let mut spread = 0.;
        for (at, normal, offset, weight) in found.iter().flatten() {
            system.add(*at, *normal, *offset, *weight);
            spread += weight * at.distance_squared(pivot);
        }
        Self {
            system,
            pairs,
            pivot,
            spread: if total > 0. { spread / total } else { 0. },
        }
    }
}

/// Area of `probes` that lies on `target` under `pose`, each probe counted
/// less the farther it stands off, down to nothing at `reach`.
pub(super) fn support<T: Target>(
    probes: &[Probe],
    target: &T,
    pose: Rigid,
    reach: f64,
    meet: Meet,
) -> f64 {
    let parts: Vec<f64> = probes
        .par_iter()
        .map_init(
            || target.scratch(),
            |scratch, probe| {
                let at = pose.apply(probe.position);
                let Some((distance, normal)) = target.distance(scratch, at, reach) else {
                    return 0.;
                };
                if !meet.allows(pose.rotation * probe.normal, normal) {
                    return 0.;
                }
                let ratio = distance / reach;
                probe.area * (1. - ratio * ratio).max(0.).powi(2)
            },
        )
        .collect();
    parts.iter().sum()
}

/// Seat `probes` on `target` from `start`.
pub(super) fn seat<T: Target>(
    probes: &[Probe],
    target: &T,
    start: Rigid,
    schedule: &Schedule<'_>,
) -> Seated {
    let mut seated = Seated {
        pose: start,
        rounds: 0,
        termination: RefinementTermination::NotStarted,
    };
    for &reach in schedule.reaches {
        seated.termination = RefinementTermination::IterationLimit;
        for _ in 0..schedule.rounds {
            match advance(probes, target, seated.pose, reach, schedule) {
                Ok((pose, moved)) => {
                    seated.pose = pose;
                    seated.rounds += 1;
                    if moved < SETTLED_MM {
                        seated.termination = RefinementTermination::StepSmall;
                        break;
                    }
                }
                Err(ended) => {
                    seated.termination = ended;
                    return seated;
                }
            }
        }
    }
    seated
}

/// One round: the next pose, and how far the round carried a point at the
/// rim of the paired surface.
fn advance<T: Target>(
    probes: &[Probe],
    target: &T,
    pose: Rigid,
    reach: f64,
    schedule: &Schedule<'_>,
) -> Result<(Rigid, f64), RefinementTermination> {
    if let Some(ended) = (schedule.stop)() {
        return Err(ended);
    }
    let found = Round::of(&pairs(probes, target, pose, reach, schedule.meet));
    // A round cut short by the clock has read only some of the points.
    if let Some(ended) = (schedule.stop)() {
        return Err(ended);
    }
    if found.pairs < MIN_PAIRS {
        return Err(RefinementTermination::NoCorrespondences);
    }
    let motion = found.system.step().ok_or(RefinementTermination::Singular)?;
    let next = motion.compose(&pose);
    if !next.is_finite() {
        return Err(RefinementTermination::NumericalTrialRejected);
    }
    let (_, angle) = motion.rotation.to_axis_angle();
    let moved = angle.abs() * found.spread.sqrt() + motion.apply(found.pivot).distance(found.pivot);
    Ok((next, moved))
}

#[cfg(test)]
mod tests {
    use super::super::cloud::Gather;
    use super::*;
    use glam::DQuat;

    fn bumpy(spacing: f64) -> Cloud {
        let height = |x: f64, y: f64| (x * 0.9).sin() * 1.5 + (y * 0.6).cos() * (x * 0.3).sin();
        let at = |i: usize, j: usize| {
            #[allow(clippy::cast_precision_loss)]
            let (x, y) = (i as f64 * 0.25, j as f64 * 0.25);
            DVec3::new(x, y, height(x, y))
        };
        let mut gather = Gather::new(spacing);
        for i in 0..80 {
            for j in 0..80 {
                gather.add_triangle([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                gather.add_triangle([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        gather.finish()
    }

    fn plan<'a>(
        sign: f64,
        reaches: &'a [f64],
        rounds: u32,
        stop: &'a (dyn Fn() -> Option<RefinementTermination> + Sync),
    ) -> Schedule<'a> {
        Schedule {
            meet: Meet { sign, facing: 0.5 },
            reaches,
            rounds,
            stop,
        }
    }

    fn probes(cloud: &Cloud, keep: impl Fn(DVec3) -> bool) -> Vec<Probe> {
        cloud
            .points
            .iter()
            .filter(|point| keep(point.position))
            .map(|point| Probe {
                position: point.position,
                normal: point.normal,
                area: point.area,
            })
            .collect()
    }

    #[test]
    fn a_part_of_a_surface_seats_back_on_the_whole() {
        let fixed = bumpy(0.3);
        // Half of the surface, moved a little.
        let part = probes(&fixed, |p| p.x < 10.);
        let away = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.2, 0.5, 0.8).normalize(), 0.05),
            DVec3::new(0.4, -0.3, 0.25),
        );
        let seated = seat(&part, &fixed, away, &plan(1., &[2., 1., 0.6], 30, &|| None));
        let worst = part
            .iter()
            .map(|probe| seated.pose.apply(probe.position).distance(probe.position))
            .fold(0., f64::max);
        assert!(worst < 0.02, "{worst} after {} rounds", seated.rounds);
        assert_eq!(seated.termination, RefinementTermination::StepSmall);
    }

    #[test]
    fn surfaces_facing_apart_do_not_pair_unless_asked() {
        let fixed = bumpy(0.3);
        let all = probes(&fixed, |_| true);
        let none = seat(
            &all,
            &fixed,
            Rigid::IDENTITY,
            &plan(-1., &[1.], 5, &|| None),
        );
        assert_eq!(none.termination, RefinementTermination::NoCorrespondences);
        let flipped: Vec<Probe> = all
            .iter()
            .map(|probe| Probe {
                normal: -probe.normal,
                ..*probe
            })
            .collect();
        let some = seat(
            &flipped,
            &fixed,
            Rigid::IDENTITY,
            &plan(-1., &[1.], 5, &|| None),
        );
        assert_eq!(some.termination, RefinementTermination::StepSmall);
    }

    #[test]
    fn a_stop_request_is_reported() {
        let fixed = bumpy(0.5);
        let all = probes(&fixed, |_| true);
        let ended = || Some(RefinementTermination::Deadline);
        let seated = seat(&all, &fixed, Rigid::IDENTITY, &plan(1., &[1.], 5, &ended));
        assert_eq!(seated.termination, RefinementTermination::Deadline);
        assert_eq!(seated.rounds, 0);
    }
}
