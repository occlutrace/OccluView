use super::state::{BridgeSplitGuard, BridgeSplitToolError};
#[cfg(test)]
use super::state::BridgeSplitTarget;
use glam::Affine3A;
use occluview_core::Mesh;
use occluview_mesh_edit::{
    bridge_split_prepared_mesh_in_world, prepare_bridge_split_source, BridgeSplitRequest,
    CoreBridgeSplitResult, PreparedBridgeSplitSource,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, Weak};
#[cfg(not(test))]
use std::sync::OnceLock;
use std::thread;

const SOURCE_CACHE_CAPACITY: usize = 8;
const MAX_ACTIVE_BRIDGE_JOBS: usize = 2;

#[derive(Debug)]
pub(crate) struct BridgeSplitJobInput {
    pub(crate) mesh: Arc<Mesh>,
    pub(crate) transform: Affine3A,
    pub(crate) request: BridgeSplitRequest,
    pub(crate) guard: BridgeSplitGuard,
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeSplitJobOutput {
    pub(crate) guard: BridgeSplitGuard,
    pub(crate) result: Result<CoreBridgeSplitResult, BridgeSplitToolError>,
}

#[derive(Default)]
struct BridgeSplitSourceCache {
    entries: BTreeMap<u64, Arc<PreparedBridgeSplitSource>>,
    recency: VecDeque<u64>,
}

impl BridgeSplitSourceCache {
    fn prepared_source(
        cache: &Mutex<Self>,
        source: &Arc<Mesh>,
        prepare: impl FnOnce(
            Arc<Mesh>,
        ) -> Result<
            PreparedBridgeSplitSource,
            occluview_mesh_edit::CoreBridgeSplitError,
        >,
    ) -> Result<Arc<PreparedBridgeSplitSource>, BridgeSplitToolError> {
        // Keyed by geometry_id, not topology_id: an interactive sculpt commit
        // preserves topology_id (to spare the renderer a re-upload) while its
        // positions change, so keying on topology_id would hand back a stale
        // pre-sculpt prepared solid. geometry_id changes on every geometry edit.
        let geometry_id = source.geometry_id();
        {
            let mut cache = cache
                .lock()
                .map_err(|_| BridgeSplitToolError::WorkerStopped)?;
            if let Some(prepared) = cache.entries.get(&geometry_id) {
                let prepared = Arc::clone(prepared);
                cache.touch(geometry_id);
                return Ok(prepared);
            }
        }

        let prepared = Arc::new(prepare(Arc::clone(source)).map_err(BridgeSplitToolError::from)?);
        let mut cache = cache
            .lock()
            .map_err(|_| BridgeSplitToolError::WorkerStopped)?;
        if let Some(existing) = cache.entries.get(&geometry_id) {
            let existing = Arc::clone(existing);
            cache.touch(geometry_id);
            return Ok(existing);
        }
        cache.entries.insert(geometry_id, Arc::clone(&prepared));
        cache.touch(geometry_id);
        while cache.recency.len() > SOURCE_CACHE_CAPACITY {
            if let Some(evicted) = cache.recency.pop_front() {
                cache.entries.remove(&evicted);
            }
        }
        Ok(prepared)
    }

    fn touch(&mut self, geometry_id: u64) {
        if let Some(position) = self.recency.iter().position(|&id| id == geometry_id) {
            self.recency.remove(position);
        }
        self.recency.push_back(geometry_id);
    }
}

type BridgeSplitCompute = dyn Fn(BridgeSplitJobInput) -> Result<CoreBridgeSplitResult, BridgeSplitToolError>
    + Send
    + Sync;
type BridgeSplitWarm = dyn Fn(Arc<Mesh>) + Send + Sync;

struct PendingPreview {
    token: Arc<()>,
    input: BridgeSplitJobInput,
}

enum PendingWork {
    Preview(PendingPreview),
    Warm {
        geometry_id: u64,
        mesh: Arc<Mesh>,
        after_warm: Option<PendingPreview>,
    },
}

impl PendingWork {
    fn preview(&self) -> Option<&PendingPreview> {
        match self {
            Self::Preview(preview) => Some(preview),
            Self::Warm { after_warm, .. } => after_warm.as_ref(),
        }
    }
}

#[derive(Clone)]
enum ActiveWork {
    Preview {
        token: Arc<()>,
        guard: BridgeSplitGuard,
    },
    Warm {
        geometry_id: u64,
    },
}

enum ScheduledWork {
    Preview(PendingPreview),
    Warm { geometry_id: u64, mesh: Arc<Mesh> },
}

impl ScheduledWork {
    fn active_work(&self) -> ActiveWork {
        match self {
            Self::Preview(preview) => ActiveWork::Preview {
                token: Arc::clone(&preview.token),
                guard: preview.input.guard,
            },
            Self::Warm { geometry_id, .. } => ActiveWork::Warm {
                geometry_id: *geometry_id,
            },
        }
    }
}

#[derive(Default)]
struct WorkerMailboxState {
    current_token: Option<Arc<()>>,
    active: Vec<ActiveWork>,
    pending: Option<PendingWork>,
    warm_target: Option<u64>,
    warming: BTreeSet<u64>,
    warmed: Option<u64>,
    output: Option<BridgeSplitJobOutput>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

struct WorkerMailbox {
    id: u64,
    state: Mutex<WorkerMailboxState>,
    compute: Arc<BridgeSplitCompute>,
    warm: Arc<BridgeSplitWarm>,
}

#[derive(Default)]
struct SchedulerState {
    // Registry entries are weak and are removed by worker Drop. `ready` carries
    // worker IDs only, with `queued` enforcing one queue entry per live worker.
    workers: BTreeMap<u64, Weak<WorkerMailbox>>,
    ready: VecDeque<u64>,
    queued: BTreeSet<u64>,
    // Reservations cover preview and warm jobs from dispatch until completion.
    active: usize,
    next_worker_id: u64,
}

#[derive(Default)]
struct BridgeSplitScheduler {
    // Mutating paths lock this first and a worker mailbox second. Mailbox-only
    // reads never acquire the scheduler afterwards.
    state: Mutex<SchedulerState>,
}

#[cfg(not(test))]
static BRIDGE_SPLIT_SCHEDULER: OnceLock<Arc<BridgeSplitScheduler>> = OnceLock::new();

#[cfg(not(test))]
fn shared_scheduler() -> Arc<BridgeSplitScheduler> {
    Arc::clone(BRIDGE_SPLIT_SCHEDULER.get_or_init(|| Arc::new(BridgeSplitScheduler::default())))
}

fn injected_compute_scheduler() -> Arc<BridgeSplitScheduler> {
    #[cfg(test)]
    {
        Arc::new(BridgeSplitScheduler::default())
    }
    #[cfg(not(test))]
    {
        shared_scheduler()
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct BridgeSplitWorker {
    scheduler: Arc<BridgeSplitScheduler>,
    mailbox: Arc<WorkerMailbox>,
}

impl Default for BridgeSplitWorker {
    fn default() -> Self {
        Self::spawn()
    }
}

impl BridgeSplitWorker {
    pub(crate) fn spawn() -> Self {
        let source_cache = Arc::new(Mutex::new(BridgeSplitSourceCache::default()));
        let compute = move |input: BridgeSplitJobInput| {
            let prepared = BridgeSplitSourceCache::prepared_source(
                &source_cache,
                &input.mesh,
                prepare_bridge_split_source,
            )?;
            bridge_split_prepared_mesh_in_world(&prepared, input.transform, input.request)
                .map_err(BridgeSplitToolError::from)
        };
        Self::spawn_with_compute(compute)
    }

    pub(crate) fn spawn_with_compute<F>(compute: F) -> Self
    where
        F: Fn(BridgeSplitJobInput) -> Result<CoreBridgeSplitResult, BridgeSplitToolError>
            + Send
            + Sync
            + 'static,
    {
        Self::with_parts(
            Arc::new(compute),
            Arc::new(|mesh| mesh.warm_bvh()),
            injected_compute_scheduler(),
        )
    }

    fn with_parts(
        compute: Arc<BridgeSplitCompute>,
        warm: Arc<BridgeSplitWarm>,
        scheduler: Arc<BridgeSplitScheduler>,
    ) -> Self {
        let mut scheduler_state = lock_recover(&scheduler.state);
        let id = loop {
            scheduler_state.next_worker_id = scheduler_state.next_worker_id.wrapping_add(1);
            if scheduler_state.next_worker_id != 0
                && !scheduler_state.workers.contains_key(&scheduler_state.next_worker_id)
            {
                break scheduler_state.next_worker_id;
            }
        };
        let mailbox = Arc::new(WorkerMailbox {
            id,
            state: Mutex::new(WorkerMailboxState::default()),
            compute,
            warm,
        });
        scheduler_state.workers.insert(id, Arc::downgrade(&mailbox));
        drop(scheduler_state);
        Self { scheduler, mailbox }
    }

    /// Queue picking-tree warming through the same bounded scheduler as previews.
    pub(crate) fn warm_source(&self, mesh: Arc<Mesh>, wake: Arc<dyn Fn() + Send + Sync>) {
        let geometry_id = mesh.geometry_id();
        let mut scheduler = lock_recover(&self.scheduler.state);
        {
            let mut state = lock_recover(&self.mailbox.state);
            state.wake = Some(wake);
            state.warm_target = Some(geometry_id);
            let after_warm = state.pending.take().and_then(|pending| match pending {
                PendingWork::Preview(preview)
                    if preview.input.mesh.geometry_id() == geometry_id =>
                {
                    Some(preview)
                }
                PendingWork::Warm {
                    geometry_id: pending_id,
                    after_warm,
                    ..
                } if pending_id == geometry_id => after_warm,
                _ => None,
            });
            if state.warmed == Some(geometry_id) || state.warming.contains(&geometry_id) {
                state.pending = after_warm.map(PendingWork::Preview);
            } else {
                state.pending = Some(PendingWork::Warm {
                    geometry_id,
                    mesh,
                    after_warm,
                });
            }
        }
        enqueue_if_ready(&mut scheduler, &self.mailbox);
        let mut wakes = Vec::new();
        pump_scheduler(&self.scheduler, &mut scheduler, &mut wakes);
        drop(scheduler);
        invoke_wakes(wakes);
    }

    #[cfg(test)]
    fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        let _scheduler = lock_recover(&self.scheduler.state);
        lock_recover(&self.mailbox.state).wake = Some(wake);
    }

    pub(crate) fn submit(
        &mut self,
        input: BridgeSplitJobInput,
    ) -> Result<(), BridgeSplitToolError> {
        let mut scheduler = lock_recover(&self.scheduler.state);
        {
            let mut state = lock_recover(&self.mailbox.state);
            let token = Arc::new(());
            state.current_token = Some(Arc::clone(&token));
            state.output = None;
            let geometry_id = input.mesh.geometry_id();
            let preview = PendingPreview { token, input };
            state.pending = match state.pending.take() {
                Some(PendingWork::Warm {
                    geometry_id: warm_id,
                    mesh,
                    ..
                }) if warm_id == geometry_id => Some(PendingWork::Warm {
                    geometry_id: warm_id,
                    mesh,
                    after_warm: Some(preview),
                }),
                _ => Some(PendingWork::Preview(preview)),
            };
        }
        enqueue_if_ready(&mut scheduler, &self.mailbox);
        let mut wakes = Vec::new();
        pump_scheduler(&self.scheduler, &mut scheduler, &mut wakes);
        drop(scheduler);
        invoke_wakes(wakes);
        Ok(())
    }

    /// Invalidate this session and release all work that has not started.
    /// Running work keeps its process slot and input until the compute returns.
    pub(crate) fn cancel(&mut self) {
        let mut scheduler = lock_recover(&self.scheduler.state);
        {
            let mut state = lock_recover(&self.mailbox.state);
            state.current_token = None;
            state.pending = None;
            state.warm_target = None;
            state.output = None;
            state.wake = None;
        }
        remove_ready(&mut scheduler, self.mailbox.id);
        let mut wakes = Vec::new();
        pump_scheduler(&self.scheduler, &mut scheduler, &mut wakes);
        drop(scheduler);
        invoke_wakes(wakes);
    }

    pub(crate) fn poll(&mut self) -> Vec<BridgeSplitJobOutput> {
        lock_recover(&self.mailbox.state)
            .output
            .take()
            .into_iter()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn active_guard(&self) -> Option<BridgeSplitGuard> {
        lock_recover(&self.mailbox.state)
            .active
            .iter()
            .find_map(|work| match work {
                ActiveWork::Preview { guard, .. } => Some(*guard),
                ActiveWork::Warm { .. } => None,
            })
    }

    #[cfg(test)]
    pub(crate) fn queued_guard(&self) -> Option<BridgeSplitGuard> {
        lock_recover(&self.mailbox.state)
            .pending
            .as_ref()
            .and_then(PendingWork::preview)
            .map(|preview| preview.input.guard)
    }
}

impl Drop for BridgeSplitWorker {
    fn drop(&mut self) {
        let mut scheduler = lock_recover(&self.scheduler.state);
        if let Some(mailbox) = scheduler
            .workers
            .remove(&self.mailbox.id)
            .and_then(|mailbox| mailbox.upgrade())
        {
            let mut state = lock_recover(&mailbox.state);
            state.current_token = None;
            state.pending = None;
            state.warm_target = None;
            state.output = None;
            state.wake = None;
        }
        remove_ready(&mut scheduler, self.mailbox.id);
        let mut wakes = Vec::new();
        pump_scheduler(&self.scheduler, &mut scheduler, &mut wakes);
        drop(scheduler);
        invoke_wakes(wakes);
    }
}

impl BridgeSplitScheduler {
    fn finish(
        self: &Arc<Self>,
        worker_id: u64,
        active: ActiveWork,
        result: Option<Result<CoreBridgeSplitResult, BridgeSplitToolError>>,
    ) {
        let mut scheduler = lock_recover(&self.state);
        scheduler.active = scheduler.active.saturating_sub(1);
        let mut wakes = Vec::new();
        if let Some(mailbox) = scheduler
            .workers
            .get(&worker_id)
            .and_then(Weak::upgrade)
        {
            {
                let mut state = lock_recover(&mailbox.state);
                state.active.retain(|candidate| !same_active(candidate, &active));
                match active {
                    ActiveWork::Preview { token, guard } => {
                        if state
                            .current_token
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &token))
                        {
                            state.output = Some(BridgeSplitJobOutput {
                                guard,
                                result: result.unwrap_or(Err(BridgeSplitToolError::WorkerStopped)),
                            });
                            if let Some(wake) = state.wake.as_ref() {
                                wakes.push(Arc::clone(wake));
                            }
                        }
                    }
                    ActiveWork::Warm { geometry_id } => {
                        state.warming.remove(&geometry_id);
                        if state.warm_target == Some(geometry_id) {
                            state.warmed = Some(geometry_id);
                        }
                        if let Some(wake) = state.wake.as_ref() {
                            wakes.push(Arc::clone(wake));
                        }
                    }
                }
            }
            enqueue_if_ready(&mut scheduler, &mailbox);
        }
        pump_scheduler(self, &mut scheduler, &mut wakes);
        drop(scheduler);
        invoke_wakes(wakes);
    }
}

fn same_active(left: &ActiveWork, right: &ActiveWork) -> bool {
    match (left, right) {
        (ActiveWork::Preview { token: left, .. }, ActiveWork::Preview { token: right, .. }) => {
            Arc::ptr_eq(left, right)
        }
        (ActiveWork::Warm { geometry_id: left }, ActiveWork::Warm { geometry_id: right }) => {
            left == right
        }
        _ => false,
    }
}

fn remove_ready(scheduler: &mut SchedulerState, worker_id: u64) {
    if scheduler.queued.remove(&worker_id) {
        scheduler.ready.retain(|queued_id| *queued_id != worker_id);
    }
}

fn enqueue_if_ready(scheduler: &mut SchedulerState, mailbox: &WorkerMailbox) {
    let state = lock_recover(&mailbox.state);
    let ready = match state.pending.as_ref() {
        Some(PendingWork::Warm { geometry_id, .. }) => {
            state.warmed != Some(*geometry_id) && !state.warming.contains(geometry_id)
        }
        Some(PendingWork::Preview(preview)) => preview_is_ready(&state, preview),
        None => false,
    };
    drop(state);
    if ready && scheduler.queued.insert(mailbox.id) {
        scheduler.ready.push_back(mailbox.id);
    }
}

fn preview_is_ready(state: &WorkerMailboxState, preview: &PendingPreview) -> bool {
    let geometry_id = preview.input.mesh.geometry_id();
    let waiting_for_warm =
        state.warm_target == Some(geometry_id) && state.warmed != Some(geometry_id);
    let same_session_active = state.active.iter().any(|work| {
        matches!(
            work,
            ActiveWork::Preview { guard, .. }
                if guard.session_id == preview.input.guard.session_id
        )
    });
    !waiting_for_warm && !state.warming.contains(&geometry_id) && !same_session_active
}

fn take_ready_work(state: &mut WorkerMailboxState) -> Option<ScheduledWork> {
    match state.pending.take()? {
        PendingWork::Warm {
            geometry_id,
            mesh,
            after_warm,
        } => {
            if state.warmed == Some(geometry_id) || state.warming.contains(&geometry_id) {
                state.pending = after_warm.map(PendingWork::Preview);
                return take_ready_work(state);
            }
            state.pending = after_warm.map(PendingWork::Preview);
            state.warming.insert(geometry_id);
            Some(ScheduledWork::Warm { geometry_id, mesh })
        }
        PendingWork::Preview(preview) => {
            if !preview_is_ready(state, &preview) {
                state.pending = Some(PendingWork::Preview(preview));
                return None;
            }
            Some(ScheduledWork::Preview(preview))
        }
    }
}

fn launch_work(
    scheduler: &Arc<BridgeSplitScheduler>,
    mailbox: &Arc<WorkerMailbox>,
    work: ScheduledWork,
) -> Result<(), std::io::Error> {
    let scheduler = Arc::clone(scheduler);
    let worker_id = mailbox.id;
    let active = work.active_work();
    let compute = Arc::clone(&mailbox.compute);
    let warm = Arc::clone(&mailbox.warm);
    let _thread = thread::Builder::new()
        .name("bridge-split".to_string())
        .spawn(move || match work {
            ScheduledWork::Preview(preview) => {
                let result = catch_unwind(AssertUnwindSafe(|| (compute)(preview.input)))
                    .unwrap_or(Err(BridgeSplitToolError::WorkerStopped));
                scheduler.finish(worker_id, active, Some(result));
            }
            ScheduledWork::Warm { mesh, .. } => {
                let _ = catch_unwind(AssertUnwindSafe(|| (warm)(mesh)));
                scheduler.finish(worker_id, active, None);
            }
        })?;
    Ok(())
}

fn pump_scheduler(
    scheduler_arc: &Arc<BridgeSplitScheduler>,
    scheduler: &mut SchedulerState,
    wakes: &mut Vec<Arc<dyn Fn() + Send + Sync>>,
) {
    while scheduler.active < MAX_ACTIVE_BRIDGE_JOBS {
        let Some(worker_id) = scheduler.ready.pop_front() else {
            break;
        };
        scheduler.queued.remove(&worker_id);
        let Some(mailbox) = scheduler.workers.get(&worker_id).and_then(Weak::upgrade) else {
            scheduler.workers.remove(&worker_id);
            continue;
        };
        let work = take_ready_work(&mut lock_recover(&mailbox.state));
        let Some(work) = work else {
            continue;
        };
        let active = work.active_work();
        lock_recover(&mailbox.state).active.push(active.clone());
        scheduler.active += 1;
        if launch_work(scheduler_arc, &mailbox, work).is_err() {
            scheduler.active = scheduler.active.saturating_sub(1);
            {
                let mut state = lock_recover(&mailbox.state);
                state.active.retain(|candidate| !same_active(candidate, &active));
                match active {
                    ActiveWork::Preview { token, guard } => {
                        if state
                            .current_token
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &token))
                        {
                            state.output = Some(BridgeSplitJobOutput {
                                guard,
                                result: Err(BridgeSplitToolError::WorkerStopped),
                            });
                            if let Some(wake) = state.wake.as_ref() {
                                wakes.push(Arc::clone(wake));
                            }
                        }
                    }
                    ActiveWork::Warm { geometry_id } => {
                        state.warming.remove(&geometry_id);
                        if state.warm_target == Some(geometry_id) {
                            state.warmed = Some(geometry_id);
                        }
                        if let Some(wake) = state.wake.as_ref() {
                            wakes.push(Arc::clone(wake));
                        }
                    }
                }
            }
        }
        enqueue_if_ready(scheduler, &mailbox);
    }
}

fn invoke_wakes(wakes: Vec<Arc<dyn Fn() + Send + Sync>>) {
    for wake in wakes {
        let _ = catch_unwind(AssertUnwindSafe(|| wake()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use occluview_core::Vertex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    fn isolated_scheduler() -> Arc<BridgeSplitScheduler> {
        Arc::new(BridgeSplitScheduler::default())
    }

    fn test_input(
        session_id: u64,
        generation: u64,
        entry: &occluview_core::SceneMesh,
    ) -> BridgeSplitJobInput {
        BridgeSplitJobInput {
            mesh: Arc::clone(&entry.mesh),
            transform: Affine3A::IDENTITY,
            request: BridgeSplitRequest {
                center: Vec3::ZERO,
                normal: Vec3::X,
                kerf_mm: 0.05,
                disc_radius_mm: 5.0,
                max_disc_radius_mm: 10.0,
            },
            guard: BridgeSplitGuard {
                session_id,
                generation,
                target: BridgeSplitTarget::capture(entry),
            },
        }
    }

    fn controlled_worker(
        scheduler: Arc<BridgeSplitScheduler>,
        started: mpsc::Sender<(u64, u64, usize)>,
        release: mpsc::Receiver<()>,
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
    ) -> BridgeSplitWorker {
        let release = Arc::new(Mutex::new(release));
        let compute = move |input: BridgeSplitJobInput| {
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now_active, Ordering::SeqCst);
            struct ActiveCounter(Arc<AtomicUsize>);
            impl Drop for ActiveCounter {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _counter = ActiveCounter(Arc::clone(&active));
            let _ = started.send((
                input.guard.session_id,
                input.guard.generation,
                now_active,
            ));
            if let Ok(receiver) = release.lock() {
                let _ = receiver.recv();
            }
            Err(BridgeSplitToolError::WorkerStopped)
        };
        BridgeSplitWorker::with_parts(Arc::new(compute), Arc::new(|_mesh| {}), scheduler)
    }

    #[allow(clippy::expect_used)]
    #[test]
    fn source_cache_reuses_each_prepared_source_across_source_switches() {
        let source = Arc::new(mesh_with_redundant_degenerate_face("A"));
        let alternate = Arc::new(mesh_with_redundant_degenerate_face("B"));
        let cache = Mutex::new(BridgeSplitSourceCache::default());

        let first =
            BridgeSplitSourceCache::prepared_source(&cache, &source, prepare_bridge_split_source)
                .expect("prepared source");
        let other = BridgeSplitSourceCache::prepared_source(
            &cache,
            &alternate,
            prepare_bridge_split_source,
        )
        .expect("alternate source");
        let second =
            BridgeSplitSourceCache::prepared_source(&cache, &source, prepare_bridge_split_source)
                .expect("cached source");

        assert!(!Arc::ptr_eq(&first, &other));
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[allow(clippy::expect_used)]
    #[test]
    fn source_cache_remembers_open_surface_preparation() {
        let source = Arc::new(open_triangle("surface"));
        let geometry_id = source.geometry_id();
        let cache = Mutex::new(BridgeSplitSourceCache::default());

        BridgeSplitSourceCache::prepared_source(&cache, &source, prepare_bridge_split_source)
            .expect("open surface is eligible for the surface fallback");
        let cache = cache.lock().expect("cache");
        assert!(cache.entries.contains_key(&geometry_id));
        assert!(cache.recency.contains(&geometry_id));
    }

    #[test]
    fn process_admission_caps_jobs_and_keeps_only_latest_pending_per_worker() {
        let timeout = Duration::from_secs(5);
        let scheduler = isolated_scheduler();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (release_second_tx, release_second_rx) = mpsc::channel();
        let (release_waiting_tx, release_waiting_rx) = mpsc::channel();
        let (release_fair_tx, release_fair_rx) = mpsc::channel();
        let mut first = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_first_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let mut second = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_second_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let mut waiting = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_waiting_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let mut fair = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_fair_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let first_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let second_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let waiting_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let fair_entry = occluview_core::SceneMesh::new(Mesh::empty());

        let _ = first.submit(test_input(1, 1, &first_entry));
        let first_started = started_rx.recv_timeout(timeout);
        let _ = second.submit(test_input(2, 1, &second_entry));
        let second_started = started_rx.recv_timeout(timeout);
        let _ = waiting.submit(test_input(3, 1, &waiting_entry));
        let _ = waiting.submit(test_input(3, 2, &waiting_entry));
        let _ = waiting.submit(test_input(3, 3, &waiting_entry));
        let _ = fair.submit(test_input(4, 1, &fair_entry));
        let early_start = started_rx.try_recv().ok();
        let (reserved_slots, queue_len, live_workers, queued_guard) = {
            let state = lock_recover(&scheduler.state);
            (
                state.active,
                state.ready.len(),
                state.workers.len(),
                waiting.queued_guard().map(|guard| guard.generation),
            )
        };

        let _ = release_first_tx.send(());
        let waiting_started = if early_start.is_none() {
            started_rx.recv_timeout(timeout).ok()
        } else {
            None
        };
        let _ = release_second_tx.send(());
        let fair_started = if waiting_started.is_some() {
            started_rx.recv_timeout(timeout).ok()
        } else {
            None
        };
        let _ = release_waiting_tx.send(());
        let _ = release_fair_tx.send(());
        drop((first, second, waiting, fair));

        assert_eq!(
            first_started.map(|(session, generation, _)| (session, generation)),
            Ok((1, 1))
        );
        assert_eq!(
            second_started.map(|(session, generation, _)| (session, generation)),
            Ok((2, 1))
        );
        assert_eq!(reserved_slots, 2);
        assert_eq!(queue_len, 2);
        assert!(queue_len <= live_workers);
        assert_eq!(queued_guard, Some(3));
        assert!(early_start.is_none(), "a third worker started before a slot opened");
        assert_eq!(
            waiting_started.map(|(session, generation, _)| (session, generation)),
            Some((3, 3)),
            "the worker should run only its newest waiting generation"
        );
        assert_eq!(
            fair_started.map(|(session, generation, _)| (session, generation)),
            Some((4, 1)),
            "the next live worker should be admitted in queue order"
        );
        assert!(maximum.load(Ordering::SeqCst) <= MAX_ACTIVE_BRIDGE_JOBS);
    }

    #[test]
    fn canceled_compute_keeps_its_slot_while_replacement_uses_the_second() {
        let timeout = Duration::from_secs(5);
        let scheduler = isolated_scheduler();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut worker = controlled_worker(
            scheduler.clone(),
            started_tx,
            release_rx,
            active,
            maximum.clone(),
        );
        let entry = occluview_core::SceneMesh::new(Mesh::empty());
        let (wake_tx, wake_rx) = mpsc::channel();

        let _ = worker.submit(test_input(10, 1, &entry));
        let first_started = started_rx.recv_timeout(timeout);
        worker.cancel();
        worker.set_wake(Arc::new(move || {
            let _ = wake_tx.send(());
        }));
        let _ = worker.submit(test_input(11, 1, &entry));
        let replacement_started = started_rx.recv_timeout(timeout);
        let active_slots = lock_recover(&scheduler.state).active;

        let _ = release_tx.send(());
        let _ = release_tx.send(());
        let replacement_woke = wake_rx.recv_timeout(timeout).is_ok();
        let output_guard = worker.poll().first().map(|output| output.guard);
        drop(worker);

        assert_eq!(
            first_started.map(|(session, generation, _)| (session, generation)),
            Ok((10, 1))
        );
        assert_eq!(
            replacement_started.map(|(session, generation, running)| {
                (session, generation, running)
            }),
            Ok((11, 1, 2))
        );
        assert_eq!(active_slots, 2);
        assert!(replacement_woke);
        assert_eq!(
            output_guard.map(|guard| (guard.session_id, guard.generation)),
            Some((11, 1))
        );
    }

    #[test]
    fn dropping_worker_releases_pending_input_but_keeps_active_slot_until_return() {
        let timeout = Duration::from_secs(5);
        let scheduler = isolated_scheduler();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (release_second_tx, release_second_rx) = mpsc::channel();
        let (release_waiting_tx, release_waiting_rx) = mpsc::channel();
        let (release_replacement_tx, release_replacement_rx) = mpsc::channel();
        let mut first = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_first_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let mut second = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_second_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let mut waiting = controlled_worker(
            Arc::clone(&scheduler),
            started_tx.clone(),
            release_waiting_rx,
            Arc::clone(&active),
            Arc::clone(&maximum),
        );
        let first_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let second_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let waiting_entry = occluview_core::SceneMesh::new(Mesh::empty());
        let replacement_entry = occluview_core::SceneMesh::new(Mesh::empty());

        let _ = first.submit(test_input(31, 1, &first_entry));
        let first_started = started_rx.recv_timeout(timeout);
        let _ = second.submit(test_input(32, 1, &second_entry));
        let second_started = started_rx.recv_timeout(timeout);
        let _ = waiting.submit(test_input(33, 1, &waiting_entry));
        let waiting_mesh_count_before_drop = Arc::strong_count(&waiting_entry.mesh);
        drop(waiting);
        let waiting_mesh_count_after_drop = Arc::strong_count(&waiting_entry.mesh);
        let registrations_after_waiting_drop = lock_recover(&scheduler.state).workers.len();

        let active_mesh_count_before_drop = Arc::strong_count(&first_entry.mesh);
        drop(first);
        let active_slots_after_drop = lock_recover(&scheduler.state).active;
        let active_mesh_count_after_drop = Arc::strong_count(&first_entry.mesh);
        let mut replacement = controlled_worker(
            Arc::clone(&scheduler),
            started_tx,
            release_replacement_rx,
            active,
            maximum.clone(),
        );
        let _ = replacement.submit(test_input(34, 1, &replacement_entry));
        let replacement_waited = matches!(
            started_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        );

        let _ = release_second_tx.send(());
        let replacement_started = started_rx.recv_timeout(timeout);
        let _ = release_first_tx.send(());
        let _ = release_replacement_tx.send(());
        let _ = release_waiting_tx.send(());
        drop((second, replacement));

        assert_eq!(
            first_started.map(|(session, generation, _)| (session, generation)),
            Ok((31, 1))
        );
        assert_eq!(
            second_started.map(|(session, generation, _)| (session, generation)),
            Ok((32, 1))
        );
        assert_eq!(waiting_mesh_count_before_drop, 2);
        assert_eq!(waiting_mesh_count_after_drop, 1);
        assert_eq!(registrations_after_waiting_drop, 2);
        assert_eq!(active_slots_after_drop, 2);
        assert_eq!(active_mesh_count_before_drop, 2);
        assert_eq!(active_mesh_count_after_drop, 2);
        assert!(replacement_waited);
        assert_eq!(
            replacement_started.map(|(session, generation, running)| {
                (session, generation, running)
            }),
            Ok((34, 1, 2))
        );
        assert!(maximum.load(Ordering::SeqCst) <= MAX_ACTIVE_BRIDGE_JOBS);
    }

    #[test]
    fn entry_warming_uses_a_slot_before_its_preview_and_wakes_the_app() {
        let timeout = Duration::from_secs(5);
        let scheduler = isolated_scheduler();
        let entry = occluview_core::SceneMesh::new(Mesh::empty());
        let mesh = Arc::clone(&entry.mesh);
        let (warm_started_tx, warm_started_rx) = mpsc::channel();
        let (preview_started_tx, preview_started_rx) = mpsc::channel();
        let (warm_release_tx, warm_release_rx) = mpsc::channel();
        let (preview_release_tx, preview_release_rx) = mpsc::channel();
        let warm_release_rx = Arc::new(Mutex::new(warm_release_rx));
        let preview_release_rx = Arc::new(Mutex::new(preview_release_rx));
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut worker = BridgeSplitWorker::with_parts(
            Arc::new({
                let preview_release_rx = Arc::clone(&preview_release_rx);
                move |input| {
                    let _ = preview_started_tx.send(input.guard.generation);
                    if let Ok(receiver) = preview_release_rx.lock() {
                        let _ = receiver.recv();
                    }
                    Err(BridgeSplitToolError::WorkerStopped)
                }
            }),
            Arc::new({
                let warm_release_rx = Arc::clone(&warm_release_rx);
                move |_mesh| {
                    let _ = warm_started_tx.send(());
                    if let Ok(receiver) = warm_release_rx.lock() {
                        let _ = receiver.recv();
                    }
                }
            }),
            scheduler.clone(),
        );

        worker.warm_source(
            Arc::clone(&mesh),
            Arc::new(move || {
                let _ = wake_tx.send(());
            }),
        );
        let warm_started = warm_started_rx.recv_timeout(timeout);
        let _ = worker.submit(test_input(20, 1, &entry));
        let queued_guard = worker.queued_guard().map(|guard| guard.generation);
        let preview_before_warm = preview_started_rx.try_recv().ok();

        let _ = warm_release_tx.send(());
        let warm_woke = wake_rx.recv_timeout(timeout).is_ok();
        let preview_started = preview_started_rx.recv_timeout(timeout);
        let _ = preview_release_tx.send(());
        drop(worker);

        assert_eq!(warm_started, Ok(()));
        assert_eq!(queued_guard, Some(1));
        assert!(preview_before_warm.is_none());
        assert!(warm_woke, "warming completion must request a poll/redraw");
        assert_eq!(preview_started, Ok(1));
    }

    #[test]
    fn dropping_workers_removes_registry_and_scheduler_queue_entries() {
        let scheduler = isolated_scheduler();
        for _ in 0..100 {
            let worker = BridgeSplitWorker::with_parts(
                Arc::new(|_input| Err(BridgeSplitToolError::WorkerStopped)),
                Arc::new(|_mesh| {}),
                Arc::clone(&scheduler),
            );
            drop(worker);
        }

        let state = lock_recover(&scheduler.state);
        assert!(state.workers.is_empty());
        assert!(state.ready.is_empty());
        assert!(state.queued.is_empty());
    }

    #[test]
    fn a_new_source_does_not_wait_for_abandoned_preparation() {
        let cache = Mutex::new(BridgeSplitSourceCache::default());
        let source = Arc::new(open_triangle("old"));
        let next = Arc::new(open_triangle("next"));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (next_tx, next_rx) = mpsc::channel();
        let (old_started, reached_next) = thread::scope(|scope| {
            let cache = &cache;
            scope.spawn(move || {
                let _ = BridgeSplitSourceCache::prepared_source(cache, &source, |mesh| {
                    let _ = started_tx.send(());
                    let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
                    prepare_bridge_split_source(mesh)
                });
            });
            let old_started = started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok();
            scope.spawn(move || {
                let _ = BridgeSplitSourceCache::prepared_source(cache, &next, |mesh| {
                    let _ = next_tx.send(());
                    prepare_bridge_split_source(mesh)
                });
            });
            let reached_next = next_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok();
            let _ = release_tx.send(());
            (old_started, reached_next)
        });
        assert!(old_started, "the first preparation did not reach its gate");
        assert!(
            reached_next,
            "source preparation serialized the replacement job"
        );
    }

    #[allow(clippy::expect_used)]
    fn mesh_with_redundant_degenerate_face(name: &str) -> Mesh {
        let vertices = vec![
            Vertex::at([0.0, 0.0, 1.0].into()),
            Vertex::at([0.0, 0.0, -1.0].into()),
            Vertex::at([1.0, 0.0, 0.0].into()),
            Vertex::at([0.0, 1.0, 0.0].into()),
            Vertex::at([-1.0, 0.0, 0.0].into()),
            Vertex::at([0.0, -1.0, 0.0].into()),
        ];
        let mut indices = vec![
            0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 2, 1, 3, 2, 1, 4, 3, 1, 5, 4, 1, 2, 5,
        ];
        indices.extend([0, 0, 1]);
        Mesh::new(Some(name.to_string()), vertices, indices).expect("indexed fixture")
    }

    #[allow(clippy::expect_used)]
    fn open_triangle(name: &str) -> Mesh {
        Mesh::new(
            Some(name.to_string()),
            vec![
                Vertex::at([0.0, 0.0, 0.0].into()),
                Vertex::at([1.0, 0.0, 0.0].into()),
                Vertex::at([0.0, 1.0, 0.0].into()),
            ],
            vec![0, 1, 2],
        )
        .expect("open triangle fixture")
    }
}
