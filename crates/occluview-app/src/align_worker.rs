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
use occluview_align::{
    deviation, deviation_colors, deviation_stats, fit_pairs, refine, CancelFlag, DeviationSettings,
    DeviationStats, FitRejection, IcpReport, Orientation, RampMode, RampSettings, RefineSettings,
    Rigid, Soup, SurfaceIndex,
};

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
    /// Whether the map is on screen.
    pub(crate) show_deviation: bool,
}

impl Default for AlignSettings {
    fn default() -> Self {
        Self {
            influence_radius_mm: 2.0,
            matching_ratio: 0.8,
            orientation: Orientation::Match,
            // Real registration deviations sit at 0.05-0.3 mm. A wider scale
            // buries them in the green centre, which is exactly why the web
            // build's map reads washed out.
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            ramp_mode: RampMode::Magnitude,
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
        /// One colour per moving vertex.
        colors: Vec<[u8; 4]>,
        /// Summary over the measured vertices.
        stats: DeviationStats,
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
    let mut cached: Option<((u64, u64), SurfaceIndex)> = None;
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

        busy.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut slot) = running.lock() {
            *slot = None;
        }
        if let Ok(mut published) = completions.lock() {
            published.push(AlignCompletion {
                generation: job.generation,
                outcome,
            });
        }
    }
}

/// Run one job.
fn execute(
    job: &AlignJob,
    cancel: &CancelFlag,
    cached: &mut Option<((u64, u64), SurfaceIndex)>,
) -> AlignOutcome {
    let moving = Soup {
        positions: &job.moving_positions,
        indices: &job.moving_indices,
        mask: job.mask.as_ref().map(|mask| mask.as_slice()),
    };

    if job.kind == AlignJobKind::Align {
        return align_from_pairs(job, moving);
    }

    let index = match cached {
        Some((key, index)) if *key == job.fixed_key => index,
        _ => {
            let fixed = Soup {
                positions: &job.fixed_world_positions,
                indices: &job.fixed_indices,
                mask: None,
            };
            let Some(built) = SurfaceIndex::build(fixed) else {
                return AlignOutcome::Failed {
                    message: "The fixed scan has no usable surface".into(),
                };
            };
            *cached = Some((job.fixed_key, built));
            match cached {
                Some((_, index)) => index,
                None => {
                    return AlignOutcome::Failed {
                        message: "The fixed scan has no usable surface".into(),
                    }
                }
            }
        }
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
            let stats = deviation_stats(&map, job.settings.tolerance_mm);
            AlignOutcome::Measured {
                colors: deviation_colors(&map, &job.settings.ramp()),
                stats,
            }
        }
    }
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
