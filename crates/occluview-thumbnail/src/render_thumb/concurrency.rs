// Poison-recovery tests deliberately panic while holding a lock to poison it,
// then assert the pool/gate recover; that needs unwrap/expect/panic in test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

use super::{
    Duration, Mutex, ThumbnailError, ThumbnailRequestKey, THUMBNAIL_INFLIGHT, THUMBNAIL_JOB_GATE,
    THUMBNAIL_RENDERER_POOL,
};
use crate::offscreen_factory::create_thumbnail_offscreen;
use occluview_render::Offscreen;
use std::collections::{HashMap, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, MutexGuard, PoisonError};
use std::time::Instant;

// A shell surrogate is a shared, memory-constrained host, not a render farm.
// Explorer can ask for twelve thumbnails at once, so keep that many bounded
// decode jobs in flight. GPU work has a separate, much smaller budget: every
// Offscreen owns a wgpu device, and creating several D3D devices concurrently
// makes the driver serialize or contend instead of making thumbnails faster.
const MAX_THUMBNAIL_JOB_LANES: usize = 12;
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
    Finished(Vec<u8>),
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
        Self {
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

        match f(offscreen) {
            Ok(value) => Ok(value),
            Err(error) => {
                // A renderer that reported a GPU/readback error may have a
                // lost device or stale backend state. Never hand it to the
                // next Explorer request; release the pool capacity instead.
                lease.discard();
                Err(error)
            }
        }
    }

    pub(super) fn checkout_renderer(&self) -> Result<Offscreen, ThumbnailError> {
        loop {
            let mut state = lock_recover(&self.state);
            if let Some(offscreen) = state.idle.pop() {
                return Ok(offscreen);
            }
            if state.total_renderers < self.max_renderers {
                state.total_renderers += 1;
                drop(state);
                match create_thumbnail_offscreen() {
                    Ok(offscreen) => return Ok(offscreen),
                    Err(error) => {
                        let mut state = lock_recover(&self.state);
                        state.total_renderers = state.total_renderers.saturating_sub(1);
                        self.ready.notify_one();
                        return Err(error);
                    }
                }
            }
            let _guard = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
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
    pub(super) fn acquire_timeout(&self, timeout: Duration) -> Option<ThumbnailJobPermit> {
        let deadline = Instant::now() + timeout;
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

const fn default_thumbnail_job_capacity() -> usize {
    MAX_THUMBNAIL_JOB_LANES
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
    pixels: &[u8],
) {
    {
        let mut state = lock_recover(&entry.state);
        *state = InflightThumbnailState::Finished(pixels.to_vec());
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
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut state = lock_recover(&entry.state);

    loop {
        match &*state {
            InflightThumbnailState::Finished(pixels) => return Some(pixels.clone()),
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
    render: impl FnOnce() -> Vec<u8>,
    follower_fallback: impl FnOnce() -> Vec<u8>,
) -> Vec<u8> {
    // Both `render` and `follower_fallback` are infallible producers of a
    // full-size bitmap (a real thumbnail or a placeholder), so every arm here
    // returns real pixels — this function never yields an empty buffer, which
    // is what "never show nothing in the folder" depends on downstream.
    match acquire_inflight_thumbnail(&key) {
        InflightThumbnailLease::Leader(entry) => {
            let pixels = panic::catch_unwind(AssertUnwindSafe(render)).unwrap_or_else(|_| {
                tracing::error!(
                    "thumbnail leader panicked outside the worker boundary; returning a placeholder"
                );
                follower_fallback()
            });
            finish_inflight_thumbnail(&key, &entry, &pixels);
            pixels
        }
        InflightThumbnailLease::Follower(entry) => {
            if let Some(pixels) = wait_for_inflight_thumbnail(&entry, timeout) {
                pixels
            } else {
                tracing::warn!(
                    ?timeout,
                    "waiting for an identical in-flight thumbnail timed out; returning fallback instead of duplicate render"
                );
                follower_fallback()
            }
        }
    }
}

#[cfg(test)]
pub(super) fn run_thumbnail_job_with_gate_and_timeouts<T, F>(
    gate: &ThumbnailJobGate,
    setup_timeout: Duration,
    render_timeout: Duration,
    work: F,
) -> ThumbnailJobOutcome<T>
where
    T: Send + 'static,
    F: FnOnce(mpsc::SyncSender<ThumbnailJobProgress<T>>) + Send + 'static,
{
    let Some(permit) = gate.acquire_timeout(setup_timeout) else {
        return ThumbnailJobOutcome::SetupTimedOut;
    };
    run_thumbnail_job_with_permit(permit, setup_timeout, render_timeout, work)
}

#[cfg(test)]
pub(super) fn run_thumbnail_job_with_permit<T, F>(
    permit: ThumbnailJobPermit,
    setup_timeout: Duration,
    render_timeout: Duration,
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
            // The worker, not its waiting caller, owns process capacity. A
            // timeout cannot cancel Rust parsing or a submitted GPU readback;
            // releasing this permit early would let every subsequent Explorer
            // request create another surviving worker.
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
        let timeout = if prepared {
            render_timeout
        } else {
            setup_timeout
        };
        match rx.recv_timeout(timeout) {
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

/// Run one thumbnail job against a single wall-clock budget. Queueing for the
/// bounded decode slot, format loading, renderer checkout, and GPU readback all
/// consume the same deadline so a mixed Explorer folder cannot multiply the
/// request budget by waiting once for setup and again for rendering.
pub(super) fn run_thumbnail_job_with_deadline<T, F>(
    timeout: Duration,
    work: F,
) -> ThumbnailJobOutcome<T>
where
    T: Send + 'static,
    F: FnOnce(mpsc::SyncSender<ThumbnailJobProgress<T>>) + Send + 'static,
{
    let Some(permit) = ThumbnailJobGate::shared().acquire_timeout(timeout) else {
        return ThumbnailJobOutcome::SetupTimedOut;
    };
    run_thumbnail_job_with_permit_deadline(permit, timeout, work)
}

/// Variant of [`run_thumbnail_job_with_deadline`] for the Windows shell path,
/// which reserves a gate permit before it copies an `IStream`.
pub(super) fn run_thumbnail_job_with_permit_deadline<T, F>(
    permit: ThumbnailJobPermit,
    timeout: Duration,
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

    let deadline = Instant::now() + timeout;
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
mod poison_recovery_tests {
    use super::*;

    fn wait_for_queued_jobs(gate: &ThumbnailJobGate, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if lock_recover(&gate.inner.state).waiters.len() == expected {
                return;
            }
            assert!(Instant::now() < deadline, "thumbnail job did not queue");
            std::thread::yield_now();
        }
    }

    /// Panic while holding `mutex`'s guard, poisoning it, then hand the
    /// (recovered) inner value back.
    fn poison<T>(mutex: &Mutex<T>) {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("fresh mutex is not poisoned");
            panic!("intentionally poison the lock for the recovery test");
        }));
        assert!(mutex.is_poisoned(), "the lock should now be poisoned");
    }

    #[test]
    fn lock_recover_returns_the_guard_even_when_poisoned() {
        let mutex = Mutex::new(41u32);
        {
            let mut guard = lock_recover(&mutex);
            *guard += 1;
        }
        poison(&mutex);
        // Recovery still yields a usable guard with the last-written value.
        let mut guard = lock_recover(&mutex);
        assert_eq!(*guard, 42);
        *guard = 7;
        assert_eq!(*guard, 7);
    }

    #[test]
    fn shell_renderer_parallelism_stays_inside_the_process_budget() {
        assert_eq!(default_thumbnail_renderer_pool_size(), 1);
        assert_eq!(default_thumbnail_job_capacity(), 12);
    }

    #[test]
    fn poisoned_job_gate_still_acquires_and_releases() {
        // One panicking request must not wedge the gate shut for every other
        // file in the folder (the mixed-folder "no thumbnails at all" bug).
        let gate = ThumbnailJobGate::new(1);
        poison(&gate.inner.state);

        let permit = gate.acquire_timeout(Duration::from_millis(50));
        assert!(
            permit.is_some(),
            "a poisoned gate must still hand out permits"
        );
        // Dropping the permit must release it despite the poison, so the single
        // slot is reusable rather than leaked forever.
        drop(permit);
        let again = gate.acquire_timeout(Duration::from_millis(50));
        assert!(
            again.is_some(),
            "release must not skip on poison, or the gate leaks its only permit"
        );
    }

    #[test]
    fn queued_thumbnail_jobs_acquire_in_arrival_order() {
        let gate = Arc::new(ThumbnailJobGate::new(1));
        let held = gate
            .acquire_timeout(Duration::from_millis(10))
            .expect("initial permit");
        let (order_tx, order_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();

        let first_gate = gate.clone();
        let first_tx = order_tx.clone();
        let first = std::thread::spawn(move || {
            let _permit = first_gate
                .acquire_timeout(Duration::from_secs(1))
                .expect("first queued permit");
            first_tx.send(1_u8).expect("record first acquisition");
            release_first_rx.recv().expect("release first waiter");
        });
        wait_for_queued_jobs(&gate, 1);

        let second_gate = gate.clone();
        let second = std::thread::spawn(move || {
            let _permit = second_gate
                .acquire_timeout(Duration::from_secs(1))
                .expect("second queued permit");
            order_tx.send(2_u8).expect("record second acquisition");
        });
        wait_for_queued_jobs(&gate, 2);

        drop(held);
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)), Ok(1));
        assert!(order_rx.recv_timeout(Duration::from_millis(20)).is_err());
        release_first_tx.send(()).expect("release first waiter");
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)), Ok(2));
        first.join().expect("first waiter thread");
        second.join().expect("second waiter thread");
    }

    #[test]
    fn poisoned_renderer_pool_still_serves_and_returns_renderers() {
        let _guard = crate::acquire_render_test_guard();
        let pool = ThumbnailRendererPool::new(2);
        poison(&pool.state);

        // A poisoned pool must still create/serve a renderer instead of failing
        // every checkout for the rest of the process.
        let renderer = pool
            .checkout_renderer()
            .expect("a poisoned renderer pool must still serve a renderer");
        {
            let lease = ThumbnailRendererLease::new(&pool, renderer);
            drop(lease);
        }
        // The returned renderer landed back in the idle set (return_renderer did
        // not skip on poison), so it is reusable.
        let idle = lock_recover(&pool.state).idle.len();
        assert_eq!(
            idle, 1,
            "the returned renderer must be reusable, not leaked, after poison"
        );
    }

    #[test]
    fn discarded_renderer_releases_pool_capacity() {
        let pool = ThumbnailRendererPool::new(2);
        {
            let mut state = lock_recover(&pool.state);
            state.total_renderers = 1;
        }

        pool.discard_renderer();

        assert_eq!(lock_recover(&pool.state).total_renderers, 0);
    }
}
