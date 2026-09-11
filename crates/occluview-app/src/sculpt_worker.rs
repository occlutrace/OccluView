//! Background execution for interactive sculpting.
//!
//! The viewport must never wait for a mesh kernel. This module owns the
//! bounded command queue and the worker-side [`SculptSession`]; the UI only
//! submits the newest brush samples and drains sparse GPU updates/completions.

use crate::sculpt_tool::{DabFailure, SculptPickState, SculptRebuild, SculptSession};
use glam::Affine3A;
use occluview_core::{BrushMode, BrushStroke, Mesh, SceneMeshId, Vertex};
use occluview_render::PreparedSceneTopology;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, TryLockError};
use std::thread::{self, JoinHandle};

#[path = "sculpt_worker_loop.rs"]
mod worker_loop;
use worker_loop::{panic_message, run_worker};

const APPLY_QUEUE_CAPACITY_PER_STROKE: usize = 4;
/// A burst of pointer samples must not turn into unbounded memory or latency.
/// Finish markers are retained when this limit is reached; the oldest queued
/// Apply is the only command eligible for coalescing/eviction.
const MAX_QUEUED_COMMANDS: usize = 64;
/// The UI drains completions once per frame. Keep only a small producer-side
/// backlog so a slow frame rate applies backpressure to the kernel thread.
const MAX_PENDING_COMPLETIONS: usize = 2;
const MAX_PENDING_TOUCHES: usize = 250_000;
/// A direct dirty-triangle scan stays cheap for small strokes. Once it would
/// become a second large traversal on every cursor move, the worker refits a
/// fresh dynamic pick mesh and starts a new bounded dirty window.
const MAX_DYNAMIC_PICK_TRIANGLES: usize = 100_000;

enum SculptCommand {
    Apply {
        stroke_id: u64,
        stroke: BrushStroke,
        mode: BrushMode,
    },
    Finish {
        stroke_id: u64,
    },
}

struct QueueState {
    commands: VecDeque<SculptCommand>,
    shutdown: bool,
    next_stroke_id: u64,
    open_stroke: Option<u64>,
}

struct SculptCommandQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
    active: AtomicBool,
    error: Arc<Mutex<Option<SculptFailure>>>,
}

impl SculptCommandQueue {
    #[cfg(test)]
    fn new() -> Self {
        Self::with_error(Arc::new(Mutex::new(None)))
    }

    fn with_error(error: Arc<Mutex<Option<SculptFailure>>>) -> Self {
        Self {
            state: Mutex::new(QueueState {
                commands: VecDeque::new(),
                shutdown: false,
                next_stroke_id: 0,
                open_stroke: None,
            }),
            wake: Condvar::new(),
            active: AtomicBool::new(false),
            error,
        }
    }

    fn report_failure(&self) {
        set_worker_error(&self.error, SculptFailure::WorkerStatePoisoned);
    }

    /// Keep each stroke's APPLY backlog bounded by replacing its oldest queued
    /// dab when the worker is busy. The stroke id is essential: a global cap
    /// would evict all dabs between two Finish markers when the operator makes
    /// two quick strokes, leaving the second stroke with no geometry to apply.
    fn push_apply(&self, stroke: BrushStroke, mode: BrushMode) -> bool {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return false;
        };
        if state.shutdown {
            return false;
        }
        let open_stroke = state.open_stroke;
        let queued_applies = open_stroke.map_or(0, |stroke_id| {
            state
                .commands
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        SculptCommand::Apply {
                            stroke_id: queued_id,
                            ..
                        } if *queued_id == stroke_id
                    )
                })
                .count()
        });
        if queued_applies >= APPLY_QUEUE_CAPACITY_PER_STROKE {
            let Some(stroke_id) = open_stroke else {
                return false;
            };
            let Some(position) = state.commands.iter().position(|command| {
                matches!(
                    command,
                    SculptCommand::Apply {
                        stroke_id: queued_id,
                        ..
                    } if *queued_id == stroke_id
                )
            }) else {
                return false;
            };
            let _ = state.commands.remove(position);
        }
        if !make_room_for_apply(&mut state) {
            return false;
        }
        let stroke_id = if let Some(stroke_id) = open_stroke {
            stroke_id
        } else {
            state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
            let stroke_id = state.next_stroke_id;
            state.open_stroke = Some(stroke_id);
            stroke_id
        };
        state.commands.push_back(SculptCommand::Apply {
            stroke_id,
            stroke,
            mode,
        });
        self.wake.notify_one();
        true
    }

    fn push_finish(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return false;
        };
        if state.shutdown {
            return false;
        }
        if !make_room_for_apply(&mut state) {
            return false;
        }
        let stroke_id = state.open_stroke.take().unwrap_or_else(|| {
            state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
            state.next_stroke_id
        });
        state
            .commands
            .push_back(SculptCommand::Finish { stroke_id });
        self.wake.notify_one();
        true
    }

    fn pop(&self) -> Option<SculptCommand> {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return None;
        };
        loop {
            if let Some(command) = state.commands.pop_front() {
                self.active.store(true, Ordering::Release);
                return Some(command);
            }
            if state.shutdown {
                return None;
            }
            state = if let Ok(state) = self.wake.wait(state) {
                state
            } else {
                self.report_failure();
                return None;
            };
        }
    }

    fn shutdown(&self) {
        match self.state.lock() {
            Ok(mut state) => {
                state.shutdown = true;
                state.commands.clear();
                self.wake.notify_one();
            }
            Err(_) => self.report_failure(),
        }
    }

    fn mark_idle(&self) {
        self.active.store(false, Ordering::Release);
    }

    fn is_empty(&self) -> bool {
        // Fail active on contention: a skipped drain retries on the repaint
        // this returns, while a false quiet would stall the worker's output.
        self.state
            .try_lock()
            .is_ok_and(|state| state.commands.is_empty() && !self.active.load(Ordering::Acquire))
    }
}

/// Make one slot available without ever removing a stroke boundary. Dropping
/// the oldest sample is deliberate lossy backpressure: the worker still sees
/// an ordered, finite stroke and the newest pointer position replaces stale
/// input, while the queue can never grow with mouse frequency.
fn make_room_for_apply(state: &mut QueueState) -> bool {
    if state.commands.len() < MAX_QUEUED_COMMANDS {
        return true;
    }
    let Some(position) = state
        .commands
        .iter()
        .position(|command| matches!(command, SculptCommand::Apply { .. }))
    else {
        return false;
    };
    let _ = state.commands.remove(position);
    true
}

struct WorkerState {
    shadow: Arc<RwLock<Vec<Vertex>>>,
    pick: Arc<RwLock<SculptPickState>>,
    pending_touched: Mutex<Vec<usize>>,
    full_sync: AtomicBool,
    /// Ordered whole-layer rebuilds from densifying dabs. A later unread
    /// rebuild from the SAME stroke may replace its predecessor because no
    /// completion can refer to an intermediate topology within one stroke;
    /// rebuilds from different strokes remain queued in order.
    rebuild: Mutex<VecDeque<PendingRebuild>>,
    completions: Mutex<VecDeque<SculptCompletion>>,
    /// Serializes publication and batch-draining of rebuilds/completions. A
    /// completion produced after a rebuild must never be observed without the
    /// rebuild that establishes its topology contract.
    publish_boundary: Mutex<()>,
    completion_wake: Condvar,
    stopping: AtomicBool,
    error: Arc<Mutex<Option<SculptFailure>>>,
}

type SculptOutputSnapshot = (
    VecDeque<SculptRebuild>,
    VecDeque<SculptCompletion>,
    Option<SculptUpdate>,
);

struct PendingRebuild {
    stroke_id: u64,
    rebuild: SculptRebuild,
}

/// Why the sculpt worker produced nothing trustworthy. Domain data only: the
/// worker never formats user-facing copy; the UI boundary renders it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SculptFailure {
    /// The background thread panicked; carries the payload text for diagnostics.
    WorkerPanicked { message: String },
    /// The worker thread could not be spawned.
    Spawn { detail: String },
    /// The sculpt kernel pool could not be created.
    KernelPool { detail: String },
    /// A finished stroke had no undo baseline.
    MissingUndoBaseline,
    /// The shadow vertex lock was poisoned.
    ShadowPoisoned,
    /// The display shadow no longer has the shape of the kernel mesh.
    ShadowShapeMismatch,
    /// The kernel returned an id outside its prepared vertex array.
    InvalidVertexIndex,
    /// A worker-owned coordination lock was poisoned.
    WorkerStatePoisoned,
    /// The sculpt result changed the vertex count.
    VertexCountChanged,
    /// A densifying dab changed the kernel topology but the app could not
    /// construct the matching authoritative mesh for the renderer.
    TopologyRebuild { detail: String },
}

fn set_worker_error(error: &Mutex<Option<SculptFailure>>, failure: SculptFailure) {
    let mut slot = match error.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if slot.is_none() {
        *slot = Some(failure);
    }
}

impl WorkerState {
    /// Park a topology change for the UI thread. Any vertex ids queued from
    /// earlier dabs are dropped: they index the pre-rebuild array, and the
    /// rebuild replaces it wholesale. Intermediate rebuilds in one unfinished
    /// stroke coalesce, while stroke boundaries stay FIFO for completion
    /// ordering.
    fn record_rebuild(&self, stroke_id: u64, rebuild: SculptRebuild) {
        let Ok(_publish) = self.publish_boundary.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        let Ok(mut pending) = self.pending_touched.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        pending.clear();
        drop(pending);
        self.full_sync.store(false, Ordering::Release);
        let Ok(mut pick) = self.pick.write() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        pick.dirty_triangles.clear();
        drop(pick);
        let Ok(mut rebuilds) = self.rebuild.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        if let Some(last) = rebuilds
            .back_mut()
            .filter(|last| last.stroke_id == stroke_id)
        {
            last.rebuild = rebuild;
        } else {
            rebuilds.push_back(PendingRebuild { stroke_id, rebuild });
        }
    }

    fn record_touched(&self, touched: Vec<usize>, dirty_triangles: Vec<usize>) {
        let Ok(_publish) = self.publish_boundary.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        if !touched.is_empty() && !self.full_sync.load(Ordering::Acquire) {
            let Ok(mut pending) = self.pending_touched.lock() else {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return;
            };
            pending.extend(touched);
            if pending.len() > MAX_PENDING_TOUCHES {
                pending.clear();
                self.full_sync.store(true, Ordering::Release);
            }
        }

        let Ok(mut pick) = self.pick.write() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        // During the one-frame window between a worker rebuild publication and
        // UI installation the old pick mesh has the wrong vertex count. Keep
        // the state explicitly cold rather than allowing a mismatched pick.
        let Ok(shadow) = pick.shadow.read() else {
            self.set_error(SculptFailure::ShadowPoisoned);
            return;
        };
        let shadow_len = shadow.len();
        drop(shadow);
        if pick.mesh.vertices().len() != shadow_len {
            pick.dirty_triangles.clear();
            return;
        }
        pick.dirty_triangles.extend(dirty_triangles);
        pick.dirty_triangles.sort_unstable();
        pick.dirty_triangles.dedup();
        if pick.dirty_triangles.len() > MAX_DYNAMIC_PICK_TRIANGLES {
            let refreshed = {
                let Ok(shadow) = pick.shadow.read() else {
                    self.set_error(SculptFailure::ShadowPoisoned);
                    return;
                };
                pick.mesh.with_sculpted_vertices(shadow.clone())
            };
            let Some(refreshed) = refreshed else {
                self.set_error(SculptFailure::ShadowShapeMismatch);
                return;
            };
            pick.mesh = Arc::new(refreshed);
            pick.dirty_triangles.clear();
        }
    }

    fn push_completion(&self, completion: SculptCompletion) -> bool {
        let Ok(mut completions) = self.completions.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return false;
        };
        while completions.len() >= MAX_PENDING_COMPLETIONS && !self.stopping.load(Ordering::Acquire)
        {
            completions = if let Ok(completions) = self.completion_wake.wait(completions) {
                completions
            } else {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return false;
            };
        }
        if self.stopping.load(Ordering::Acquire) {
            return false;
        }
        // The frame path takes the publication boundary before draining both
        // output queues. Release this lock before taking that boundary, or a
        // full completion backlog could deadlock producer and UI.
        drop(completions);
        let Ok(_publish) = self.publish_boundary.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return false;
        };
        let Ok(mut completions) = self.completions.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return false;
        };
        if self.stopping.load(Ordering::Acquire) || completions.len() >= MAX_PENDING_COMPLETIONS {
            return false;
        }
        completions.push_back(completion);
        true
    }

    fn set_error(&self, failure: SculptFailure) {
        set_worker_error(&self.error, failure);
    }

    fn has_error(&self) -> bool {
        match self.error.lock() {
            Ok(error) => error.is_some(),
            Err(poisoned) => poisoned.into_inner().is_some(),
        }
    }

    /// Re-queue a drained-but-unapplied update (lock contention on the frame
    /// path). A full sync supersedes queued deltas; sparse ids merge back and
    /// overflow escalates to a full sync. Never blocks.
    fn restore_update(&self, update: SculptUpdate) {
        let _publish = match self.publish_boundary.try_lock() {
            Ok(publish) => publish,
            Err(TryLockError::WouldBlock) => {
                self.full_sync.store(true, Ordering::Release);
                return;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return;
            }
        };
        if update.full_sync {
            self.full_sync.store(true, Ordering::Release);
            return;
        }
        if update.touched.is_empty() {
            return;
        }
        // A queued rebuild supersedes sparse ids: they index the pre-rebuild
        // array, so escalate to a full sync of the latest shadow instead of
        // resurrecting stale ids behind the rebuild. A contended rebuild
        // lock means the worker is mid-publish; a full sync covers either
        // outcome.
        let slot = match self.rebuild.try_lock() {
            Ok(slot) => slot,
            Err(TryLockError::WouldBlock) => {
                self.full_sync.store(true, Ordering::Release);
                return;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return;
            }
        };
        if !slot.is_empty() {
            self.full_sync.store(true, Ordering::Release);
            return;
        }
        drop(slot);
        let mut pending = match self.pending_touched.try_lock() {
            Ok(pending) => pending,
            Err(TryLockError::WouldBlock) => {
                self.full_sync.store(true, Ordering::Release);
                return;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return;
            }
        };
        pending.extend(update.touched);
        if pending.len() > MAX_PENDING_TOUCHES {
            pending.clear();
            self.full_sync.store(true, Ordering::Release);
        }
    }

    /// Mark the next drain authoritative after a GPU-write rejection.
    fn request_full_sync(&self) {
        self.full_sync.store(true, Ordering::Release);
    }

    /// Atomically snapshot every worker output that has been published so far.
    /// The worker holds `publish_boundary` while adding rebuilds, sparse
    /// updates, and completions; the UI holds it while taking this snapshot.
    /// Therefore a completion can never be observed without the topology
    /// rebuild that makes its mesh valid, and a sparse update can never pass a
    /// queued rebuild into the old GPU buffers.
    fn take_ordered_outputs(&self) -> Result<SculptOutputSnapshot, ()> {
        let _publish = match self.publish_boundary.try_lock() {
            Ok(publish) => publish,
            Err(TryLockError::WouldBlock) => return Err(()),
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return Err(());
            }
        };
        let mut rebuilds = match self.rebuild.try_lock() {
            Ok(rebuilds) => rebuilds,
            Err(TryLockError::WouldBlock) => return Err(()),
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return Err(());
            }
        };
        let mut completions = match self.completions.try_lock() {
            Ok(completions) => completions,
            Err(TryLockError::WouldBlock) => return Err(()),
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return Err(());
            }
        };
        let mut pending = match self.pending_touched.try_lock() {
            Ok(pending) => pending,
            Err(TryLockError::WouldBlock) => return Err(()),
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return Err(());
            }
        };
        let rebuilds = std::mem::take(&mut *rebuilds)
            .into_iter()
            .map(|pending| pending.rebuild)
            .collect();
        let completions = std::mem::take(&mut *completions);
        let full_sync = self.full_sync.swap(false, Ordering::AcqRel);
        let touched = std::mem::take(&mut *pending);
        self.completion_wake.notify_all();
        let update =
            (full_sync || !touched.is_empty()).then_some(SculptUpdate { touched, full_sync });
        Ok((rebuilds, completions, update))
    }
}

/// A completed stroke that is ready to become one undoable scene edit.
pub(crate) struct SculptCompletion {
    /// Mesh state before this stroke, prepared off the UI thread for undo.
    pub(crate) before: Arc<Mesh>,
    /// Mesh state after this stroke, ready for scene commit.
    pub(crate) mesh: Mesh,
}

/// Sparse live update accumulated by the worker between UI frames.
pub(crate) struct SculptUpdate {
    pub(crate) touched: Vec<usize>,
    pub(crate) full_sync: bool,
}

/// Persistent worker for one prepared layer.
pub(crate) struct SculptWorker {
    pub(crate) layer_id: SceneMeshId,
    /// Identity of the layer geometry this worker is authoritative for. A
    /// densifying dab replaces the layer, so the UI updates this (and
    /// `topology`) when it installs the rebuild.
    pub(crate) topology_id: u64,
    pub(crate) topology: PreparedSceneTopology,
    pub(crate) world_to_local: Affine3A,
    pub(crate) local_per_world: f32,
    state: Arc<WorkerState>,
    queue: Arc<SculptCommandQueue>,
    worker_thread: Option<JoinHandle<()>>,
}

impl SculptWorker {
    pub(crate) fn spawn(session: SculptSession) -> Self {
        let layer_id = session.layer_id;
        let topology_id = session.topology_id;
        let topology = session.topology;
        let world_to_local = session.world_to_local;
        let local_per_world = session.local_per_world;
        let error = Arc::new(Mutex::new(None));
        let state = Arc::new(WorkerState {
            shadow: Arc::clone(&session.shadow),
            pick: Arc::new(RwLock::new(SculptPickState {
                mesh: Arc::clone(&session.base_mesh),
                shadow: Arc::clone(&session.shadow),
                dirty_triangles: Vec::new(),
            })),
            pending_touched: Mutex::new(Vec::new()),
            full_sync: AtomicBool::new(false),
            rebuild: Mutex::new(VecDeque::new()),
            completions: Mutex::new(VecDeque::new()),
            publish_boundary: Mutex::new(()),
            completion_wake: Condvar::new(),
            stopping: AtomicBool::new(false),
            error: Arc::clone(&error),
        });
        let queue = Arc::new(SculptCommandQueue::with_error(error));
        let worker_queue = Arc::clone(&queue);
        let worker_state = Arc::clone(&state);
        let pool_threads = thread::available_parallelism()
            .map_or(1, |count| count.get().saturating_sub(1).clamp(1, 4));
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(pool_threads)
            .thread_name(|index| format!("occluview-sculpt-kernel-{index}"))
            .build();
        let worker_thread = match pool {
            Ok(pool) => {
                let spawn_result = thread::Builder::new()
                    .name("occluview-sculpt-worker".to_string())
                    .spawn(move || {
                        // Dead in the shipped binary: the release profile is
                        // `panic = "abort"`, so a panic here takes the process
                        // regardless. It still catches under `cargo test` and
                        // under the `release-unwind` profile the shell DLL
                        // uses, which is where a panicking worker is worth
                        // reporting rather than losing.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_worker(session, worker_queue.clone(), worker_state.clone(), pool);
                        }));
                        if let Err(payload) = result {
                            worker_state.set_error(SculptFailure::WorkerPanicked {
                                message: panic_message(payload),
                            });
                            worker_queue.mark_idle();
                        }
                    });
                match spawn_result {
                    Ok(handle) => Some(handle),
                    Err(error) => {
                        state.set_error(SculptFailure::Spawn {
                            detail: error.to_string(),
                        });
                        None
                    }
                }
            }
            Err(error) => {
                state.set_error(SculptFailure::KernelPool {
                    detail: error.to_string(),
                });
                None
            }
        };
        Self {
            layer_id,
            topology_id,
            topology,
            world_to_local,
            local_per_world,
            state,
            queue,
            worker_thread,
        }
    }

    pub(crate) fn try_apply(&self, stroke: BrushStroke, mode: BrushMode) -> bool {
        self.queue.push_apply(stroke, mode)
    }

    pub(crate) fn finish_stroke(&self) -> bool {
        self.queue.push_finish()
    }

    pub(crate) fn shadow(&self) -> Arc<RwLock<Vec<Vertex>>> {
        Arc::clone(&self.state.shadow)
    }

    /// Pick the current live Sculpt surface in mesh-local coordinates. A
    /// contended read means the worker is publishing a dab; returning `None`
    /// for that frame keeps the UI non-blocking and retries on repaint.
    pub(crate) fn pick_local_ray(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
    ) -> Option<(usize, glam::Vec3)> {
        let pick = self.state.pick.try_read().ok()?;
        let shadow = pick.shadow.try_read().ok()?;
        pick.mesh.pick_ray_local_with_vertices(
            occluview_core::LiveRayPick::new(&shadow, &pick.dirty_triangles, origin, direction),
            |_| true,
        )
    }

    pub(crate) fn local_triangle_normal(&self, triangle: usize) -> Option<glam::Vec3> {
        let pick = self.state.pick.try_read().ok()?;
        let shadow = pick.shadow.try_read().ok()?;
        pick.mesh
            .triangle_normal_local_with_vertices(&shadow, triangle)
    }

    /// Install the freshly rebuilt topology into the dynamic picker after the
    /// UI has accepted the same mesh into the scene.
    pub(crate) fn replace_pick_mesh(&self, mesh: Arc<Mesh>) {
        if let Ok(mut pick) = self.state.pick.write() {
            pick.mesh = mesh;
            pick.dirty_triangles.clear();
        } else {
            self.state.set_error(SculptFailure::WorkerStatePoisoned);
        }
    }

    #[cfg(test)]
    pub(crate) fn take_update(&self) -> Option<SculptUpdate> {
        // Non-blocking: a contended backlog (and its full-sync flag) stays
        // queued for the next frame instead of stalling the egui frame.
        let Ok(_publish) = self.state.publish_boundary.try_lock() else {
            return None;
        };
        // A rebuild published between two frame-path operations owns the
        // vertex array. Keep sparse ids behind it until the UI has installed
        // that whole-layer replacement.
        let Ok(rebuilds) = self.state.rebuild.try_lock() else {
            return None;
        };
        if !rebuilds.is_empty() {
            return None;
        }
        drop(rebuilds);
        let Ok(mut pending) = self.state.pending_touched.try_lock() else {
            return None;
        };
        let full_sync = self.state.full_sync.swap(false, Ordering::AcqRel);
        let touched = std::mem::take(&mut *pending);
        (full_sync || !touched.is_empty()).then_some(SculptUpdate { touched, full_sync })
    }

    /// Take the pending whole-layer rebuild, if a dab densified the mesh.
    /// Must be drained BEFORE `take_update`, so a sparse write never lands on
    /// buffers the rebuild is about to replace. The `Err` result is a
    /// contention signal: a completion can only be interpreted after the UI
    /// has successfully observed every earlier rebuild.
    #[cfg(test)]
    pub(crate) fn try_take_rebuild(&self) -> Result<Option<SculptRebuild>, ()> {
        let _publish = self.state.publish_boundary.try_lock().map_err(|_| ())?;
        let mut rebuilds = self.state.rebuild.try_lock().map_err(|_| ())?;
        Ok(rebuilds.pop_front().map(|pending| pending.rebuild))
    }

    #[cfg(test)]
    pub(crate) fn take_completion(&self) -> Option<SculptCompletion> {
        let Ok(_publish) = self.state.publish_boundary.try_lock() else {
            return None;
        };
        let completion = self
            .state
            .completions
            .try_lock()
            .ok()
            .and_then(|mut completions| completions.pop_front());
        if completion.is_some() {
            self.state.completion_wake.notify_one();
        }
        completion
    }

    pub(crate) fn take_error(&self) -> Option<SculptFailure> {
        match self.state.error.try_lock() {
            Ok(mut error) => error.take(),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
        }
    }

    /// Re-queue a drained-but-unapplied update after frame-path contention.
    pub(crate) fn restore_update(&self, update: SculptUpdate) {
        self.state.restore_update(update);
    }

    /// Escalate to an authoritative full sync after a GPU-write rejection.
    pub(crate) fn request_full_sync(&self) {
        self.state.request_full_sync();
    }

    /// Drain topology, completion, and sparse-update outputs as one
    /// publication boundary. The production frame poller uses this method so
    /// it cannot observe a completion or sparse write without the rebuild that
    /// establishes its topology contract.
    pub(crate) fn take_ordered_outputs(&self) -> Result<SculptOutputSnapshot, ()> {
        self.state.take_ordered_outputs()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        // Every undrained slot counts: a dab the worker already processed but
        // the UI has not flushed yet (pending touches, full-sync flag, layer
        // rebuild, queued completion) is still live work. Reporting quiet
        // here lets Done/undo invalidate the session and drop it. On lock
        // contention report busy instead: deferring one frame is free, while
        // a false quiet loses sculpted geometry.
        let Ok(_publish) = self.state.publish_boundary.try_lock() else {
            return false;
        };
        self.queue.is_empty()
            && self
                .state
                .completions
                .try_lock()
                .is_ok_and(|completions| completions.is_empty())
            && self
                .state
                .pending_touched
                .try_lock()
                .is_ok_and(|touched| touched.is_empty())
            && !self.state.full_sync.load(Ordering::Acquire)
            && self
                .state
                .rebuild
                .try_lock()
                .is_ok_and(|rebuilds| rebuilds.is_empty())
    }
}

impl Drop for SculptWorker {
    fn drop(&mut self) {
        self.state.stopping.store(true, Ordering::Release);
        self.state.completion_wake.notify_all();
        self.queue.shutdown();
        let Some(worker_thread) = self.worker_thread.take() else {
            return;
        };
        // The worker never owns this handle, so this branch is defensive only;
        // still avoid a self-join if a future refactor moves the owner into
        // the worker closure.
        if worker_thread.thread().id() != thread::current().id() {
            let _ = worker_thread.join();
        }
    }
}

// Split out to hold the workspace's 800-line file budget. A `#[path]` child
// module so the tests still reach this file's private items.
#[cfg(test)]
#[path = "sculpt_worker_tests.rs"]
mod tests;
