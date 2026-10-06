//! Background execution for Align Scans.
//!
//! Heavy alignment and deviation work runs off the UI thread.
//!
//! Each job carries a generation; completions from older generations are
//! discarded. A monotonically increasing request id additionally makes the
//! latest submission win when cancellation races with a fast completion in the
//! same scene generation.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use eframe::egui;
use glam::DVec3;
use occluview_align::suggested_scale_mm;
use occluview_align::{
    deviation, deviation_stats, display_map, fit_pairs, observability, ramp_color, CancelFlag,
    DeviationMap, DeviationSettings, DeviationStats, FitBounds, FitRejection, Observability,
    Orientation, RampMode, RampSettings, Rigid, Soup, SurfaceIndex, Validity, NO_DATA_COLOR,
};
use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

/// Initial display maximum, in millimetres: the hot end of the clinical
/// deviation bar is 100 um, so a 10 um gap already reads as a real mismatch
/// rather than the low end of a wide band.
pub(crate) const WORKING_MAX_MM: f64 = 0.10;
/// Initial cool end of the displayed heatmap range. Zero, so the map starts at
/// "no measurable gap" instead of hiding everything below 50 um.
pub(crate) const WORKING_MIN_DISPLAY_MM: f64 = 0.0;
/// Absolute zero of the operator-controlled deviation display range.
pub(crate) const WORKING_SCALE_MIN_MM: f64 = 0.0;
/// Initial nominal tolerance band, in millimetres.
pub(crate) const WORKING_MIN_MM: f64 = 0.01;

/// Operator-facing knobs, in the operator's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignSettings {
    /// Farthest a moving vertex looks for fixed surface, in millimetres.
    pub(crate) influence_radius_mm: f64,
    /// How the two surfaces are taken to face each other.
    pub(crate) orientation: Orientation,
    /// Deviation mapped to the ends of the colour ramp, in millimetres.
    pub(crate) scale_mm: f64,
    /// Cool end of the displayed deviation range, in millimetres.
    pub(crate) min_display_mm: f64,
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
            orientation: Orientation::Match,
            // Start at the clinical range: zero to 100 um.
            scale_mm: WORKING_MAX_MM,
            min_display_mm: WORKING_MIN_DISPLAY_MM,
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
    fn search(self) -> occluview_align::SearchSettings {
        use occluview_align::{NormalPolicy, SearchSettings};
        SearchSettings {
            normal_policy: match self.orientation {
                Orientation::Match => NormalPolicy::Match,
                Orientation::Inverted => NormalPolicy::Opposed,
                Orientation::Ignored => NormalPolicy::Unsigned,
            },
            ..SearchSettings::default()
        }
    }

    fn deviation(self) -> DeviationSettings {
        DeviationSettings {
            influence_radius_mm: self.influence_radius_mm,
            orientation: self.orientation,
        }
    }

    /// Finite cool and hot display endpoints within the working range.
    pub(crate) fn display_limits(self) -> (f64, f64) {
        let scale_mm = if self.scale_mm.is_nan() {
            WORKING_MAX_MM
        } else {
            self.scale_mm
        }
        .clamp(WORKING_SCALE_MIN_MM, WORKING_MAX_MM);
        let min_mm = if self.min_display_mm.is_nan() {
            WORKING_MIN_DISPLAY_MM
        } else {
            self.min_display_mm
        }
        .clamp(WORKING_SCALE_MIN_MM, scale_mm);
        (min_mm, scale_mm)
    }

    fn ramp(self) -> RampSettings {
        let (min_mm, scale_mm) = self.display_limits();
        RampSettings {
            min_mm,
            scale_mm,
            tolerance_mm: self.tolerance_mm,
            // The operator-facing Align Meshes map is one continuous absolute
            // scale. The field exists only so stored settings load; a stored
            // band count never quantizes a measurement.
            bands: None,
            mode: self.ramp_mode,
        }
    }
}

/// Whether a settings edit changes what a fit or its measurement means.
/// Display range and visibility are excluded: they can recolour
/// an already landed measurement, while these two inputs require a new Best
/// fit result before the heatmap may describe the session again.
pub(crate) fn matching_inputs_changed(before: AlignSettings, after: AlignSettings) -> bool {
    before.influence_radius_mm.to_bits() != after.influence_radius_mm.to_bits()
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
    /// The submission sequence assigned by [`AlignWorker::submit`]. The value
    /// in a caller-built job is ignored and exists only to keep the snapshot
    /// self-contained for the worker boundary.
    pub(crate) request_id: u64,
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
    /// Authored affine retained without absorbing scale into a rigid fit.
    pub(crate) authored_pose: glam::Affine3A,
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
    /// The clicked pairs do not determine a pose, carrying the refusal to
    /// report.
    Fit(FitRejection),
    /// The fixed scan has no usable surface.
    FixedSurfaceMissing,
    /// The moving scan has no usable surface.
    MovingSurfaceMissing,
    /// The cached measurement was dropped before it could be coloured.
    MeasurementDropped,
    /// The map has samples, but they do not expose enough rigid motion to be
    /// a reliable visual confirmation of the match.
    MeasurementUnobservable,
}

/// What a finished job produced.
pub(crate) enum AlignOutcome {
    /// The clicked pairs were fitted. The pose corrects the placement the job
    /// was submitted with.
    Aligned {
        /// The correction to compose with the submitted placement.
        correction: Rigid,
        /// Pairs dropped as outliers.
        rejected: Vec<u32>,
    },
    /// A surface search finished, best pose first. Each pose corrects the
    /// placement the job was submitted with.
    Candidates(occluview_align::AlignmentSearchResult),
    /// Encountered non-finite numeric input, with its field and scalar index.
    InvalidInput(occluview_align::AlignmentInputError),
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
    /// The request that produced this completion.
    pub(crate) request_id: u64,
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

/// Shared worker state used by the background thread as one unit.
struct WorkerThread {
    queue: Arc<JobQueue>,
    completions: Arc<Mutex<Vec<AlignCompletion>>>,
    running: Arc<Mutex<Option<CancelFlag>>>,
    busy: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    repaint_ctx: Option<egui::Context>,
}

/// The worker handle the app holds.
pub(crate) struct AlignWorker {
    queue: Arc<JobQueue>,
    completions: Arc<Mutex<Vec<AlignCompletion>>>,
    running: Arc<Mutex<Option<CancelFlag>>>,
    generation: Arc<AtomicU64>,
    request_sequence: Arc<AtomicU64>,
    busy: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl AlignWorker {
    /// Start the worker thread.
    #[cfg(test)]
    pub(crate) fn spawn() -> Self {
        Self::spawn_inner(None)
    }

    /// Start a worker that wakes the owning UI when it publishes a result or
    /// fails. A frame can drain just before either event, after which the busy
    /// state is already false; the worker-side notification closes that idle
    /// loop race.
    pub(crate) fn spawn_with_repaint(repaint_ctx: egui::Context) -> Self {
        Self::spawn_inner(Some(repaint_ctx))
    }

    fn spawn_inner(repaint_ctx: Option<egui::Context>) -> Self {
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
        let request_sequence = Arc::new(AtomicU64::new(0));
        let busy = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));

        let thread_state = WorkerThread {
            queue: Arc::clone(&queue),
            completions: Arc::clone(&completions),
            running: Arc::clone(&running),
            busy: Arc::clone(&busy),
            failed: Arc::clone(&failed),
            repaint_ctx: repaint_ctx.clone(),
        };
        let handle = thread::Builder::new()
            .name("occluview-align".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker(&thread_state);
                }));
                if let Err(payload) = result {
                    mark_failed(
                        &thread_state.failed,
                        "align worker panicked",
                        Some(panic_message(payload)),
                    );
                }
                if thread_state.failed.load(Ordering::Acquire) {
                    if let Some(ctx) = thread_state.repaint_ctx.as_ref() {
                        ctx.request_repaint();
                    }
                }
            })
            .map_err(|error| {
                mark_failed(&failed, "thread spawn failed", Some(error.to_string()));
            })
            .ok();
        if handle.is_none() {
            if let Some(ctx) = repaint_ctx.as_ref() {
                ctx.request_repaint();
            }
        }

        Self {
            queue,
            completions,
            running,
            generation,
            request_sequence,
            busy,
            failed,
            handle,
        }
    }

    /// Publish a completion as if the worker thread had produced it.
    ///
    /// Test-only. `drain` keeps only the newest request, and `submit` mints a
    /// fresh request id every time, so a submission can never put two
    /// completions in front of the UI at once. The app's drain loop still has to
    /// re-read the generation between them, because applying one result can
    /// invalidate the rest of the batch; this is how that boundary is reached.
    #[cfg(test)]
    pub(crate) fn publish_for_tests(&self, generation: u64, outcome: AlignOutcome) {
        let request_id = self.request_sequence.load(Ordering::SeqCst);
        if let Ok(mut published) = self.completions.lock() {
            published.push(AlignCompletion {
                generation,
                request_id,
                outcome,
            });
        }
    }

    /// Whether this worker can still accept or publish work.
    /// Poison this worker's queue, the way a panicking job does.
    ///
    /// Test-only: the real failure path is a panic inside the worker thread,
    /// which cannot be provoked from outside without a job that panics.
    #[cfg(test)]
    #[allow(clippy::expect_used, clippy::panic)]
    pub(crate) fn poison_queue_for_tests(&mut self) {
        let queue = Arc::clone(&self.queue);
        let _ = thread::spawn(move || {
            let _guard = queue.state.lock().expect("queue lock before poisoning");
            panic!("poison the align worker queue");
        })
        .join();
        self.queue.wake.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    pub(crate) fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Move to a new generation, so every result still in flight is discarded.
    pub(crate) fn bump_generation(&self) -> u64 {
        let Ok(mut state) = self.queue.state.lock() else {
            let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            mark_failed(&self.failed, "queue lock poisoned", None);
            self.clear_completions();
            return next;
        };
        let Ok(running) = self.running.lock() else {
            let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            state.jobs.clear();
            mark_failed(&self.failed, "running-job lock poisoned", None);
            drop(state);
            self.clear_completions();
            return next;
        };
        if let Some(flag) = running.as_ref() {
            flag.cancel();
        }
        let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        state.jobs.clear();
        drop(running);
        drop(state);
        self.clear_completions();
        next
    }

    /// Clear any result that arrived before or during generation retirement.
    fn clear_completions(&self) {
        match self.completions.lock() {
            Ok(mut completions) => completions.clear(),
            Err(_) => mark_failed(&self.failed, "completion lock poisoned", None),
        }
    }

    /// The generation new jobs should carry.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Whether anything is queued or running.
    ///
    /// A failed worker cannot run or accept its queued work.
    pub(crate) fn is_busy(&self) -> bool {
        if self.has_failed() {
            return false;
        }
        let queued = self
            .queue
            .state
            .lock()
            .is_ok_and(|state| !state.jobs.is_empty());
        queued || self.busy.load(Ordering::SeqCst) > 0
    }

    /// Whether a finished result is waiting for the UI to drain it.
    ///
    /// Kept separate from `is_busy`: callers that wait for computation to
    /// finish should not stay busy merely because the UI has not applied the
    /// result yet, while the egui loop still needs one frame to consume it.
    pub(crate) fn has_pending_output(&self) -> bool {
        if let Ok(completions) = self.completions.lock() {
            !completions.is_empty()
        } else {
            mark_failed(&self.failed, "completion lock poisoned", None);
            true
        }
    }

    /// Queue the newest job and cancel every older queued/running request.
    ///
    /// Align jobs are mutually exclusive snapshots: a queued refine followed
    /// by a measure, or an old measure followed by a new refine, cannot both be
    /// correct for the operator's current intent. Clearing the whole queue
    /// avoids applying a stale kind after a newer kind has landed.
    pub(crate) fn submit(&self, mut job: AlignJob) -> bool {
        if self.has_failed() {
            return false;
        }
        // The worker pops and registers its cancellation token under this same
        // queue -> running lock order. Holding the queue while cancelling and
        // replacing its job closes the interval where a popped job existed but
        // its token was not yet visible to the submitter.
        let Ok(mut state) = self.queue.state.lock() else {
            mark_failed(&self.failed, "queue lock poisoned", None);
            return false;
        };
        let Ok(running) = self.running.lock() else {
            mark_failed(&self.failed, "running-job lock poisoned", None);
            return false;
        };
        if let Some(flag) = running.as_ref() {
            flag.cancel();
        }
        job.request_id = self.request_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        state.jobs.clear();
        state.jobs.push_back(job);
        drop(running);
        drop(state);
        self.queue.wake.notify_one();
        true
    }

    /// Take every completion that still belongs to the current generation.
    pub(crate) fn drain(&self) -> Vec<AlignCompletion> {
        let current = self.generation();
        let newest_request = self.request_sequence.load(Ordering::SeqCst);
        let Ok(mut completions) = self.completions.lock() else {
            mark_failed(&self.failed, "completion lock poisoned", None);
            return Vec::new();
        };
        let drained: Vec<AlignCompletion> = completions.drain(..).collect();
        drained
            .into_iter()
            .filter(|completion| {
                completion.generation == current && completion.request_id == newest_request
            })
            .collect()
    }
}

impl Drop for AlignWorker {
    fn drop(&mut self) {
        {
            if let Ok(mut state) = self.queue.state.lock() {
                if let Ok(running) = self.running.lock() {
                    if let Some(flag) = running.as_ref() {
                        flag.cancel();
                    }
                } else {
                    mark_failed(&self.failed, "running-job lock poisoned", None);
                }
                state.shutdown = true;
                state.jobs.clear();
            } else if let Ok(running) = self.running.lock() {
                if let Some(flag) = running.as_ref() {
                    flag.cancel();
                }
            }
        }
        self.queue.wake.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The worker loop: take a job, run it, publish what came out.
fn run_worker(worker: &WorkerThread) {
    let mut cached = WorkerCache::default();
    loop {
        let (job, cancel, _busy) = {
            let Ok(mut state) = worker.queue.state.lock() else {
                mark_failed(&worker.failed, "queue lock poisoned", None);
                return;
            };
            while state.jobs.is_empty() && !state.shutdown {
                let Ok(next) = worker.queue.wake.wait(state) else {
                    mark_failed(&worker.failed, "queue wait poisoned", None);
                    return;
                };
                state = next;
            }
            if state.shutdown {
                return;
            }
            let Some(job) = state.jobs.pop_front() else {
                continue;
            };
            let cancel = CancelFlag::new();
            let Ok(mut slot) = worker.running.lock() else {
                mark_failed(&worker.failed, "running-job lock poisoned", None);
                return;
            };
            *slot = Some(cancel.clone());
            drop(slot);
            // Start before releasing the queue lock so `is_busy` sees either
            // queued work or a registered running job. RAII covers every exit.
            (job, cancel, Busy::new(&worker.busy))
        };
        let outcome = execute(&job, &cancel, &mut cached);
        // Cancelled stages may return structurally valid but unusable values;
        // do not publish them.
        let abandoned = cancel.is_cancelled();

        let Ok(mut slot) = worker.running.lock() else {
            mark_failed(&worker.failed, "running-job lock poisoned", None);
            return;
        };
        *slot = None;
        drop(slot);
        if abandoned {
            continue;
        }
        let Ok(mut published) = worker.completions.lock() else {
            mark_failed(&worker.failed, "completion lock poisoned", None);
            return;
        };
        published.push(AlignCompletion {
            generation: job.generation,
            request_id: job.request_id,
            outcome,
        });
        drop(published);
        if let Some(ctx) = worker.repaint_ctx.as_ref() {
            ctx.request_repaint();
        }
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

/// A panic-safe hold on the busy counter.
///
/// The counter is what the panel shows as a spinner and what
/// `finish_align_session` reads before claiming a session ended while work was
/// running. Decrementing it by hand means any early return or unwind between
/// the two calls leaves Align busy forever.
struct Busy(Arc<AtomicU64>);

impl Busy {
    fn new(counter: &Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
    /// That map as it is painted: calmed of single-vertex noise. Made once per
    /// measurement, so dragging the display range does not redo it.
    shown: Option<(MeasureKey, DeviationMap)>,
    /// The last summary, and the measurement and tolerance it was taken at.
    summary: Option<(MeasureKey, u64, DeviationStats)>,
    /// What that measurement was capable of seeing. Independent of the ramp and
    /// the tolerance, so it survives a re-colour as the map does.
    seen: Option<(MeasureKey, Option<Observability>)>,
}

/// Run one job.
fn execute(job: &AlignJob, cancel: &CancelFlag, cached: &mut WorkerCache) -> AlignOutcome {
    let moving = Soup {
        positions: &job.moving_positions,
        indices: &job.moving_indices,
        mask: job.mask.as_ref().map(|mask| mask.as_slice()),
    };

    match job.kind {
        AlignJobKind::Align => return align_from_pairs(job, moving),
        AlignJobKind::Refine => return execute_search(job, cancel),
        AlignJobKind::Measure => {}
    }

    // Re-colouring changes only the display, so reuse the cached map.
    if cached
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

    let map = deviation(moving, index, job.pose, &job.settings.deviation(), cancel);
    // Do not cache a cancelled map or expose it to later re-colouring.
    let seen = observability(moving, index, job.pose, &job.settings.deviation(), cancel);
    if !cancel.is_cancelled() {
        cached.shown = Some((job.measure_key, display_map(&map, moving)));
        cached.measured = Some((job.measure_key, map));
        cached.summary = None;
        cached.seen = Some((job.measure_key, seen));
        return recolor(job, cached);
    }
    let stats = deviation_stats(&map, job.settings.tolerance_mm);
    paint(&map, job, stats, seen)
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
    // The colours come from the calmed copy; the numbers stay on the raw map.
    let shown = cached
        .shown
        .as_ref()
        .filter(|(key, _)| *key == job.measure_key)
        .map_or(map, |(_, shown)| shown);
    paint(shown, job, stats, seen)
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
    // A numerically valid distance map can still be blind to a rigid slide or
    // turn. Do not publish colours that look authoritative when the sampled
    // surface cannot determine the motion that produced them.
    //
    // `None` is the degenerate end of that: too little surface, or samples that
    // do not span six degrees of freedom. It is not extended to a *weak* blind
    // direction. `observability()` exists to report those (see
    // `hidden_displacement_mm`, measured at 0.94-1.007 of the truth on real arch
    // scans), and `has_blind_direction` is the threshold that says "the estimate
    // is doing real work here", not "this measurement is worthless". Refusing on
    // it would block legitimate full-arch alignments: the sensitivity allowed
    // for a real arch in `real_scans.rs` extends below it.
    if stats.summary.is_some() && seen.is_none() {
        return AlignOutcome::Failed {
            rejection: AlignFailure::MeasurementUnobservable,
        };
    }
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

/// Fit the clicked pairs in closed form. No surface is searched, so the pose
/// lands as soon as the job is taken.
///
/// Both point sets are fitted in world, the moving one through the placement
/// the job was submitted with, so the result corrects that placement and any
/// authored scale in it stays where it was.
fn align_from_pairs(job: &AlignJob, moving: Soup<'_>) -> AlignOutcome {
    let placed = job.authored_pose.as_daffine3();
    let facing = placed.matrix3.inverse().transpose();
    let moving_points: Vec<DVec3> = job
        .pairs
        .iter()
        .map(|pair| placed.transform_point3(pair.moving))
        .collect();
    let fixed_points: Vec<DVec3> = job.pairs.iter().map(|pair| pair.fixed).collect();
    let moving_normals: Vec<DVec3> = job
        .pairs
        .iter()
        .map(|pair| (facing * pair.moving_normal).normalize_or_zero())
        .collect();
    let fixed_normals: Vec<DVec3> = job.pairs.iter().map(|pair| pair.fixed_normal).collect();
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
    let placed_scale = placed
        .matrix3
        .x_axis
        .length()
        .max(placed.matrix3.y_axis.length())
        .max(placed.matrix3.z_axis.length());
    let bounds = FitBounds {
        moving_center: placed.transform_point3(moving_center),
        moving_extent: moving_extent * placed_scale,
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
            correction: fit.rigid,
            rejected: fit.rejected,
        },
        Err(rejection) => AlignOutcome::Failed {
            rejection: AlignFailure::Fit(rejection),
        },
    }
}

/// Search the two surfaces for the poses that seat one on the other.
fn execute_search(job: &AlignJob, cancel: &CancelFlag) -> AlignOutcome {
    use occluview_align::{AlignmentInput, MeshInput, SearchControl};
    let input = AlignmentInput {
        moving: MeshInput {
            soup: Soup {
                positions: &job.moving_positions,
                indices: &job.moving_indices,
                mask: job.mask.as_deref().map(Vec::as_slice),
            },
            world_from_local: job.authored_pose.as_daffine3(),
            revision: job.measure_key.moving.0,
        },
        fixed: MeshInput {
            soup: Soup {
                positions: &job.fixed_world_positions,
                indices: &job.fixed_indices,
                mask: job.fixed_mask.as_deref().map(Vec::as_slice),
            },
            world_from_local: glam::DAffine3::IDENTITY,
            revision: job.fixed_key.geometry,
        },
        landmarks: &[],
        seeds: &[],
    };
    let settings = job.settings.search();
    match occluview_align::search_alignment(
        &input,
        &settings,
        &SearchControl::new(cancel.clone(), settings.wall_limit),
    ) {
        Ok(result) => AlignOutcome::Candidates(result),
        Err(error) => AlignOutcome::InvalidInput(error),
    }
}

// A `#[path]` child module so the tests reach this file's private items.
#[cfg(test)]
#[path = "align_worker_tests.rs"]
mod tests;
