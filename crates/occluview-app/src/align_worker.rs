//! Background execution for Align Scans.
//!
//! Every heavy call lives here. A full arch is hundreds of thousands of
//! triangles; building the surface index, refining, and measuring on the UI
//! thread would freeze the window for seconds — the bug we already fixed once
//! in Bridge Split.
//!
//! Two guards make late results safe. A *generation* stamps each job, and a
//! completion whose generation is behind the worker's current one is dropped:
//! the pair, pose, or settings it was computed for no longer exist. A *kind*
//! lets a newer job of the same kind replace a queued one, so dragging a
//! slider does not queue a hundred measurements.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use glam::DVec3;
use occluview_align::suggested_scale_mm;
use occluview_align::{
    deviation, deviation_stats, fit_pairs, observability, ramp_color, refine, CancelFlag,
    DeviationMap, DeviationSettings, DeviationStats, FitRejection, IcpReport, Observability,
    Orientation, RampMode, RampSettings, RefineSettings, Rigid, Soup, SurfaceIndex, Validity,
    NO_DATA_COLOR,
};
use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

/// Operator-facing knobs, in the operator's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignSettings {
    /// Farthest a moving vertex looks for fixed surface, in millimetres.
    pub(crate) influence_radius_mm: f64,
    /// Fraction of correspondences kept after trimming.
    pub(crate) matching_ratio: f64,
    /// How the two surfaces are taken to face each other.
    pub(crate) orientation: Orientation,
    /// Deviation mapped to the ends of the colour ramp, in millimetres.
    pub(crate) scale_mm: f64,
    /// Tolerance band the statistics report, in millimetres.
    pub(crate) tolerance_mm: f64,
    /// Steps per side for a banded ramp; `None` is continuous.
    pub(crate) bands: Option<u32>,
    /// Which colour scheme the map paints with.
    pub(crate) ramp_mode: RampMode,
    /// Whether the display scale follows the measurement instead of a guess.
    ///
    /// Cleared the moment the operator moves the slider: once they have chosen
    /// a range, the tool must not keep overriding it.
    pub(crate) auto_scale: bool,
    /// Whether the map is on screen.
    pub(crate) show_deviation: bool,
}

impl Default for AlignSettings {
    fn default() -> Self {
        Self {
            // Far enough that a roughly-placed scan still has something to
            // measure against. Too tight and the map comes out mostly grey,
            // which reads as "broken" rather than "out of reach".
            influence_radius_mm: 5.0,
            matching_ratio: 0.8,
            orientation: Orientation::Match,
            // Real registration deviations sit at 0.05-0.3 mm. A wider scale
            // buries them in the green centre, which is exactly why the web
            // build's map reads washed out.
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            ramp_mode: RampMode::default(),
            auto_scale: true,
            show_deviation: true,
        }
    }
}

impl AlignSettings {
    fn refine(self) -> RefineSettings {
        RefineSettings {
            influence_radius_mm: self.influence_radius_mm,
            matching_ratio: self.matching_ratio,
            orientation: self.orientation,
            ..RefineSettings::default()
        }
    }

    fn deviation(self) -> DeviationSettings {
        DeviationSettings {
            influence_radius_mm: self.influence_radius_mm,
            orientation: self.orientation,
        }
    }

    fn ramp(self) -> RampSettings {
        RampSettings {
            scale_mm: self.scale_mm,
            tolerance_mm: self.tolerance_mm,
            bands: self.bands,
            mode: self.ramp_mode,
        }
    }
}

/// One correspondence, already in the frame each stage wants: the moving point
/// in its layer's local coordinates, the fixed point in world.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldPair {
    /// Moving point in the moving layer's local frame.
    pub(crate) moving: DVec3,
    /// Moving surface normal, same frame.
    pub(crate) moving_normal: DVec3,
    /// Fixed point in world.
    pub(crate) fixed: DVec3,
    /// Fixed surface normal in world.
    pub(crate) fixed_normal: DVec3,
}

/// Identity of everything a measurement's DISTANCES depend on.
///
/// Two jobs with equal keys measure the same two surfaces in the same relative
/// pose through the same mask and reach, so they cannot produce different
/// distances — and the second may reuse the first's map instead of spending
/// half a second re-deriving it. Everything the ramp reads (the display scale,
/// the band count, the colour scheme) is deliberately absent: those change the
/// COLOUR of a measurement, never the measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MeasureKey {
    /// Geometry and pose of the layer being measured.
    pub(crate) moving: (u64, u64),
    /// Geometry and pose of the surface it is measured against.
    pub(crate) fixed: (u64, u64),
    /// Which revision of the exclusion mask was in force.
    pub(crate) mask: u64,
    /// The reach, in raw bits so the key compares exactly.
    pub(crate) influence_radius_bits: u64,
    /// How the two surfaces are taken to face each other.
    pub(crate) orientation: Orientation,
}

/// What a job asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignJobKind {
    /// Fit the clicked pairs.
    Align,
    /// Seat the surfaces with ICP.
    Refine,
    /// Measure the deviation map.
    Measure,
}

/// Everything one job needs. Geometry is borrowed through `Arc`, so submitting
/// a job never copies a mesh.
pub(crate) struct AlignJob {
    /// The generation this job belongs to.
    pub(crate) generation: u64,
    /// What to compute.
    pub(crate) kind: AlignJobKind,
    /// Moving layer geometry, in its own local frame.
    pub(crate) moving_positions: Arc<Vec<f32>>,
    /// Moving layer triangles.
    pub(crate) moving_indices: Arc<Vec<u32>>,
    /// Fixed layer geometry, already posed into world.
    pub(crate) fixed_world_positions: Arc<Vec<f32>>,
    /// Fixed layer triangles.
    pub(crate) fixed_indices: Arc<Vec<u32>>,
    /// Identity of the fixed geometry, so its index can be reused.
    pub(crate) fixed_key: (u64, u64),
    /// Identity of the measurement, so a colour-only change reuses its map.
    pub(crate) measure_key: MeasureKey,
    /// The moving layer's current pose: local to world.
    pub(crate) pose: Rigid,
    /// Clicked pairs, for an `Align` job.
    pub(crate) pairs: Vec<WorldPair>,
    /// Per-vertex exclusion mask over the moving layer.
    pub(crate) mask: Option<Arc<Vec<u8>>>,
    /// The settings in force.
    pub(crate) settings: AlignSettings,
}

/// What a finished job produced.
pub(crate) enum AlignOutcome {
    /// A fit landed. The pose maps the moving layer's local frame to world.
    Aligned {
        /// The new layer pose.
        pose: Rigid,
        /// Root-mean-square pair residual, in millimetres.
        rms: f64,
        /// Pairs dropped as outliers.
        rejected: Vec<u32>,
    },
    /// A refine landed.
    Refined {
        /// The new layer pose.
        pose: Rigid,
        /// Diagnostics the panel reports.
        report: Box<IcpReport>,
    },
    /// A measurement landed.
    Measured {
        /// One colour per measured vertex.
        colors: Vec<[u8; 4]>,
        /// Summary over the measured vertices. One-sided, and a lower bound on
        /// displacement — never report it without `seen`.
        stats: DeviationStats,
        /// How much of a rigid displacement this pair converts into measured
        /// distance. `None` when the geometry does not determine it.
        seen: Option<Observability>,
        /// The display scale the colours were painted at. With auto-scale on
        /// this is derived from the measurement itself, so the panel has to
        /// adopt it or its legend would describe a different range.
        scale_mm: f64,
    },
    /// Nothing trustworthy came out, and this is why.
    Failed {
        /// A sentence naming what went wrong, not "alignment failed".
        message: String,
    },
}

/// A finished job.
pub(crate) struct AlignCompletion {
    /// The generation the job belonged to.
    pub(crate) generation: u64,
    /// The result. Which job produced it is already implied by the variant.
    pub(crate) outcome: AlignOutcome,
}

struct QueueState {
    jobs: VecDeque<AlignJob>,
    shutdown: bool,
}

struct JobQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}

/// The worker handle the app holds.
pub(crate) struct AlignWorker {
    queue: Arc<JobQueue>,
    completions: Arc<Mutex<Vec<AlignCompletion>>>,
    running: Arc<Mutex<Option<CancelFlag>>>,
    generation: Arc<AtomicU64>,
    busy: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl AlignWorker {
    /// Start the worker thread.
    pub(crate) fn spawn() -> Self {
        let queue = Arc::new(JobQueue {
            state: Mutex::new(QueueState {
                jobs: VecDeque::new(),
                shutdown: false,
            }),
            wake: Condvar::new(),
        });
        let completions = Arc::new(Mutex::new(Vec::new()));
        let running: Arc<Mutex<Option<CancelFlag>>> = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));
        let busy = Arc::new(AtomicU64::new(0));

        let thread_queue = Arc::clone(&queue);
        let thread_completions = Arc::clone(&completions);
        let thread_running = Arc::clone(&running);
        let thread_busy = Arc::clone(&busy);
        let handle = thread::Builder::new()
            .name("occluview-align".into())
            .spawn(move || {
                run_worker(
                    &thread_queue,
                    &thread_completions,
                    &thread_running,
                    &thread_busy,
                );
            })
            .ok();

        Self {
            queue,
            completions,
            running,
            generation,
            busy,
            handle,
        }
    }

    /// Move to a new generation, so every result still in flight is discarded.
    pub(crate) fn bump_generation(&self) -> u64 {
        self.cancel_running();
        let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut state) = self.queue.state.lock() {
            state.jobs.clear();
        }
        if let Ok(mut completions) = self.completions.lock() {
            completions.clear();
        }
        next
    }

    /// The generation new jobs should carry.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Whether anything is queued or running.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0
            || self
                .queue
                .state
                .lock()
                .is_ok_and(|state| !state.jobs.is_empty())
    }

    /// Queue a job, replacing any queued job of the same kind and cancelling a
    /// running one. Dragging a slider must not queue a hundred measurements.
    pub(crate) fn submit(&self, job: AlignJob) {
        self.cancel_running();
        let Ok(mut state) = self.queue.state.lock() else {
            return;
        };
        state.jobs.retain(|queued| queued.kind != job.kind);
        state.jobs.push_back(job);
        drop(state);
        self.queue.wake.notify_one();
    }

    /// Take every completion that still belongs to the current generation.
    pub(crate) fn drain(&self) -> Vec<AlignCompletion> {
        let current = self.generation();
        let Ok(mut completions) = self.completions.lock() else {
            return Vec::new();
        };
        let drained: Vec<AlignCompletion> = completions.drain(..).collect();
        drained
            .into_iter()
            .filter(|completion| completion.generation == current)
            .collect()
    }

    /// Ask a running job to stop.
    pub(crate) fn cancel_running(&self) {
        if let Ok(running) = self.running.lock() {
            if let Some(flag) = running.as_ref() {
                flag.cancel();
            }
        }
    }
}

impl Drop for AlignWorker {
    fn drop(&mut self) {
        self.cancel_running();
        if let Ok(mut state) = self.queue.state.lock() {
            state.shutdown = true;
            state.jobs.clear();
        }
        self.queue.wake.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The worker loop: take a job, run it, publish what came out.
fn run_worker(
    queue: &Arc<JobQueue>,
    completions: &Arc<Mutex<Vec<AlignCompletion>>>,
    running: &Arc<Mutex<Option<CancelFlag>>>,
    busy: &Arc<AtomicU64>,
) {
    let mut cached = WorkerCache::default();
    loop {
        let job = {
            let Ok(mut state) = queue.state.lock() else {
                return;
            };
            while state.jobs.is_empty() && !state.shutdown {
                let Ok(next) = queue.wake.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.shutdown {
                return;
            }
            match state.jobs.pop_front() {
                Some(job) => job,
                None => continue,
            }
        };

        let cancel = CancelFlag::new();
        if let Ok(mut slot) = running.lock() {
            *slot = Some(cancel.clone());
        }
        busy.fetch_add(1, Ordering::SeqCst);

        let outcome = execute(&job, &cancel, &mut cached);
        // A cancelled stage returns a well-formed but meaningless value — the
        // start pose, or a map where nothing was measured. Publishing that
        // reverts the operator's drag or flashes a fully grey scan.
        let abandoned = cancel.is_cancelled();

        busy.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut slot) = running.lock() {
            *slot = None;
        }
        if abandoned {
            continue;
        }
        if let Ok(mut published) = completions.lock() {
            published.push(AlignCompletion {
                generation: job.generation,
                outcome,
            });
        }
    }
}

/// What the worker keeps between jobs.
///
/// Everything here is derived from geometry the operator has not changed. The
/// alternative — starting from the mesh on every settings change — is what made
/// nudging the display scale cost most of a second on a full arch.
#[derive(Default)]
struct WorkerCache {
    /// The fixed surface's spatial index, and the geometry it was built for.
    surface: Option<((u64, u64), SurfaceIndex)>,
    /// The last deviation map, and the measurement it belongs to.
    measured: Option<(MeasureKey, DeviationMap)>,
    /// The last summary, and the measurement and tolerance it was taken at.
    summary: Option<(MeasureKey, u64, DeviationStats)>,
    /// What that measurement was capable of seeing. Independent of the ramp and
    /// the tolerance, so it survives a re-colour exactly as the map does.
    seen: Option<(MeasureKey, Option<Observability>)>,
}

/// Run one job.
fn execute(job: &AlignJob, cancel: &CancelFlag, cached: &mut WorkerCache) -> AlignOutcome {
    let moving = Soup {
        positions: &job.moving_positions,
        indices: &job.moving_indices,
        mask: job.mask.as_ref().map(|mask| mask.as_slice()),
    };

    if job.kind == AlignJobKind::Align {
        return align_from_pairs(job, moving);
    }

    // A re-colour of a measurement already in hand never touches the surface:
    // the distances did not change, only what they are painted with.
    if job.kind == AlignJobKind::Measure
        && cached
            .measured
            .as_ref()
            .is_some_and(|(key, _)| *key == job.measure_key)
    {
        return recolor(job, cached);
    }

    let Some(index) = surface_index(&mut cached.surface, job) else {
        return AlignOutcome::Failed {
            message: "The fixed scan has no usable surface".into(),
        };
    };

    match job.kind {
        // Handled above, before the surface index is touched.
        AlignJobKind::Align => AlignOutcome::Failed {
            message: "Internal routing error".into(),
        },
        AlignJobKind::Refine => {
            match refine(moving, index, job.pose, &job.settings.refine(), cancel) {
                Ok(report) => AlignOutcome::Refined {
                    pose: report.rigid,
                    report: Box::new(report),
                },
                Err(rejection) => AlignOutcome::Failed {
                    message: describe(rejection),
                },
            }
        }
        AlignJobKind::Measure => {
            let map = deviation(moving, index, job.pose, &job.settings.deviation(), cancel);
            // A cancelled measurement is a map where nothing was measured. It
            // must not be remembered as this key's answer, or the abandoned
            // result would be handed to every later re-colour.
            // Without this the operator reads a nearest-point distance as
            // though it were a displacement — on a real arch, an understatement
            // by about a factor of two.
            let seen = observability(moving, index, job.pose, &job.settings.deviation(), cancel);
            if !cancel.is_cancelled() {
                cached.measured = Some((job.measure_key, map));
                cached.summary = None;
                cached.seen = Some((job.measure_key, seen));
                return recolor(job, cached);
            }
            let stats = deviation_stats(&map, job.settings.tolerance_mm);
            paint(&map, job, stats, seen)
        }
    }
}

/// Colour the map already in hand, taking the summary from the cache when the
/// tolerance has not moved either.
fn recolor(job: &AlignJob, cached: &mut WorkerCache) -> AlignOutcome {
    let Some((_, map)) = cached.measured.as_ref() else {
        return AlignOutcome::Failed {
            message: "The measurement was dropped before it could be coloured".into(),
        };
    };
    let tolerance = job.settings.tolerance_mm.to_bits();
    let stats = match cached.summary {
        Some((key, at, stats)) if key == job.measure_key && at == tolerance => stats,
        _ => {
            let stats = deviation_stats(map, job.settings.tolerance_mm);
            cached.summary = Some((job.measure_key, tolerance, stats));
            stats
        }
    };
    let seen = cached
        .seen
        .and_then(|(key, seen)| (key == job.measure_key).then_some(seen))
        .flatten();
    paint(map, job, stats, seen)
}

/// How far past the nominal band an automatic range must reach.
const BAND_HEADROOM: f64 = 2.5;

/// Turn a map and its summary into the colours the panel will show.
fn paint(
    map: &DeviationMap,
    job: &AlignJob,
    stats: DeviationStats,
    seen: Option<Observability>,
) -> AlignOutcome {
    // A fixed range is a guess about data nobody has measured yet: too wide and
    // a good fit is one flat colour, too narrow and everything saturates.
    // Fitting the range to the measurement is what makes the map show structure
    // on the first press instead of the tenth.
    let mut ramp = job.settings.ramp();
    if job.settings.auto_scale {
        // Never inside the nominal band, and never so close to it that the ramp
        // has nowhere to run: everything within tolerance is painted one
        // colour, so a range that only just clears the band leaves a map with
        // two colours in it and no gradient to read.
        ramp.scale_mm = suggested_scale_mm(&stats).max(job.settings.tolerance_mm * BAND_HEADROOM);
    }
    AlignOutcome::Measured {
        colors: color_map(map, &ramp),
        stats,
        seen,
        scale_mm: ramp.scale_mm,
    }
}

/// One RGBA per map entry, grey wherever there is no measurement.
///
/// [`occluview_align::deviation_colors`] does exactly this, serially. On a
/// 945k-vertex arch that is twenty-odd milliseconds of a re-colour that should
/// feel instant, so this walks the same ramp in parallel;
/// `colouring_in_parallel_matches_the_library` pins the two to the same bytes.
fn color_map(map: &DeviationMap, ramp: &RampSettings) -> Vec<[u8; 4]> {
    map.signed_mm
        .par_iter()
        .zip(map.validity.par_iter())
        .map(|(value, state)| {
            if *state == Validity::Measured {
                ramp_color(f64::from(*value), ramp)
            } else {
                NO_DATA_COLOR
            }
        })
        .collect()
}

/// The fixed surface's index, built once and then reused while that surface
/// stays where it is.
fn surface_index<'a>(
    cached: &'a mut Option<((u64, u64), SurfaceIndex)>,
    job: &AlignJob,
) -> Option<&'a SurfaceIndex> {
    if cached.as_ref().is_none_or(|(key, _)| *key != job.fixed_key) {
        let built = SurfaceIndex::build(Soup {
            positions: &job.fixed_world_positions,
            indices: &job.fixed_indices,
            mask: None,
        })?;
        *cached = Some((job.fixed_key, built));
    }
    cached.as_ref().map(|(_, index)| index)
}

/// Fit the clicked pairs. The moving points are in the moving layer's local
/// frame and the fixed points are in world, so the result *is* the new layer
/// pose — no composition, and no chance of composing it the wrong way round.
fn align_from_pairs(job: &AlignJob, moving: Soup<'_>) -> AlignOutcome {
    let moving_points: Vec<DVec3> = job.pairs.iter().map(|pair| pair.moving).collect();
    let fixed_points: Vec<DVec3> = job.pairs.iter().map(|pair| pair.fixed).collect();
    let moving_normals: Vec<DVec3> = job.pairs.iter().map(|pair| pair.moving_normal).collect();
    let fixed_normals: Vec<DVec3> = job.pairs.iter().map(|pair| pair.fixed_normal).collect();
    let extent = soup_extent(moving);

    match fit_pairs(
        &moving_points,
        &fixed_points,
        Some((&moving_normals, &fixed_normals)),
        extent,
    ) {
        Ok(fit) => AlignOutcome::Aligned {
            pose: fit.rigid,
            rms: fit.pair_rms,
            rejected: fit.rejected,
        },
        Err(rejection) => AlignOutcome::Failed {
            message: describe(rejection),
        },
    }
}

/// Bounding-box diagonal of a soup, used as the runaway guard's budget.
fn soup_extent(soup: Soup<'_>) -> f64 {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for chunk in soup.positions.chunks_exact(3) {
        let point = DVec3::new(
            f64::from(chunk[0]),
            f64::from(chunk[1]),
            f64::from(chunk[2]),
        );
        if point.is_finite() {
            min = min.min(point);
            max = max.max(point);
        }
    }
    if min.x.is_finite() && max.x.is_finite() {
        (max - min).length()
    } else {
        0.0
    }
}

/// Turn a refusal into a sentence the operator can act on.
///
/// Never "alignment failed": every refusal knows something specific, and
/// saying it is the difference between a tool that helps and one that shrugs.
fn describe(rejection: FitRejection) -> String {
    match rejection {
        FitRejection::TooFewPairs { have, need } => {
            format!("{have} of {need} point pairs — place another pair")
        }
        FitRejection::Unpaired { moving, fixed } => {
            format!("{moving} points on one scan and {fixed} on the other — a point has no partner")
        }
        FitRejection::Degenerate { weak_axes } => {
            let named = axis_names(weak_axes);
            if named.is_empty() {
                "The clicked points do not determine a rotation — spread them out".into()
            } else {
                format!("The clicked points lie on a line: rotation about {named} is undetermined")
            }
        }
        FitRejection::UnitMismatch { ratio } => format!(
            "The two scans are {ratio:.1}x apart in size — they are probably in different units"
        ),
        FitRejection::Runaway { moved_by, allowed } => format!(
            "That fit would move the scan {moved_by:.0} mm, further than its own size ({allowed:.0} mm) — check the pairs"
        ),
        FitRejection::NonFinite => "A clicked point or surface normal was not a finite number".into(),
    }
}

/// Name the world axes a degeneracy report flagged.
fn axis_names(weak: [bool; 3]) -> String {
    ["X", "Y", "Z"]
        .into_iter()
        .zip(weak)
        .filter_map(|(name, flagged)| flagged.then_some(name))
        .collect::<Vec<_>>()
        .join(", ")
}

// Split out to hold the workspace's 800-line file budget. A `#[path]` child
// module so the tests still reach this file's private items.
#[cfg(test)]
#[path = "align_worker_tests.rs"]
mod tests;
