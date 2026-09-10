//! Background execution for Align Scans.
//!
//! Heavy alignment and deviation work runs off the UI thread.
//!
//! Each job carries a generation; completions from older generations are
//! discarded. A job kind allows a queued job to be replaced by a newer request
//! of the same kind.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use glam::DVec3;
use occluview_align::suggested_scale_mm;
use occluview_align::{
    deviation, deviation_stats, fit_pairs, observability, ramp_color, refine, CancelFlag,
    DeviationMap, DeviationSettings, DeviationStats, FitBounds, FitRejection, Observability,
    Orientation, RampMode, RampSettings, RefineSettings, Rigid, Soup, SurfaceIndex, Validity,
    NO_DATA_COLOR,
};
use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

/// Initial display maximum, in millimetres.
pub(crate) const WORKING_MAX_MM: f64 = 0.10;
/// Absolute zero of the operator-controlled deviation display range.
pub(crate) const WORKING_SCALE_MIN_MM: f64 = 0.0;
/// Initial nominal tolerance band, in millimetres.
pub(crate) const WORKING_MIN_MM: f64 = 0.01;

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
    /// Whether the display scale follows the measurement.
    pub(crate) auto_scale: bool,
    /// Whether the map is on screen.
    pub(crate) show_deviation: bool,
}

impl Default for AlignSettings {
    fn default() -> Self {
        Self {
            // Allow a roughly placed mesh to find the other surface.
            influence_radius_mm: 2.0,
            matching_ratio: 0.8,
            orientation: Orientation::Match,
            // Start at the tightest standard range; manual changes remain
            // stable until the operator selects another range.
            scale_mm: WORKING_MAX_MM,
            tolerance_mm: WORKING_MIN_MM,
            bands: None,
            // Magnitude is the default display mode; signed values are an
            // optional diagnostic.
            ramp_mode: RampMode::Magnitude,
            // Keep the clinical display range fixed unless requested.
            auto_scale: false,
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
            scale_mm: self.scale_mm.clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM),
            tolerance_mm: self.tolerance_mm,
            // The operator-facing Align Meshes map is one continuous absolute
            // scale. Keep the field only for loading old state; never let that
            // legacy value quantize a production measurement.
            bands: None,
            mode: self.ramp_mode,
        }
    }
}

/// Whether a settings edit changes the optimizer's interpretation of a fit.
/// Display range and visibility are deliberately excluded: they can recolour
/// an already landed measurement, while these three inputs require a new Best
/// fit result before the heatmap may describe the session again.
pub(crate) fn matching_inputs_changed(before: AlignSettings, after: AlignSettings) -> bool {
    before.matching_ratio.to_bits() != after.matching_ratio.to_bits()
        || before.influence_radius_mm.to_bits() != after.influence_radius_mm.to_bits()
        || before.orientation != after.orientation
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

/// Inputs that determine a deviation map. Display-only settings are excluded
/// because they affect colours, not measured distances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MeasureKey {
    /// Geometry and pose of the layer being measured.
    pub(crate) moving: (u64, u64),
    /// Geometry, pose and markings of the surface it is measured against.
    pub(crate) fixed: SurfaceKey,
    /// Which revision of the exclusion mask was in force.
    pub(crate) mask: u64,
    /// The reach, in raw bits so the key compares exactly.
    pub(crate) influence_radius_bits: u64,
    /// How the two surfaces are taken to face each other.
    pub(crate) orientation: Orientation,
}

/// Identity of a built surface index: what it was built from, posed where, with
/// which markings taken out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SurfaceKey {
    /// The geometry the triangles came from.
    pub(crate) geometry: u64,
    /// The pose they were baked into world at.
    pub(crate) pose: u64,
    /// Which revision of the fixed markings was left out of it.
    pub(crate) markings: u64,
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
    /// Identity of the fixed surface, so its index can be reused.
    pub(crate) fixed_key: SurfaceKey,
    /// Identity of the measurement, so a colour-only change reuses its map.
    pub(crate) measure_key: MeasureKey,
    /// The moving layer's current pose: local to world.
    pub(crate) pose: Rigid,
    /// Clicked pairs, for an `Align` job.
    pub(crate) pairs: Vec<WorldPair>,
    /// Per-vertex exclusion mask over the moving layer.
    pub(crate) mask: Option<Arc<Vec<u8>>>,
    /// Per-vertex exclusion mask over the fixed layer. Masked triangles are
    /// left out of the surface index entirely, so nothing can match against
    /// them or measure to them.
    pub(crate) fixed_mask: Option<Arc<Vec<u8>>>,
    /// The settings in force.
    pub(crate) settings: AlignSettings,
}

/// Why a job produced nothing trustworthy. Domain data only: the worker never
/// renders user-facing copy; the presentation boundary renders it from the
/// typed reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AlignFailure {
    /// A fit or refine refused, carrying the refusal to report.
    Fit(FitRejection),
    /// The fixed scan has no usable surface.
    FixedSurfaceMissing,
    /// The moving scan has no usable surface.
    MovingSurfaceMissing,
    /// The cached measurement was dropped before it could be coloured.
    MeasurementDropped,
}

/// What a finished job produced.
pub(crate) enum AlignOutcome {
    /// A fit landed. The pose maps the moving layer's local frame to world.
    Aligned {
        /// The new layer pose.
        pose: Rigid,
        /// Pairs dropped as outliers.
        rejected: Vec<u32>,
    },
    /// A refine landed.
    Refined {
        /// The new layer pose.
        pose: Rigid,
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
        /// The typed reason; the panel renders the sentence.
        rejection: AlignFailure,
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
    failed: Arc<AtomicBool>,
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
        let failed = Arc::new(AtomicBool::new(false));

        let thread_queue = Arc::clone(&queue);
        let thread_completions = Arc::clone(&completions);
        let thread_running = Arc::clone(&running);
        let thread_busy = Arc::clone(&busy);
        let thread_failed = Arc::clone(&failed);
        let handle = thread::Builder::new()
            .name("occluview-align".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker(
                        &thread_queue,
                        &thread_completions,
                        &thread_running,
                        &thread_busy,
                        &thread_failed,
                    );
                }));
                if let Err(payload) = result {
                    mark_failed(
                        &thread_failed,
                        "align worker panicked",
                        Some(panic_message(payload)),
                    );
                }
            })
            .map_err(|error| {
                mark_failed(&failed, "thread spawn failed", Some(error.to_string()));
            })
            .ok();

        Self {
            queue,
            completions,
            running,
            generation,
            busy,
            failed,
            handle,
        }
    }

    /// Whether this worker can still accept or publish work.
    pub(crate) fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Move to a new generation, so every result still in flight is discarded.
    pub(crate) fn bump_generation(&self) -> u64 {
        self.cancel_running();
        let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        match self.queue.state.lock() {
            Ok(mut state) => state.jobs.clear(),
            Err(_) => mark_failed(&self.failed, "queue lock poisoned", None),
        }
        match self.completions.lock() {
            Ok(mut completions) => completions.clear(),
            Err(_) => mark_failed(&self.failed, "completion lock poisoned", None),
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

    /// Queue a job, replacing queued work of the same kind and cancelling the
    /// running job.
    pub(crate) fn submit(&self, job: AlignJob) -> bool {
        if self.has_failed() {
            return false;
        }
        self.cancel_running();
        let Ok(mut state) = self.queue.state.lock() else {
            mark_failed(&self.failed, "queue lock poisoned", None);
            return false;
        };
        state.jobs.retain(|queued| queued.kind != job.kind);
        state.jobs.push_back(job);
        drop(state);
        self.queue.wake.notify_one();
        true
    }

    /// Take every completion that still belongs to the current generation.
    pub(crate) fn drain(&self) -> Vec<AlignCompletion> {
        let current = self.generation();
        let Ok(mut completions) = self.completions.lock() else {
            mark_failed(&self.failed, "completion lock poisoned", None);
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
        match self.running.lock() {
            Ok(running) => {
                if let Some(flag) = running.as_ref() {
                    flag.cancel();
                }
            }
            Err(_) => mark_failed(&self.failed, "running-job lock poisoned", None),
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
    failed: &Arc<AtomicBool>,
) {
    let mut cached = WorkerCache::default();
    loop {
        let job = {
            let Ok(mut state) = queue.state.lock() else {
                mark_failed(failed, "queue lock poisoned", None);
                return;
            };
            while state.jobs.is_empty() && !state.shutdown {
                let Ok(next) = queue.wake.wait(state) else {
                    mark_failed(failed, "queue wait poisoned", None);
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
        let Ok(mut slot) = running.lock() else {
            mark_failed(failed, "running-job lock poisoned", None);
            return;
        };
        *slot = Some(cancel.clone());
        drop(slot);
        busy.fetch_add(1, Ordering::SeqCst);

        let outcome = execute(&job, &cancel, &mut cached);
        // Cancelled stages may return structurally valid but unusable values;
        // do not publish them.
        let abandoned = cancel.is_cancelled();

        busy.fetch_sub(1, Ordering::SeqCst);
        let Ok(mut slot) = running.lock() else {
            mark_failed(failed, "running-job lock poisoned", None);
            return;
        };
        *slot = None;
        drop(slot);
        if abandoned {
            continue;
        }
        let Ok(mut published) = completions.lock() else {
            mark_failed(failed, "completion lock poisoned", None);
            return;
        };
        published.push(AlignCompletion {
            generation: job.generation,
            outcome,
        });
    }
}

/// Record a terminal worker failure once and keep the UI-side state machine
/// fail-closed. Details go to the diagnostic log; the panel receives only a
/// stable localized status instead of an OS/thread error sentence.
fn mark_failed(failed: &AtomicBool, reason: &'static str, detail: Option<String>) {
    if !failed.swap(true, Ordering::AcqRel) {
        if let Some(detail) = detail {
            tracing::error!(reason, detail = %detail, "align worker stopped");
        } else {
            tracing::error!(reason, "align worker stopped");
        }
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

/// What the worker keeps between jobs.
///
/// Cached geometry-derived results reused across settings changes.
#[derive(Default)]
struct WorkerCache {
    /// The fixed surface's spatial index, and the surface it was built for.
    surface: Option<(SurfaceKey, SurfaceIndex)>,
    /// The last deviation map, and the measurement it belongs to.
    measured: Option<(MeasureKey, DeviationMap)>,
    /// The last summary, and the measurement and tolerance it was taken at.
    summary: Option<(MeasureKey, u64, DeviationStats)>,
    /// What that measurement was capable of seeing. Independent of the ramp and
    /// the tolerance, so it survives a re-colour exactly as the map does.
    seen: Option<(MeasureKey, Option<Observability>)>,
}

/// Jobs that require the fixed surface index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SurfaceJob {
    /// Seat the surfaces with ICP.
    Refine,
    /// Measure the deviation map.
    Measure,
}

/// Run one job.
fn execute(job: &AlignJob, cancel: &CancelFlag, cached: &mut WorkerCache) -> AlignOutcome {
    let moving = Soup {
        positions: &job.moving_positions,
        indices: &job.moving_indices,
        mask: job.mask.as_ref().map(|mask| mask.as_slice()),
    };

    let surface_job = match job.kind {
        AlignJobKind::Align => return align_from_pairs(job, moving),
        AlignJobKind::Refine => SurfaceJob::Refine,
        AlignJobKind::Measure => SurfaceJob::Measure,
    };

    // Re-colouring changes only the display, so reuse the cached map.
    if surface_job == SurfaceJob::Measure
        && cached
            .measured
            .as_ref()
            .is_some_and(|(key, _)| *key == job.measure_key)
    {
        return recolor(job, cached);
    }

    let Some(index) = surface_index(&mut cached.surface, job) else {
        return AlignOutcome::Failed {
            rejection: AlignFailure::FixedSurfaceMissing,
        };
    };

    match surface_job {
        SurfaceJob::Refine => {
            match refine(moving, index, job.pose, &job.settings.refine(), cancel) {
                Ok(report) => AlignOutcome::Refined { pose: report.rigid },
                Err(rejection) => AlignOutcome::Failed {
                    rejection: AlignFailure::Fit(rejection),
                },
            }
        }
        SurfaceJob::Measure => {
            let map = deviation(moving, index, job.pose, &job.settings.deviation(), cancel);
            // Do not cache a cancelled map or expose it to later re-colouring.
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
            rejection: AlignFailure::MeasurementDropped,
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
    // Automatic scaling exposes measured structure while keeping the selected
    // working range bounded; tolerance remains a statistics threshold only.
    let mut ramp = job.settings.ramp();
    if job.settings.auto_scale {
        // Leave room above the nominal band for a readable gradient.
        ramp.scale_mm = suggested_scale_mm(&stats)
            .max(job.settings.tolerance_mm * BAND_HEADROOM)
            .clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM);
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
/// Uses the same ramp as [`occluview_align::deviation_colors`] while preserving
/// its no-data colour.
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
    cached: &'a mut Option<(SurfaceKey, SurfaceIndex)>,
    job: &AlignJob,
) -> Option<&'a SurfaceIndex> {
    if cached.as_ref().is_none_or(|(key, _)| *key != job.fixed_key) {
        let built = SurfaceIndex::build(Soup {
            positions: &job.fixed_world_positions,
            indices: &job.fixed_indices,
            mask: job.fixed_mask.as_ref().map(|mask| mask.as_slice()),
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
    // Bounds are measured in the frames used by the corresponding point sets.
    let fixed_soup = Soup {
        positions: &job.fixed_world_positions,
        indices: &job.fixed_indices,
        mask: job.fixed_mask.as_ref().map(|mask| mask.as_slice()),
    };
    // Missing bounds are reported instead of inventing an overlap allowance.
    let Some((moving_center, moving_extent)) = occluview_align::bounds_of(moving) else {
        return AlignOutcome::Failed {
            rejection: AlignFailure::MovingSurfaceMissing,
        };
    };
    let Some((fixed_center, fixed_extent)) = occluview_align::bounds_of(fixed_soup) else {
        return AlignOutcome::Failed {
            rejection: AlignFailure::FixedSurfaceMissing,
        };
    };
    let bounds = FitBounds {
        moving_center,
        moving_extent,
        fixed_center,
        fixed_extent,
    };

    match fit_pairs(
        &moving_points,
        &fixed_points,
        Some((&moving_normals, &fixed_normals)),
        &bounds,
    ) {
        Ok(fit) => AlignOutcome::Aligned {
            pose: fit.rigid,
            rejected: fit.rejected,
        },
        Err(rejection) => AlignOutcome::Failed {
            rejection: AlignFailure::Fit(rejection),
        },
    }
}

// Split out to hold the workspace's 800-line file budget. A `#[path]` child
// module so the tests still reach this file's private items.
#[cfg(test)]
#[path = "align_worker_tests.rs"]
mod tests;
