//! Background worker for contact field measurements.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use occluview_align::CancelFlag;
use occluview_contact::{compute_contact_field, ContactDiagnostics, ContactSettings, ContactStats};

/// Inputs that determine a contact field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContactJobKeys {
    /// Geometry identity and pose of the layer wearing the marks.
    pub(crate) subject: (u64, u64),
    /// Geometry identity and pose of the surface it is measured against.
    pub(crate) antagonist: (u64, u64),
    /// Whether penetration patches are collapsed to their peak depth.
    pub(crate) flatten_patches: bool,
}

/// Why a contact field could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContactFailure {
    /// One of the two layers has no usable triangle surface.
    NoSurface,
    Worker,
}

/// One queued measurement.
pub(crate) struct ContactJob {
    /// Generation used to reject stale results.
    pub(crate) generation: u64,
    pub(crate) request_id: u64,
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
    pub(crate) request_id: u64,
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
    request_sequence: Arc<AtomicU64>,
    busy: Arc<AtomicU64>,
    unusable: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ContactWorker {
    /// Start the worker thread.
    pub(crate) fn spawn() -> Self {
        Self::spawn_with(|queue, completions, running, busy| {
            thread::Builder::new()
                .name("occluview-contacts".into())
                .spawn(move || {
                    run_worker(&queue, &completions, &running, &busy);
                })
        })
    }

    fn spawn_with<F>(spawn_thread: F) -> Self
    where
        F: FnOnce(
            Arc<JobQueue>,
            Arc<Mutex<Vec<ContactCompletion>>>,
            Arc<Mutex<Option<CancelFlag>>>,
            Arc<AtomicU64>,
        ) -> std::io::Result<JoinHandle<()>>,
    {
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
        let unusable = Arc::new(AtomicBool::new(false));
        let handle = spawn_thread(
            Arc::clone(&queue),
            Arc::clone(&completions),
            Arc::clone(&running),
            Arc::clone(&busy),
        )
        .map_err(|error| {
            tracing::warn!(%error, "contact worker thread could not be started");
            unusable.store(true, Ordering::Release);
        })
        .ok();

        Self {
            queue,
            completions,
            running,
            generation,
            request_sequence,
            busy,
            unusable,
            handle,
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_failing() -> Self {
        Self::spawn_with(|_, _, _, _| Err(std::io::Error::other("no thread for the test")))
    }

    pub(crate) fn has_failed(&self) -> bool {
        self.unusable.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn publish_for_tests(&self, completion: ContactCompletion) {
        if let Ok(mut published) = self.completions.lock() {
            published.push(completion);
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

    #[cfg(test)]
    #[must_use]
    pub(crate) fn latest_request(&self) -> Option<u64> {
        match self.request_sequence.load(Ordering::SeqCst) {
            0 => None,
            request => Some(request),
        }
    }

    /// The generation new jobs should carry.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Whether a job is queued or running.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0
            || self
                .queue
                .state
                .lock()
                .is_ok_and(|state| !state.jobs.is_empty())
    }

    /// Queue a job, replacing any queued job and cancelling the current run.
    #[must_use]
    pub(crate) fn submit(&self, mut job: ContactJob) -> Option<u64> {
        if self.has_failed() {
            return None;
        }
        self.cancel_running();
        let request_id = self.request_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let Ok(mut state) = self.queue.state.lock() else {
            mark_unusable(&self.unusable, "contact queue lock poisoned");
            return None;
        };
        job.request_id = request_id;
        state.jobs.clear();
        state.jobs.push_back(job);
        drop(state);
        self.queue.wake.notify_one();
        Some(request_id)
    }

    /// Return completions for the newest request in the current generation.
    #[must_use]
    pub(crate) fn drain(&self) -> Vec<ContactCompletion> {
        let current = self.generation();
        let newest = self.request_sequence.load(Ordering::SeqCst);
        let Ok(mut completions) = self.completions.lock() else {
            mark_unusable(&self.unusable, "contact completion lock poisoned");
            return Vec::new();
        };
        let drained: Vec<ContactCompletion> = completions.drain(..).collect();
        drained
            .into_iter()
            .filter(|completion| {
                completion.generation == current && completion.request_id == newest
            })
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
                request_id: job.request_id,
                keys: job.keys,
                outcome,
            });
        }
    }
}

/// Run one job.
fn execute(job: &ContactJob, cancel: &CancelFlag) -> ContactOutcome {
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

fn mark_unusable(unusable: &AtomicBool, reason: &'static str) {
    if !unusable.swap(true, Ordering::AcqRel) {
        tracing::warn!(reason, "contact worker cannot run");
    }
}

/// Whether a layer carries at least one whole triangle to measure against.
fn has_surface(positions: &[f32], indices: &[u32]) -> bool {
    positions.len() >= 9 && positions.len().is_multiple_of(3) && indices.len() >= 3
}

#[cfg(test)]
#[path = "contact_worker_tests.rs"]
mod tests;
