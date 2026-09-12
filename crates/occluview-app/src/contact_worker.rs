//! Background execution for the occlusal contact reading.
//!
//! A contact field is a nearest-surface search over two full arches — hundreds
//! of thousands of vertices each — and it must never run on the UI thread. The
//! shape is deliberately the same as [`crate::align_worker`]: one background
//! thread, a single queued job of a single kind, a generation that discards
//! results the operator has already moved past, and a cancel flag the compute
//! checks between sides.
//!
//! SIMPLER THAN THE ALIGN WORKER ON PURPOSE. There is no cache: a contact
//! reading has exactly one display-only knob (the load depth) and that knob does
//! not touch the worker at all — it moves stop positions in a shader uniform —
//! so there is nothing a second run could reuse. Re-measuring is what the
//! operator asked for, and it happens when the geometry, the pose, or the patch
//! rule changes.
//!
//! NO PANICS. Release builds abort on panic, so a worker must not unwrap: a
//! poisoned lock ends the thread's work quietly and the panel keeps the last
//! sentence it had.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use occluview_align::CancelFlag;
use occluview_contact::{compute_contact_field, ContactDiagnostics, ContactSettings, ContactStats};

/// Everything the distances depend on. Two readings with equal keys produce the
/// same field, so a change in any of these is the only thing that re-runs a
/// measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContactJobKeys {
    /// Geometry identity and pose of the layer wearing the marks.
    pub(crate) subject: (u64, u64),
    /// Geometry identity and pose of the surface it is measured against.
    pub(crate) antagonist: (u64, u64),
    /// Whether penetration patches are collapsed to their peak depth.
    pub(crate) flatten_patches: bool,
}

/// Why a reading produced nothing trustworthy. Domain data only: the worker
/// never renders a sentence; the panel renders copy from this reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContactFailure {
    /// One of the two layers has no usable triangle surface.
    NoSurface,
    /// Nothing usable came out of the compute.
    Worker,
}

/// One queued measurement. Geometry arrives through `Arc`, so submitting never
/// copies a mesh.
pub(crate) struct ContactJob {
    /// The generation this job belongs to, so a result the operator has already
    /// moved past is discarded rather than applied.
    pub(crate) generation: u64,
    /// What the measurement describes; echoed back with the result.
    pub(crate) keys: ContactJobKeys,
    /// Subject vertex positions, posed into world, xyz triples.
    pub(crate) subject_positions: Arc<Vec<f32>>,
    /// Subject triangle indices.
    pub(crate) subject_indices: Arc<Vec<u32>>,
    /// Antagonist vertex positions, posed into world, xyz triples.
    pub(crate) antagonist_positions: Arc<Vec<f32>>,
    /// Antagonist triangle indices.
    pub(crate) antagonist_indices: Arc<Vec<u32>>,
    /// Search radius and the patch rule.
    pub(crate) settings: ContactSettings,
}

/// What a finished job produced.
pub(crate) enum ContactOutcome {
    /// A field landed, one value per vertex on each side.
    Measured {
        /// Signed millimetres per subject vertex, in subject vertex order.
        subject_signed_mm: Vec<f32>,
        /// Signed millimetres per antagonist vertex.
        antagonist_signed_mm: Vec<f32>,
        /// Contact area, patch count, and the depth extremes.
        stats: ContactStats,
        /// Counters and timings, for the log line.
        diagnostics: ContactDiagnostics,
    },
    /// Nothing trustworthy came out, and this is why.
    Failed(ContactFailure),
}

/// A finished job, tagged with what it was measuring.
pub(crate) struct ContactCompletion {
    /// The generation the job belonged to.
    pub(crate) generation: u64,
    /// The keys the job was submitted for.
    pub(crate) keys: ContactJobKeys,
    /// The result.
    pub(crate) outcome: ContactOutcome,
}

struct QueueState {
    jobs: VecDeque<ContactJob>,
    shutdown: bool,
}

struct JobQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}

/// The worker handle the app holds.
pub(crate) struct ContactWorker {
    queue: Arc<JobQueue>,
    completions: Arc<Mutex<Vec<ContactCompletion>>>,
    running: Arc<Mutex<Option<CancelFlag>>>,
    generation: Arc<AtomicU64>,
    busy: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl ContactWorker {
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
            .name("occluview-contacts".into())
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

    /// Ask a running job to stop.
    pub(crate) fn cancel_running(&self) {
        if let Ok(running) = self.running.lock() {
            if let Some(flag) = running.as_ref() {
                flag.cancel();
            }
        }
    }

    /// Move to a new generation, so every result still in flight is discarded.
    pub(crate) fn bump_generation(&self) -> u64 {
        self.cancel_running();
        if let Ok(mut state) = self.queue.state.lock() {
            state.jobs.clear();
        }
        if let Ok(mut completions) = self.completions.lock() {
            completions.clear();
        }
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The generation new jobs should carry.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Whether anything is queued or running on the worker thread.
    ///
    /// Distinct from [`crate::contact::ContactState::is_busy`]: this answers
    /// about the thread, that one about the reading on screen.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0
            || self
                .queue
                .state
                .lock()
                .is_ok_and(|state| !state.jobs.is_empty())
    }

    /// Queue a job, replacing anything already queued and cancelling the run in
    /// progress.
    ///
    /// One pending job is the whole queue: a reading always describes the scene
    /// as it is now, and computing an older one on the way would only delay the
    /// answer the operator is waiting for.
    pub(crate) fn submit(&self, job: ContactJob) {
        self.cancel_running();
        let Ok(mut state) = self.queue.state.lock() else {
            return;
        };
        state.jobs.clear();
        state.jobs.push_back(job);
        drop(state);
        self.queue.wake.notify_one();
    }

    /// Take every completion that still belongs to the current generation.
    #[must_use]
    pub(crate) fn drain(&self) -> Vec<ContactCompletion> {
        let current = self.generation();
        let Ok(mut completions) = self.completions.lock() else {
            return Vec::new();
        };
        let drained: Vec<ContactCompletion> = completions.drain(..).collect();
        drained
            .into_iter()
            .filter(|completion| completion.generation == current)
            .collect()
    }
}

impl Drop for ContactWorker {
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
    completions: &Arc<Mutex<Vec<ContactCompletion>>>,
    running: &Arc<Mutex<Option<CancelFlag>>>,
    busy: &Arc<AtomicU64>,
) {
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
        let outcome = execute(&job, &cancel);
        // A cancelled compute returns partial values; publishing them would put
        // a half-measured map under a legend that describes the whole one.
        let abandoned = cancel.is_cancelled();
        busy.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut slot) = running.lock() {
            *slot = None;
        }
        if abandoned {
            continue;
        }
        if let Ok(mut published) = completions.lock() {
            published.push(ContactCompletion {
                generation: job.generation,
                keys: job.keys,
                outcome,
            });
        }
    }
}

/// Run one job.
fn execute(job: &ContactJob, cancel: &CancelFlag) -> ContactOutcome {
    // A layer with no triangle surface cannot be measured against, and saying so
    // here is what keeps the message on the panel about the operator's case
    // rather than about the compute.
    if !has_surface(&job.subject_positions, &job.subject_indices)
        || !has_surface(&job.antagonist_positions, &job.antagonist_indices)
    {
        return ContactOutcome::Failed(ContactFailure::NoSurface);
    }

    let field = compute_contact_field(
        crate::contact::world_soup(&job.subject_positions, &job.subject_indices),
        crate::contact::world_soup(&job.antagonist_positions, &job.antagonist_indices),
        job.settings,
        cancel,
    );

    if field.subject_signed_mm.len() != job.subject_positions.len() / 3
        || field.antagonist_signed_mm.len() != job.antagonist_positions.len() / 3
    {
        return ContactOutcome::Failed(ContactFailure::Worker);
    }

    tracing::info!(
        subject_verts = field.diagnostics.subject_verts,
        antagonist_verts = field.diagnostics.antagonist_verts,
        subject_measured = field.diagnostics.subject_measured,
        contact_area_mm2 = field.stats.contact_area_mm2,
        contacts = field.stats.contacts,
        worker_ms = field.diagnostics.worker_ms,
        "contact field measured"
    );

    ContactOutcome::Measured {
        subject_signed_mm: field.subject_signed_mm,
        antagonist_signed_mm: field.antagonist_signed_mm,
        stats: field.stats,
        diagnostics: field.diagnostics,
    }
}

/// Whether a layer carries at least one whole triangle to measure against.
fn has_surface(positions: &[f32], indices: &[u32]) -> bool {
    positions.len() >= 9 && positions.len().is_multiple_of(3) && indices.len() >= 3
}

#[cfg(test)]
#[path = "contact_worker_tests.rs"]
mod tests;
