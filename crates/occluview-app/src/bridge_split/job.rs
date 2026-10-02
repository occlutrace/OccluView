use super::state::{BridgeSplitGuard, BridgeSplitToolError};
use glam::Affine3A;
use occluview_core::Mesh;
use occluview_mesh_edit::{
    bridge_split_prepared_mesh_in_world, prepare_bridge_split_source, BridgeSplitRequest,
    CoreBridgeSplitResult, PreparedBridgeSplitSource,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

const SOURCE_CACHE_CAPACITY: usize = 8;

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

pub(crate) struct BridgeSplitWorker {
    request_tx: Option<mpsc::SyncSender<BridgeSplitJobInput>>,
    result_rx: mpsc::Receiver<BridgeSplitJobOutput>,
    active: Option<BridgeSplitGuard>,
    queued: Option<BridgeSplitJobInput>,
    /// The compute this worker was built with, kept so `abandon` can restart the
    /// thread and the channel state without discarding it. Constructing a fresh
    /// default worker instead would throw away an injected compute (the
    /// cancellation tests inject one).
    compute: Arc<BridgeSplitCompute>,
}

type BridgeSplitCompute = dyn Fn(BridgeSplitJobInput) -> Result<CoreBridgeSplitResult, BridgeSplitToolError>
    + Send
    + Sync;

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
        Self::with_compute(Arc::new(compute))
    }

    /// Start a worker thread over `compute` with fresh channels and no work.
    fn with_compute(compute: Arc<BridgeSplitCompute>) -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::channel();
        // Dropping the handle detaches the thread; channel closure stops it after active compute.
        let _worker_thread = thread::Builder::new()
            .name("bridge-split".to_string())
            .spawn({
                let compute = Arc::clone(&compute);
                move || worker_loop(request_rx, result_tx, compute)
            });
        Self {
            request_tx: Some(request_tx),
            result_rx,
            active: None,
            queued: None,
            compute,
        }
    }

    /// Throw this worker's thread and channel state away and start a new one over
    /// the same compute.
    ///
    /// A running `compute` has no cancellation flag, and its `active` guard stays
    /// set until the output arrives, so the next request would otherwise queue
    /// behind an abandoned job. Dropping the sender ends the old thread once it
    /// finishes; the fresh channels leave the next job unblocked. Output still in
    /// flight becomes unreachable, which is correct: a cancel ends the session it
    /// described.
    pub(crate) fn abandon(&mut self) {
        *self = Self::with_compute(Arc::clone(&self.compute));
    }

    pub(crate) fn submit(
        &mut self,
        input: BridgeSplitJobInput,
    ) -> Result<(), BridgeSplitToolError> {
        if self.active.is_some() {
            self.queued = Some(input);
            return Ok(());
        }
        self.send_to_worker(input)
    }

    pub(crate) fn clear_queued(&mut self) {
        self.queued = None;
    }

    pub(crate) fn poll(&mut self) -> Vec<BridgeSplitJobOutput> {
        let mut outputs = Vec::new();
        loop {
            match self.result_rx.try_recv() {
                Ok(output) => {
                    if let Some(extra) = self.finish_active(output.guard) {
                        outputs.push(extra);
                    }
                    outputs.push(output);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.request_tx = None;
                    if let Some(guard) = self.active.take() {
                        outputs.push(BridgeSplitJobOutput {
                            guard,
                            result: Err(BridgeSplitToolError::WorkerStopped),
                        });
                    }
                    if let Some(input) = self.queued.take() {
                        outputs.push(BridgeSplitJobOutput {
                            guard: input.guard,
                            result: Err(BridgeSplitToolError::WorkerStopped),
                        });
                    }
                    break;
                }
            }
        }
        outputs
    }

    #[cfg(test)]
    pub(crate) fn active_guard(&self) -> Option<BridgeSplitGuard> {
        self.active
    }

    #[cfg(test)]
    pub(crate) fn queued_guard(&self) -> Option<BridgeSplitGuard> {
        self.queued.as_ref().map(|input| input.guard)
    }

    fn send_to_worker(&mut self, input: BridgeSplitJobInput) -> Result<(), BridgeSplitToolError> {
        let guard = input.guard;
        let Some(request_tx) = self.request_tx.as_ref() else {
            return Err(BridgeSplitToolError::WorkerStopped);
        };
        request_tx
            .send(input)
            .map_err(|_| BridgeSplitToolError::WorkerStopped)?;
        self.active = Some(guard);
        Ok(())
    }

    fn finish_active(&mut self, guard: BridgeSplitGuard) -> Option<BridgeSplitJobOutput> {
        if self.active == Some(guard) {
            self.active = None;
        }
        let next = self.queued.take()?;
        let next_guard = next.guard;
        match self.send_to_worker(next) {
            Ok(()) => None,
            Err(error) => Some(BridgeSplitJobOutput {
                guard: next_guard,
                result: Err(error),
            }),
        }
    }
}

fn worker_loop<F>(
    request_rx: mpsc::Receiver<BridgeSplitJobInput>,
    result_tx: mpsc::Sender<BridgeSplitJobOutput>,
    compute: Arc<F>,
) where
    F: ?Sized
        + Fn(BridgeSplitJobInput) -> Result<CoreBridgeSplitResult, BridgeSplitToolError>
        + Send
        + Sync
        + 'static,
{
    while let Ok(input) = request_rx.recv() {
        let guard = input.guard;
        let result = compute(input);
        if result_tx
            .send(BridgeSplitJobOutput { guard, result })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::Vertex;

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
    fn a_new_source_does_not_wait_for_abandoned_preparation() {
        let cache = Mutex::new(BridgeSplitSourceCache::default());
        let source = Arc::new(open_triangle("old"));
        let next = Arc::new(open_triangle("next"));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (next_tx, next_rx) = mpsc::channel();
        let reached_next = thread::scope(|scope| {
            let cache = &cache;
            scope.spawn(move || {
                let _ = BridgeSplitSourceCache::prepared_source(cache, &source, |mesh| {
                    let _ = started_tx.send(());
                    let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
                    prepare_bridge_split_source(mesh)
                });
            });
            assert!(started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok());
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
            reached_next
        });
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
