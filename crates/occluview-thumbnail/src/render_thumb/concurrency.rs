// Poison-recovery tests deliberately panic while holding a lock to poison it,
// then assert the pool/gate recover; that needs unwrap/expect/panic in test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

use super::{
    Duration, Mutex, ThumbnailAttempt, ThumbnailError, ThumbnailRequestKey, THUMBNAIL_INFLIGHT,
    THUMBNAIL_JOB_GATE, THUMBNAIL_RENDERER_POOL,
};
use crate::offscreen_factory::create_thumbnail_offscreen;
use occluview_render::Offscreen;
use std::collections::{HashMap, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, MutexGuard, PoisonError};
use std::time::Instant;

// A shell surrogate is a shared, memory-constrained host, not a render farm.
// Decode jobs are bounded, and the bound is a latency decision rather than a
// throughput one: measured on a folder of 120 real scans, three lanes give
// 48.6 files/s at a median of 45 ms per file, six give 58.6 at 87 ms, and
// twelve give 57.9 at 185 ms. Past six the queue is the only thing that grows
// -- and under Explorer's Apartment hosting every extraction of this CLSID
// serialises through one host thread, so a request that waits its turn inside
// this process is a request the whole folder waits behind.
//
// GPU work has a separate, much smaller budget: every Offscreen owns a wgpu
// device, and creating several D3D devices concurrently makes the driver
// serialize or contend instead of making thumbnails faster.
const MAX_THUMBNAIL_JOB_LANES: usize = 6;
const MAX_THUMBNAIL_RENDERERS: usize = 1;

/// Lock a shared thumbnail mutex, recovering the guard even if a previous
/// holder panicked and poisoned it.
///
/// Poison-tolerance is deliberate and load-bearing for **per-request
/// isolation**: the thumbnail statics (renderer pool, job gate, in-flight map)
/// are shared by every concurrent `IThumbnailProvider` in a `dllhost`
/// surrogate. If one file's render panicked *while* one of these locks was held
/// and we treated the resulting poison as fatal, every *other* file in the
/// folder would then fail to check out a renderer / release a permit — a single
/// bad file would silently blank the whole mixed folder. The pool/gate/map
/// state is plain bookkeeping (idle renderers, an active-job counter, a
/// coalescing map); recovering it is always safe and self-correcting.
fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct ThumbnailRendererPool {
    state: Mutex<ThumbnailRendererPoolState>,
    ready: Condvar,
    max_renderers: usize,
    /// How a renderer is built. A function pointer rather than a direct call
    /// so the recovery paths below -- a create that fails, and a create that
    /// panics -- can be exercised without a GPU.
    create: fn() -> Result<Offscreen, ThumbnailError>,
}

/// Longest a request may wait for the pooled renderer before it gives up.
///
/// This is not the request's budget; the request has its own, and Explorer
/// abandons the call long before this. It is the backstop that keeps a
/// renderer lost to a wedged device from parking every decode lane forever:
/// a worker waiting here holds its lane, so twelve of them are the whole
/// folder. Reaching this ceiling means something is wrong, not that the
/// machine is busy.
const RENDERER_WAIT_CEILING: Duration = Duration::from_secs(30);

/// The pool never produced a renderer inside the ceiling above.
///
/// Reported as a render error, which the request path already treats as
/// transient: Explorer retries, and a retry is exactly right for a pool that
/// is merely oversubscribed.
fn renderer_wait_expired() -> ThumbnailError {
    ThumbnailError::Render(occluview_render::RenderError::Surface(
        "timed out waiting for the thumbnail renderer".to_string(),
    ))
}

pub(super) struct ThumbnailJobGate {
    inner: Arc<ThumbnailJobGateInner>,
}

struct ThumbnailJobGateInner {
    state: Mutex<ThumbnailJobGateState>,
    ready: Condvar,
    max_jobs: usize,
}

#[derive(Default)]
struct ThumbnailJobGateState {
    active_jobs: usize,
    next_ticket: u64,
    waiters: VecDeque<u64>,
}

impl ThumbnailJobGateState {
    fn remove_waiter(&mut self, ticket: u64) {
        if let Some(position) = self.waiters.iter().position(|queued| *queued == ticket) {
            let _ = self.waiters.remove(position);
        }
    }
}

pub(super) struct ThumbnailJobPermit {
    gate: Arc<ThumbnailJobGateInner>,
}

pub(super) struct InflightThumbnail {
    state: Mutex<InflightThumbnailState>,
    ready: Condvar,
}

enum InflightThumbnailState {
    Running,
    Finished(InflightThumbnailResult),
}

/// The clonable form of a leader's verdict, published to coalesced followers.
///
/// Followers must inherit the leader's transient/deterministic split: handing
/// a follower a placeholder bitmap while the leader reported a transient
/// failure would put exactly the cacheable stand-in into Explorer's thumbcache
/// that the split exists to prevent.
#[derive(Clone)]
enum InflightThumbnailResult {
    Bitmap(Vec<u8>),
    TransientFailure,
}

impl From<ThumbnailAttempt> for InflightThumbnailResult {
    fn from(attempt: ThumbnailAttempt) -> Self {
        match attempt {
            ThumbnailAttempt::Bitmap(pixels) => Self::Bitmap(pixels),
            ThumbnailAttempt::TransientFailure => Self::TransientFailure,
        }
    }
}

impl From<InflightThumbnailResult> for ThumbnailAttempt {
    fn from(result: InflightThumbnailResult) -> Self {
        match result {
            InflightThumbnailResult::Bitmap(pixels) => Self::Bitmap(pixels),
            InflightThumbnailResult::TransientFailure => Self::TransientFailure,
        }
    }
}

pub(super) enum InflightThumbnailLease {
    Leader(Arc<InflightThumbnail>),
    Follower(Arc<InflightThumbnail>),
}

pub(super) enum ThumbnailJobProgress<T> {
    Prepared,
    Finished(T),
}

pub(super) enum ThumbnailJobOutcome<T> {
    Finished(T),
    SetupTimedOut,
    RenderTimedOut,
    Failed,
}

#[derive(Default)]
struct ThumbnailRendererPoolState {
    idle: Vec<Offscreen>,
    total_renderers: usize,
}

impl ThumbnailRendererPool {
    pub(super) fn shared() -> &'static Self {
        THUMBNAIL_RENDERER_POOL.get_or_init(|| Self::new(default_thumbnail_renderer_pool_size()))
    }

    pub(super) const fn new(max_renderers: usize) -> Self {
        Self::with_create(max_renderers, create_thumbnail_offscreen)
    }

    pub(super) const fn with_create(
        max_renderers: usize,
        create: fn() -> Result<Offscreen, ThumbnailError>,
    ) -> Self {
        Self {
            create,
            state: Mutex::new(ThumbnailRendererPoolState {
                idle: Vec::new(),
                total_renderers: 0,
            }),
            ready: Condvar::new(),
            max_renderers,
        }
    }

    pub(super) fn with_renderer<R>(
        &self,
        f: impl FnOnce(&Offscreen) -> Result<R, ThumbnailError>,
    ) -> Result<R, ThumbnailError> {
        let renderer = self.checkout_renderer()?;
        let lease = ThumbnailRendererLease::new(self, renderer);
        let Some(offscreen) = lease.offscreen.as_ref() else {
            return Err(ThumbnailError::Render(
                occluview_render::RenderError::Surface(
                    "thumbnail renderer lease lost its Offscreen".to_string(),
                ),
            ));
        };

        // A panic is the same evidence as an error: whatever the renderer was
        // doing, it did not finish, and its device may be lost or mid-command.
        // Unwinding on its own would run the lease's `Drop`, which parks the
        // renderer back in the pool for the next file in the folder.
        match panic::catch_unwind(AssertUnwindSafe(|| f(offscreen))) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                // A renderer that reported a GPU/readback error may have a
                // lost device or stale backend state. Never hand it to the
                // next Explorer request; release the pool capacity instead.
                lease.discard();
                Err(error)
            }
            Err(payload) => {
                lease.discard();
                panic::resume_unwind(payload);
            }
        }
    }

    pub(super) fn checkout_renderer(&self) -> Result<Offscreen, ThumbnailError> {
        self.checkout_renderer_within(RENDERER_WAIT_CEILING)
    }

    pub(super) fn checkout_renderer_within(
        &self,
        budget: Duration,
    ) -> Result<Offscreen, ThumbnailError> {
        let deadline = Instant::now() + budget;
        loop {
            let mut state = lock_recover(&self.state);
            if let Some(offscreen) = state.idle.pop() {
                return Ok(offscreen);
            }
            if state.total_renderers < self.max_renderers {
                state.total_renderers += 1;
                drop(state);
                // The slot is claimed before the device exists, so every way
                // out of the create has to give it back. An unwind out of wgpu
                // -- a driver reset during device creation is exactly the kind
                // of thing a long-lived surrogate sees -- would otherwise leave
                // the pool believing its one renderer is in use forever, and
                // every later request waiting for a renderer that is not
                // coming.
                match panic::catch_unwind(AssertUnwindSafe(self.create)) {
                    Ok(Ok(offscreen)) => return Ok(offscreen),
                    Ok(Err(error)) => {
                        self.release_reservation();
                        return Err(error);
                    }
                    Err(payload) => {
                        self.release_reservation();
                        panic::resume_unwind(payload);
                    }
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(renderer_wait_expired());
            }
            let (_guard, wait) = self
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            if wait.timed_out() && Instant::now() >= deadline {
                return Err(renderer_wait_expired());
            }
        }
    }

    fn release_reservation(&self) {
        let mut state = lock_recover(&self.state);
        state.total_renderers = state.total_renderers.saturating_sub(1);
        drop(state);
        self.ready.notify_one();
    }

    fn return_renderer(&self, offscreen: Offscreen) {
        let mut state = lock_recover(&self.state);
        state.idle.push(offscreen);
        self.ready.notify_one();
    }

    fn discard_renderer(&self) {
        let mut state = lock_recover(&self.state);
        state.total_renderers = state.total_renderers.saturating_sub(1);
        self.ready.notify_one();
    }
}

impl ThumbnailJobGate {
    pub(super) fn shared() -> &'static Self {
        THUMBNAIL_JOB_GATE.get_or_init(|| Self::new(default_thumbnail_job_capacity()))
    }

    pub(super) fn new(max_jobs: usize) -> Self {
        Self {
            inner: Arc::new(ThumbnailJobGateInner {
                state: Mutex::new(ThumbnailJobGateState::default()),
                ready: Condvar::new(),
                max_jobs: max_jobs.max(1),
            }),
        }
    }

    /// Acquire a job permit, waiting up to `timeout`. Returns `None` on timeout.
    ///
    /// Infallible with respect to lock poisoning: the gate counter is recovered
    /// rather than treated as fatal, so one panicking request cannot wedge the
    /// gate shut for the rest of a folder's thumbnails.
    #[cfg(test)]
    pub(super) fn acquire_timeout(&self, timeout: Duration) -> Option<ThumbnailJobPermit> {
        self.acquire_by(Instant::now() + timeout)
    }

    /// Acquires a permit by a caller-provided deadline.
    pub(super) fn acquire_by(&self, deadline: Instant) -> Option<ThumbnailJobPermit> {
        let mut state = lock_recover(&self.inner.state);
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiters.push_back(ticket);

        loop {
            if state.active_jobs < self.inner.max_jobs
                && state.waiters.front().copied() == Some(ticket)
            {
                let _ = state.waiters.pop_front();
                state.active_jobs += 1;
                self.inner.ready.notify_all();
                return Some(ThumbnailJobPermit {
                    gate: self.inner.clone(),
                });
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                state.remove_waiter(ticket);
                self.inner.ready.notify_all();
                return None;
            };

            let (next_state, _) = self
                .inner
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            state = next_state;
        }
    }
}

impl ThumbnailJobGateInner {
    fn release(&self) {
        let mut state = lock_recover(&self.state);
        state.active_jobs = state.active_jobs.saturating_sub(1);
        self.ready.notify_all();
    }
}

impl Drop for ThumbnailJobPermit {
    fn drop(&mut self) {
        self.gate.release();
    }
}

impl Drop for ThumbnailRendererPool {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            for offscreen in state.idle.drain(..) {
                std::mem::forget(offscreen);
            }
        }
    }
}

pub(super) struct ThumbnailRendererLease<'a> {
    pool: &'a ThumbnailRendererPool,
    pub(super) offscreen: Option<Offscreen>,
}

impl<'a> ThumbnailRendererLease<'a> {
    pub(super) fn new(pool: &'a ThumbnailRendererPool, offscreen: Offscreen) -> Self {
        Self {
            pool,
            offscreen: Some(offscreen),
        }
    }

    fn discard(mut self) {
        let _discarded = self.offscreen.take();
        self.pool.discard_renderer();
    }
}

impl Drop for ThumbnailRendererLease<'_> {
    fn drop(&mut self) {
        if let Some(offscreen) = self.offscreen.take() {
            self.pool.return_renderer(offscreen);
        }
    }
}

/// Create the pooled renderer ahead of the first request and park it idle.
///
/// Under Explorer's Apartment hosting every extraction serializes through one
/// host STA thread, so the very first `GetThumbnail` used to pay wgpu
/// instance + adapter + device + pipeline creation in line, in front of the
/// whole folder's queue. Prewarming from a background thread at class
/// activation overlaps that fixed cost with the shell's Initialize and
/// stream-copy phase. A failure is deliberately swallowed: the first real
/// request repeats the attempt and owns the error path, and the pool's
/// capacity accounting already tolerates a failed create.
pub(super) fn prewarm_renderer_pool() {
    let pool = ThumbnailRendererPool::shared();
    if let Ok(renderer) = pool.checkout_renderer() {
        drop(ThumbnailRendererLease::new(pool, renderer));
    }
}

fn default_thumbnail_job_capacity() -> usize {
    // Never more lanes than the machine has threads to run them on: the decode
    // inside a lane is itself parallel, so oversubscription buys queueing.
    std::thread::available_parallelism()
        .map_or(MAX_THUMBNAIL_JOB_LANES, |threads| {
            threads.get().min(MAX_THUMBNAIL_JOB_LANES)
        })
        .max(1)
}

const fn default_thumbnail_renderer_pool_size() -> usize {
    // Keep GPU context ownership independent from shell request fan-out. The
    // renderer pool serializes device-bound upload/readback while decode jobs
    // continue to prepare the next mesh in parallel.
    MAX_THUMBNAIL_RENDERERS
}

fn thumbnail_inflight() -> &'static Mutex<HashMap<ThumbnailRequestKey, Arc<InflightThumbnail>>> {
    THUMBNAIL_INFLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

impl InflightThumbnail {
    fn new() -> Self {
        Self {
            state: Mutex::new(InflightThumbnailState::Running),
            ready: Condvar::new(),
        }
    }
}

fn acquire_inflight_thumbnail(key: &ThumbnailRequestKey) -> InflightThumbnailLease {
    let mut inflight = lock_recover(thumbnail_inflight());
    if let Some(existing) = inflight.get(key) {
        return InflightThumbnailLease::Follower(existing.clone());
    }
    let entry = Arc::new(InflightThumbnail::new());
    inflight.insert(key.clone(), entry.clone());
    InflightThumbnailLease::Leader(entry)
}

fn finish_inflight_thumbnail(
    key: &ThumbnailRequestKey,
    entry: &Arc<InflightThumbnail>,
    result: InflightThumbnailResult,
) {
    {
        let mut state = lock_recover(&entry.state);
        *state = InflightThumbnailState::Finished(result);
        entry.ready.notify_all();
    }

    let mut inflight = lock_recover(thumbnail_inflight());
    if inflight
        .get(key)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        inflight.remove(key);
    }
}

fn wait_for_inflight_thumbnail(
    entry: &Arc<InflightThumbnail>,
    timeout: Duration,
) -> Option<InflightThumbnailResult> {
    let deadline = Instant::now() + timeout;
    let mut state = lock_recover(&entry.state);

    loop {
        match &*state {
            InflightThumbnailState::Finished(result) => return Some(result.clone()),
            InflightThumbnailState::Running => {
                let remaining = deadline.checked_duration_since(Instant::now())?;
                let (next_state, wait_result) = entry
                    .ready
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(PoisonError::into_inner);
                state = next_state;
                if wait_result.timed_out() && matches!(&*state, InflightThumbnailState::Running) {
                    return None;
                }
            }
        }
    }
}

pub(super) fn render_coalesced_thumbnail(
    key: ThumbnailRequestKey,
    timeout: Duration,
    render: impl FnOnce() -> ThumbnailAttempt,
) -> ThumbnailAttempt {
    // `render` is an infallible producer of a verdict: a full-size bitmap (a
    // real thumbnail or a deterministic placeholder) or an explicit transient
    // failure. Followers inherit the leader's verdict verbatim; a follower
    // that outwaits its budget reports transient failure rather than
    // duplicating the render or inventing a cacheable placeholder.
    match acquire_inflight_thumbnail(&key) {
        InflightThumbnailLease::Leader(entry) => {
            let attempt = panic::catch_unwind(AssertUnwindSafe(render)).unwrap_or_else(|_| {
                tracing::error!(
                    "thumbnail leader panicked outside the worker boundary; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            });
            let result = InflightThumbnailResult::from(attempt);
            finish_inflight_thumbnail(&key, &entry, result.clone());
            result.into()
        }
        InflightThumbnailLease::Follower(entry) => {
            if let Some(result) = wait_for_inflight_thumbnail(&entry, timeout) {
                result.into()
            } else {
                tracing::warn!(
                    ?timeout,
                    "waiting for an identical in-flight thumbnail timed out; reporting transient failure instead of duplicate render"
                );
                ThumbnailAttempt::TransientFailure
            }
        }
    }
}

/// request budget by waiting once for setup and again for rendering.
pub(super) fn run_thumbnail_job_with_deadline<T, F>(
    timeout: Duration,
    work: F,
) -> ThumbnailJobOutcome<T>
where
    T: Send + 'static,
    F: FnOnce(mpsc::SyncSender<ThumbnailJobProgress<T>>) + Send + 'static,
{
    // One budget for the whole request. Waiting for a slot and then rendering
    // each took the full timeout of their own, so a file could spend twice
    // what the caller asked for -- twelve seconds against six -- and under
    // Explorer's Apartment hosting that is twelve seconds the rest of the
    // folder spends queued behind it. The deadline is fixed here, once, and
    // both halves spend the same one.
    let deadline = Instant::now() + timeout;
    let Some(permit) = ThumbnailJobGate::shared().acquire_by(deadline) else {
        return ThumbnailJobOutcome::SetupTimedOut;
    };
    run_thumbnail_job_by(permit, deadline, work)
}

/// Variant of [`run_thumbnail_job_with_deadline`] for the Windows shell path,
/// which reserves a gate permit before it copies an `IStream`.
/// Run `work` on a worker thread against a deadline the caller already fixed,
/// so that a wait which has already happened is spent, not forgotten.
///
/// The permit is the caller's: the shell reserves one before it copies an
/// `IStream`, and a test hands one from a private gate so that its assertions
/// do not depend on what the rest of the suite is doing.
pub(super) fn run_thumbnail_job_by<T, F>(
    permit: ThumbnailJobPermit,
    deadline: Instant,
    work: F,
) -> ThumbnailJobOutcome<T>
where
    T: Send + 'static,
    F: FnOnce(mpsc::SyncSender<ThumbnailJobProgress<T>>) + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel(2);
    let timed_out = Arc::new(AtomicBool::new(false));
    let timed_out_worker = timed_out.clone();
    let spawn = std::thread::Builder::new()
        .name("occluview-thumbnail-job".to_string())
        .spawn(move || {
            // Keep the permit with the worker after the caller times out. The
            // decode/readback can not be cancelled safely, and releasing the
            // slot early would let a large folder create unbounded survivors.
            let _permit = permit;
            let _ = panic::catch_unwind(AssertUnwindSafe(|| work(tx)));
            if timed_out_worker.load(Ordering::Relaxed) {
                tracing::debug!(
                    "thumbnail worker completed after caller timed out; releasing its burst slot"
                );
            }
        });
    if spawn.is_err() {
        return ThumbnailJobOutcome::Failed;
    }

    let mut prepared = false;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            timed_out.store(true, Ordering::Relaxed);
            return if prepared {
                ThumbnailJobOutcome::RenderTimedOut
            } else {
                ThumbnailJobOutcome::SetupTimedOut
            };
        };
        match rx.recv_timeout(remaining) {
            Ok(ThumbnailJobProgress::Prepared) => prepared = true,
            Ok(ThumbnailJobProgress::Finished(value)) => {
                return ThumbnailJobOutcome::Finished(value)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                timed_out.store(true, Ordering::Relaxed);
                return if prepared {
                    ThumbnailJobOutcome::RenderTimedOut
                } else {
                    ThumbnailJobOutcome::SetupTimedOut
                };
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return ThumbnailJobOutcome::Failed,
        }
    }
}

#[cfg(test)]
mod recovery_tests;
