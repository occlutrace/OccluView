//! The search for the rigid motion that brings a moving scan onto a fixed one.
//!
//! Scans that are only partly alike share no centre, no extent and no
//! outline, so nothing here assumes them. The search reads both surfaces as
//! even clouds, pairs places whose surroundings look alike, keeps the motions
//! on which many such pairs agree, and adds the motions it was given: the
//! current placement, the operator's landmarks and seeds, and the principal
//! frames. It then seats every motion worth keeping, first on coarse clouds
//! with a wide reach, then on fine clouds, then on the triangles themselves,
//! and reads what each seated pose is worth in both directions.
//!
//! Every step is a fixed amount of work in a fixed order, so equal input
//! gives an equal result unless the clock or the caller ends the search
//! early. Ended early, the search returns the poses it has, each with the
//! evidence it could still read.

mod cloud;
mod consensus;
mod descriptor;
mod evidence;
mod matching;
mod seat;
mod solve;
mod starts;
mod surface;

use crate::{
    AlignmentInput, CandidateEvidence, Completion, FitRejection, Metric, MissingReason,
    NormalPolicy, RefinementTermination, Rigid, SearchControl, SearchProfile, SearchSettings,
    SeedOrigin,
};
use seat::{seat, Meet, Probe, Schedule, Target};
use starts::{apart, shortlist, Described, Start, Starts};
use std::time::Instant;
use surface::{Layout, Side};

/// Spacing of the fine clouds, in millimetres, and the most points one may
/// have; a larger surface gets a wider spacing.
const FINE_MM: f64 = 0.3;
const FINE_POINTS: f64 = 300_000.;
/// The coarse clouds give the smaller surface about this many points, within
/// these spacings, and the larger surface at most `COARSE_MOST` points.
const COARSE_POINTS: f64 = 3_000.;
const COARSE_MM: [f64; 2] = [0.5, 1.];
const COARSE_MOST: f64 = 9_000.;
/// A cloud has about this many points for every spacing squared of area: a
/// surface seldom lies along the cells.
const POINTS_PER_CELL: f64 = 1.5;
/// Motions seated on the coarse clouds and on the fine clouds.
const COARSE_KEPT: usize = 8;
const FINE_KEPT: usize = 5;
/// Reaches of the coarse seating, in coarse spacings, and of the later ones
/// in millimetres; a cloud answers evenly down to twice its spacing.
const COARSE_REACH: [f64; 3] = [3., 2., 1.5];
const FINE_REACH: [f64; 2] = [1.2, 0.6];
const EXACT_REACH: [f64; 2] = [0.4, 0.25];
const ROUNDS: u32 = 12;
const COARSE_ROUNDS: u32 = 8;
/// Least cosine between paired normals while the pose is still rough.
const ROUGH_FACING: f64 = 0.3;
/// Probes that rank and seat a pose on the coarse clouds, probes that seat it
/// on the fine clouds and the triangles, and probes that read its evidence.
const COARSE_PROBES: usize = 4_000;
const SEAT_PROBES: usize = 6_000;
const READ_PROBES: usize = 20_000;
/// A rival with less than this share of the best pose's support cannot
/// change what the best pose is worth, and is not seated further.
const RIVAL_SHARE: f64 = 0.25;
/// Two poses that move the surface less than this apart, root mean square,
/// are one answer: on the coarse clouds in spacings, later in millimetres.
const SAME_COARSE: f64 = 1.;
const SAME_MM: f64 = 0.25;
/// A pose this near a better one, whose own seating ran out of rounds, lies
/// in the better pose's basin.
const SAME_BASIN_MM: f64 = 1.;
/// Reach within which the two surfaces are compared for size.
const SIZE_REACH: f64 = 2.;
/// Rounds of each seating with one part of the surface left out.
const LEFT_OUT_ROUNDS: u32 = 3;

/// One seated pose and what it is worth.
pub(crate) struct Found {
    /// Correction of the moving scan's authored placement.
    pub pose: Rigid,
    pub origin: SeedOrigin,
    /// Ordinal among the motions of its origin.
    pub proposal: u32,
    pub evidence: CandidateEvidence,
    pub termination: RefinementTermination,
}

/// The outcome of one search.
#[derive(Default)]
pub(crate) struct Registration {
    /// Best first; never empty when both surfaces are usable and the search
    /// reached its starts.
    pub found: Vec<Found>,
    /// Why the search ended early, if it did.
    pub stopped: Option<Completion>,
    /// Eligible areas of the moving and the fixed scan.
    pub areas: [f64; 2],
    /// Triangles of the moving and the fixed scan left out for naming a
    /// vertex the scan does not have.
    pub omitted: [usize; 2],
    /// Motions examined per origin.
    pub examined: Vec<(SeedOrigin, u32)>,
    /// Seating rounds taken in all.
    pub rounds: u64,
    /// Whether the triangles of the moving and the fixed scan face one way
    /// across shared edges; unknown when the triangles were not indexed.
    pub coherent: [Option<bool>; 2],
    /// Why the operator's landmarks gave no motion, if they did not.
    pub landmark_rejection: Option<FitRejection>,
}

#[cfg(feature = "search-probe")]
fn note(stage: &str, since: Instant, detail: std::fmt::Arguments<'_>) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stdout().lock(),
        "ALIGN_STAGE {stage} at={:.3}s {detail}",
        since.elapsed().as_secs_f64()
    );
}
#[cfg(not(feature = "search-probe"))]
fn note(_: &str, _: Instant, _: std::fmt::Arguments<'_>) {}

/// Both scans read, and the probes every stage takes of them.
struct Search<'a> {
    settings: &'a SearchSettings,
    control: &'a SearchControl,
    began: Instant,
    layout: Layout,
    moving: Side,
    fixed: Side,
    moving_coarse: Vec<Probe>,
    fixed_coarse: Vec<Probe>,
    moving_seat: Vec<Probe>,
    moving_read: Vec<Probe>,
    fixed_read: Vec<Probe>,
}

impl Search<'_> {
    fn ended(&self) -> Option<Completion> {
        self.control.checkpoint(self.settings.wall_limit)
    }

    /// The end of the search as a seating reports it.
    fn seating_ended(&self) -> Option<RefinementTermination> {
        self.ended().map(|done| {
            if done == Completion::Cancelled {
                RefinementTermination::Cancelled
            } else {
                RefinementTermination::Deadline
            }
        })
    }

    /// A correction of the authored placement as a pose of the work frame,
    /// and back; the two differ by the origin only.
    fn to_work(&self, pose: Rigid) -> Rigid {
        let origin = self.layout.origin;
        Rigid::new(
            pose.rotation,
            pose.rotation * origin + pose.translation - origin,
        )
    }

    fn to_world(&self, pose: Rigid) -> Rigid {
        let origin = self.layout.origin;
        Rigid::new(
            pose.rotation,
            pose.translation + origin - pose.rotation * origin,
        )
    }

    /// Every motion to try: the given ones for each facing, then, unless the
    /// search is local, the ones the surfaces suggest.
    fn starts(&self, input: &AlignmentInput<'_>, out: &mut Registration) -> Vec<Start> {
        let signs: &[f64] = match self.settings.normal_policy {
            NormalPolicy::Match => &[1.],
            NormalPolicy::Opposed => &[-1.],
            NormalPolicy::Unsigned => &[1., -1.],
        };
        let landmark = starts::landmark(
            input,
            self.layout.origin,
            &self.moving.fine,
            &self.fixed.fine,
        );
        out.landmark_rejection = landmark.and_then(Result::err);
        let mut all = Starts::default();
        for &sign in signs {
            if input.seeds.is_empty() {
                all.add(Rigid::IDENTITY, SeedOrigin::Start, sign);
            }
            for seed in input.seeds {
                all.add(self.to_work(*seed), SeedOrigin::Start, sign);
            }
            if let Some(Ok(pose)) = landmark {
                all.add(pose, SeedOrigin::Landmarks, sign);
            }
        }
        let stop = || self.ended().is_some();
        if self.settings.profile != SearchProfile::Local && !stop() {
            let fixed = Described::new(&self.fixed.coarse, 1.);
            for &sign in signs {
                let family = if sign > 0. {
                    SeedOrigin::FeatureSame
                } else {
                    SeedOrigin::FeatureOpposed
                };
                let motions = starts::look_alike(&self.moving.coarse, &fixed, sign, &stop);
                note(
                    "look-alike",
                    self.began,
                    format_args!(
                        "sign={sign} support={:?}",
                        motions.iter().map(|m| m.support).collect::<Vec<_>>()
                    ),
                );
                for motion in motions {
                    all.add(motion.pose, family, sign);
                }
                for pose in starts::principal(&self.moving.coarse, &self.fixed.coarse) {
                    all.add(pose, SeedOrigin::PrincipalFrame, sign);
                }
            }
        }
        out.examined = [
            SeedOrigin::Start,
            SeedOrigin::Landmarks,
            SeedOrigin::FeatureSame,
            SeedOrigin::FeatureOpposed,
            SeedOrigin::PrincipalFrame,
        ]
        .map(|family| (family, all.count(family)))
        .to_vec();
        all.0
    }

    /// Common area under a start's pose, both ways, each place counted less
    /// the farther it stands off.
    fn support<M: Target, F: Target>(
        start: &Start,
        moving: (&[Probe], &M),
        fixed: (&[Probe], &F),
        reach: f64,
        facing: f64,
    ) -> f64 {
        let meet = Meet {
            sign: start.sign,
            facing,
        };
        seat::support(moving.0, fixed.1, start.pose, reach, meet)
            + seat::support(fixed.0, moving.1, start.pose.inverse(), reach, meet)
    }

    fn coarse_support(&self, start: &Start, spacings: f64) -> f64 {
        Self::support(
            start,
            (&self.moving_coarse, &self.moving.coarse),
            (&self.fixed_coarse, &self.fixed.coarse),
            spacings * self.layout.coarse_mm,
            ROUGH_FACING,
        )
    }

    /// Seat one start and keep where and how it ended; the rounds it took.
    fn seat_on<T: Target>(
        &self,
        start: &mut Start,
        probes: &[Probe],
        target: &T,
        plan: (&[f64], u32, f64),
    ) -> u64 {
        let (reaches, rounds, facing) = plan;
        let seated = seat(
            probes,
            target,
            start.pose,
            &Schedule {
                meet: Meet {
                    sign: start.sign,
                    facing,
                },
                reaches,
                rounds,
                stop: &|| self.seating_ended(),
            },
        );
        start.pose = seated.pose;
        start.termination = seated.termination;
        u64::from(seated.rounds)
    }

    /// Coarse clouds: rank every motion, seat the best, rank again.
    fn seat_coarse(&self, starts: &mut Vec<Start>) -> u64 {
        let same = SAME_COARSE * self.layout.coarse_mm;
        for start in starts.iter_mut() {
            if self.ended().is_some() {
                break;
            }
            start.support = self.coarse_support(start, COARSE_REACH[0]);
        }
        shortlist(starts, &self.moving_coarse, same, COARSE_KEPT);
        let reaches = COARSE_REACH.map(|spacings| spacings * self.layout.coarse_mm);
        let mut rounds = 0;
        for start in starts.iter_mut() {
            rounds += self.seat_on(
                start,
                &self.moving_coarse,
                &self.fixed.coarse,
                (&reaches, COARSE_ROUNDS, ROUGH_FACING),
            );
            start.support = self.coarse_support(start, COARSE_REACH[2]);
        }
        shortlist(starts, &self.moving_coarse, same, FINE_KEPT);
        rounds
    }

    /// Fine clouds: seat, rank, and leave behind the rivals that are far
    /// behind the best. Returns the rounds taken and the support of the best
    /// rival left behind, which still bounds how far ahead the best pose is.
    fn seat_fine(&self, starts: &mut Vec<Start>) -> (u64, f64) {
        let reaches = FINE_REACH.map(|reach| reach.max(self.layout.fine_mm * 2.));
        let mut rounds = 0;
        for start in starts.iter_mut() {
            rounds += self.seat_on(
                start,
                &self.moving_seat,
                &self.fixed.fine,
                (&reaches, ROUNDS, evidence::FACING),
            );
            start.support = Self::support(
                start,
                (&self.moving_read, &self.moving.fine),
                (&self.fixed_read, &self.fixed.fine),
                evidence::COMMON_MM,
                evidence::FACING,
            );
        }
        let most = self.settings.top_k.clamp(1, 5);
        shortlist(starts, &self.moving_coarse, SAME_MM, most);
        starts.truncate(most);
        let best = starts.first().map_or(0., |start| start.support);
        let (mut behind, mut rank) = (0f64, 0usize);
        starts.retain(|start| {
            rank += 1;
            let keep = rank == 1
                || start.origin == SeedOrigin::Landmarks
                || start.support >= best * RIVAL_SHARE;
            if !keep {
                behind = behind.max(start.support);
            }
            keep
        });
        (rounds, behind)
    }

    /// The triangles themselves, when the fixed scan's were indexed.
    fn seat_exact(&self, starts: &mut [Start]) -> u64 {
        let Some(target) = self.fixed.exact() else {
            return 0;
        };
        let mut rounds = 0;
        for start in starts {
            rounds += self.seat_on(
                start,
                &self.moving_seat,
                &target,
                (&EXACT_REACH, ROUNDS, evidence::FACING),
            );
        }
        rounds
    }

    /// What a seated pose is worth, read on the triangles of each scan when
    /// they were indexed and on its fine cloud otherwise.
    fn evidence(&self, start: &Start) -> CandidateEvidence {
        let mut found = match (self.moving.exact(), self.fixed.exact()) {
            (Some(moving), Some(fixed)) => self.read(start, &moving, &fixed, EXACT_REACH[1]),
            (Some(moving), None) => self.read(start, &moving, &self.fixed.fine, self.cloud_reach()),
            (None, Some(fixed)) => self.read(start, &self.moving.fine, &fixed, EXACT_REACH[1]),
            (None, None) => self.read(
                start,
                &self.moving.fine,
                &self.fixed.fine,
                self.cloud_reach(),
            ),
        };
        found.original_surface_exact =
            [self.moving.exact().is_some(), self.fixed.exact().is_some()];
        found
    }

    /// The narrowest reach at which a fine cloud still answers evenly.
    fn cloud_reach(&self) -> f64 {
        FINE_REACH[1].max(self.layout.fine_mm * 2.)
    }

    fn read<M: Target, F: Target>(
        &self,
        start: &Start,
        moving: &M,
        fixed: &F,
        tightest: f64,
    ) -> CandidateEvidence {
        let meet = Meet {
            sign: start.sign,
            facing: evidence::FACING,
        };
        // A reading probe seated the pose when the seating took the same
        // point of the fine cloud.
        let read_stride = surface::stride(&self.moving.fine, READ_PROBES);
        let seat_stride = surface::stride(&self.moving.fine, SEAT_PROBES);
        let trained = move |ordinal: usize| (ordinal * read_stride).is_multiple_of(seat_stride);
        let forward = evidence::read(&self.moving_read, &trained, fixed, start.pose, meet);
        let backward = evidence::read(
            &self.fixed_read,
            &|_| false,
            moving,
            start.pose.inverse(),
            meet,
        );
        let mut found = evidence::join(&forward, &backward, [self.moving.area, self.fixed.area]);
        if let Some(trend) =
            evidence::size_trend(&self.moving_seat, fixed, start.pose, SIZE_REACH, meet)
        {
            found.size_trend = Metric::Measured(trend);
        }
        // A pose without enough tight common surface stays Weak whatever its
        // stability, so that is not measured for it.
        let drift = crate::confidence::Support::of(&found)
            .filter(crate::confidence::Support::sufficient)
            .and_then(|_| evidence::hold(&forward.shared))
            .and_then(|held| {
                evidence::left_out_drift(
                    &self.moving_seat,
                    fixed,
                    start.pose,
                    held.centre,
                    &Schedule {
                        meet,
                        reaches: &[tightest],
                        rounds: LEFT_OUT_ROUNDS,
                        stop: &|| self.seating_ended(),
                    },
                )
            });
        match drift {
            Some(drift) => {
                found.jackknife_mm_deg = Metric::Measured(drift);
                found.jackknife_complete = true;
            }
            None => found.jackknife_mm_deg = Metric::Missing(MissingReason::NoSupport),
        }
        found
    }

    /// Order the poses by score, fold a pose whose seating ran out of rounds
    /// beside a better pose into that pose, and tell each pose how far ahead
    /// of its best rival it is. `behind` is the support of the best rival
    /// that was left behind before the evidence was read.
    fn rank(&self, starts: &[Start], found: Vec<Found>, behind: f64) -> Vec<Found> {
        let stopped = self.ended().is_some();
        let scores: Vec<f64> = found
            .iter()
            .map(|found| match found.evidence.score {
                Metric::Measured(score) => score,
                Metric::Missing(_) => 0.,
            })
            .collect();
        let mut order: Vec<usize> = (0..found.len()).collect();
        order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
        let mut kept: Vec<usize> = Vec::new();
        for &index in &order {
            let again = starts[index].termination != RefinementTermination::StepSmall
                && kept.iter().any(|&better| {
                    starts[better].sign.to_bits() == starts[index].sign.to_bits()
                        && apart(
                            &starts[better].pose,
                            &starts[index].pose,
                            &self.moving_coarse,
                        ) < SAME_BASIN_MM
                });
            if !again {
                kept.push(index);
            }
        }
        let mut found: Vec<Option<Found>> = found.into_iter().map(Some).collect();
        let mut ranked = Vec::with_capacity(kept.len());
        for (rank, &index) in kept.iter().enumerate() {
            let Some(mut pose) = found[index].take() else {
                continue;
            };
            let rival = kept
                .iter()
                .enumerate()
                .find(|&(other, _)| other != rank)
                .map_or(0., |(_, &other)| scores[other])
                .max(behind);
            let evidence = &mut pose.evidence;
            if scores[index] > 0. {
                evidence.rival_gap =
                    Metric::Measured(((scores[index] - rival) / scores[index]).max(0.));
            }
            evidence.rival_probes_complete = !stopped;
            evidence.verification_complete =
                !stopped && evidence.holdout_complete && evidence.jackknife_complete;
            ranked.push(pose);
        }
        ranked
    }
}

/// Spacings of the fine and the coarse clouds for surfaces of these areas.
fn spacings(areas: [f64; 2]) -> [f64; 2] {
    let spacing = |area: f64, points: f64| (area * POINTS_PER_CELL / points).sqrt();
    let (smaller, larger) = (areas[0].min(areas[1]), areas[0].max(areas[1]));
    let fine = FINE_MM.max(spacing(larger, FINE_POINTS));
    let coarse = spacing(smaller, COARSE_POINTS)
        .clamp(COARSE_MM[0], COARSE_MM[1])
        .max(spacing(larger, COARSE_MOST))
        .max(fine * 1.5);
    [fine, coarse]
}

/// Search for the poses that bring `input.moving` onto `input.fixed`.
pub(crate) fn register(
    input: &AlignmentInput<'_>,
    settings: &SearchSettings,
    control: &SearchControl,
) -> Registration {
    let began = Instant::now();
    let regions = settings.reference_regions;
    let (moving_area, _) = surface::extent(input.moving, regions);
    let (fixed_area, origin) = surface::extent(input.fixed, regions);
    let mut out = Registration {
        areas: [moving_area, fixed_area],
        ..Registration::default()
    };
    if !(solve::positive(moving_area) && solve::positive(fixed_area)) {
        out.stopped = Some(Completion::NoUsableSurface);
        return out;
    }
    let [fine_mm, coarse_mm] = spacings(out.areas);
    let layout = Layout {
        origin,
        fine_mm,
        coarse_mm,
        regions,
    };
    let (moving, fixed) = rayon::join(
        || Side::read(input.moving, &layout, control.surface_control(settings)),
        || Side::read(input.fixed, &layout, control.surface_control(settings)),
    );
    out.omitted = [moving.omitted, fixed.omitted];
    out.areas = [moving.area, fixed.area];
    out.coherent = [moving.coherent(), fixed.coherent()];
    note(
        "surfaces",
        began,
        format_args!(
            "fine_mm={fine_mm:.3} coarse_mm={coarse_mm:.3} moving={}/{} fixed={}/{} coherent={:?}",
            moving.fine.points.len(),
            moving.coarse.points.len(),
            fixed.fine.points.len(),
            fixed.coarse.points.len(),
            out.coherent
        ),
    );
    let search = Search {
        settings,
        control,
        began,
        layout,
        moving_coarse: surface::probes(&moving.coarse, COARSE_PROBES),
        fixed_coarse: surface::probes(&fixed.coarse, COARSE_PROBES),
        moving_seat: surface::probes(&moving.fine, SEAT_PROBES),
        moving_read: surface::probes(&moving.fine, READ_PROBES),
        fixed_read: surface::probes(&fixed.fine, READ_PROBES),
        moving,
        fixed,
    };
    if let Some(done) = search.ended() {
        out.stopped = Some(done);
        return out;
    }
    let mut starts = search.starts(input, &mut out);
    out.rounds += search.seat_coarse(&mut starts);
    note("coarse", began, format_args!("kept={}", starts.len()));
    let (rounds, behind) = search.seat_fine(&mut starts);
    out.rounds += rounds;
    note("fine", began, format_args!("kept={}", starts.len()));
    out.rounds += search.seat_exact(&mut starts);
    note("exact", began, format_args!("rounds={}", out.rounds));
    let found: Vec<Found> = starts
        .iter()
        .map(|start| Found {
            pose: search.to_world(start.pose),
            origin: start.origin,
            proposal: start.proposal,
            evidence: search.evidence(start),
            termination: start.termination,
        })
        .collect();
    out.found = search.rank(&starts, found, behind);
    out.stopped = search.ended();
    note("evidence", began, format_args!("found={}", out.found.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_spacings_follow_the_areas() {
        // An arch against an arch: the finest spacings.
        assert_eq!(spacings([4_000., 5_000.]), [0.3, 1.]);
        // A small fragment against an arch: a finer coarse cloud, so the
        // fragment still has enough points to describe.
        let [fine, coarse] = spacings([400., 5_000.]);
        assert!((fine - 0.3).abs() < 1e-12);
        assert!((0.5..0.95).contains(&coarse), "{coarse}");
        // A very large surface: wider spacings, so the clouds stay bounded.
        let [fine, coarse] = spacings([200_000., 200_000.]);
        assert!(fine > 0.9 && coarse > fine, "{fine} {coarse}");
    }
}
