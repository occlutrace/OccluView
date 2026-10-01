//! Background execution for interactive sculpting.
//!
//! The viewport must never wait for a mesh kernel. This module owns the
//! bounded command queue and the worker-side [`SculptSession`]; the UI only
//! submits the newest brush samples and drains sparse GPU updates/completions.

use crate::sculpt_kernel::BrushRayStep;
#[cfg(test)]
use crate::sculpt_kernel::{BrushMode, BrushStroke, DabDose};
#[cfg(test)]
use crate::sculpt_tool::SculptTip;
use crate::sculpt_tool::{DabFailure, DabOutcome, SculptPickState, SculptSession};
use glam::{Affine3A, DVec3, Vec3};
use occluview_core::{Mesh, SceneMeshId, Vertex};
use occluview_render::{PreparedSceneTopology, SculptTopologyDelta};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[path = "sculpt_worker_loop.rs"]
mod worker_loop;
use worker_loop::{panic_message, run_worker};

const APPLY_QUEUE_CAPACITY_PER_STROKE: usize = 4;
/// Bound on total queued commands. One slot is reserved for Finish so accepted
/// ray samples can never block their own stroke boundary.
const MAX_QUEUED_COMMANDS: usize = 64;
/// The UI drains completions once per frame. Keep only a small producer-side
/// backlog so a slow frame rate applies backpressure to the kernel thread.
const MAX_PENDING_COMPLETIONS: usize = 2;
const MAX_PENDING_TOUCHES: usize = 250_000;
/// A direct dirty-triangle scan stays cheap for small strokes. Once it would
/// become a second large traversal on every cursor move, the worker refits a
/// fresh dynamic pick mesh and starts a new bounded dirty window.
const MAX_DYNAMIC_PICK_TRIANGLES: usize = 100_000;

// Queue coalescing must preserve exact captured brush settings; an epsilon
// could change the operation or its user-selected strength.
#[allow(clippy::float_cmp)]
pub(crate) fn same_brush_and_visibility(a: &BrushRayStep, b: &BrushRayStep) -> bool {
    a.mode == b.mode
        && a.tip == b.tip
        && a.radius_mm == b.radius_mm
        && a.strength == b.strength
        && a.axis == b.axis
        && a.preserve_skirt == b.preserve_skirt
        && a.near_mm == b.near_mm
        && a.far_mm == b.far_mm
        && a.clip_plane == b.clip_plane
}

// Keep the established sub-millimetre tolerance for coalescing stationary rays;
// exact equality is reserved for captured brush parameters above.
pub(crate) fn same_ray(a: &BrushRayStep, b: &BrushRayStep) -> bool {
    let a_origin = Vec3::from_array(a.origin);
    let b_origin = Vec3::from_array(b.origin);
    let a_direction = Vec3::from_array(a.direction).normalize_or_zero();
    let b_direction = Vec3::from_array(b.direction).normalize_or_zero();
    a_origin.distance_squared(b_origin) <= 1e-6 && a_direction.dot(b_direction) >= 1.0 - 1e-6
}

enum SculptCommand {
    RayStep {
        stroke_id: u64,
        step: BrushRayStep,
        /// The first pointer sample is retained when later samples arrive
        /// before the worker starts it, so a quick press still has a dab.
        first: bool,
    },
    BreakPath {
        stroke_id: u64,
    },
    PrimeWallRegion {
        center: DVec3,
        radius_mm: f64,
        budget: usize,
    },
    #[cfg(test)]
    Apply {
        stroke_id: u64,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
    },
    Finish,
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
    #[cfg(test)]
    pause_pop: AtomicBool,
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
            #[cfg(test)]
            pause_pop: AtomicBool::new(false),
        }
    }

    fn report_failure(&self) {
        set_worker_error(&self.error, SculptFailure::WorkerStatePoisoned);
    }

    /// Submit one physical pointer sample. The first sample is pinned until
    /// the worker starts the stroke; compatible queued travel samples
    /// coalesce to the newest ray, while parameter changes and distinct dwell
    /// samples retain FIFO order.
    fn push_ray_step(&self, step: BrushRayStep) -> bool {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return false;
        };
        if state.shutdown {
            return false;
        }
        let Some(stroke_id) = state.open_stroke else {
            if state.commands.len() >= MAX_QUEUED_COMMANDS - 1 {
                return false;
            }
            state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
            let stroke_id = state.next_stroke_id;
            state.open_stroke = Some(stroke_id);
            state.commands.push_back(SculptCommand::RayStep {
                stroke_id,
                step,
                first: true,
            });
            self.wake.notify_one();
            return true;
        };

        let last_for_stroke = state.commands.iter().rposition(|command| match command {
            SculptCommand::RayStep {
                stroke_id: queued_id,
                ..
            }
            | SculptCommand::BreakPath {
                stroke_id: queued_id,
            } => *queued_id == stroke_id,
            _ => false,
        });
        let last_command_is_ray =
            last_for_stroke.is_some_and(|position| position + 1 == state.commands.len());
        if let Some(position) = last_for_stroke.filter(|_| last_command_is_ray) {
            if let Some(SculptCommand::RayStep {
                step: previous,
                first,
                ..
            }) = state.commands.get_mut(position)
            {
                if !*first
                    && same_brush_and_visibility(previous, &step)
                    && !previous.hold
                    && !step.hold
                {
                    *previous = step;
                    self.wake.notify_one();
                    return true;
                }
                if !*first
                    && same_brush_and_visibility(previous, &step)
                    && same_ray(previous, &step)
                    && previous.hold
                    && step.hold
                {
                    *previous = step;
                    self.wake.notify_one();
                    return true;
                }
            }
        }
        let queued = state
            .commands
            .iter()
            .filter(|command| {
                matches!(command, SculptCommand::RayStep { stroke_id: queued_id, .. } if *queued_id == stroke_id)
            })
            .count();
        if queued >= APPLY_QUEUE_CAPACITY_PER_STROKE
            || state.commands.len() >= MAX_QUEUED_COMMANDS - 1
        {
            return false;
        }
        state.commands.push_back(SculptCommand::RayStep {
            stroke_id,
            step,
            first: false,
        });
        self.wake.notify_one();
        true
    }

    fn push_break_path(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return false;
        };
        if state.shutdown {
            return false;
        }
        let Some(stroke_id) = state.open_stroke else {
            return true;
        };
        if matches!(
            state.commands.back(),
            Some(SculptCommand::BreakPath { stroke_id: queued_id }) if *queued_id == stroke_id
        ) {
            return true;
        }
        if state.commands.len() >= MAX_QUEUED_COMMANDS - 1 {
            return false;
        }
        state
            .commands
            .push_back(SculptCommand::BreakPath { stroke_id });
        self.wake.notify_one();
        true
    }

    fn push_prime_wall_region(&self, center: DVec3, radius_mm: f64, budget: usize) -> bool {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return false;
        };
        if state.shutdown || state.open_stroke.is_some() {
            return false;
        }
        if let Some(command) = state
            .commands
            .iter_mut()
            .rev()
            .find(|command| matches!(command, SculptCommand::PrimeWallRegion { .. }))
        {
            *command = SculptCommand::PrimeWallRegion {
                center,
                radius_mm,
                budget,
            };
            self.wake.notify_one();
            return true;
        }
        if state.commands.len() >= MAX_QUEUED_COMMANDS - 1 {
            return false;
        }
        state.commands.push_back(SculptCommand::PrimeWallRegion {
            center,
            radius_mm,
            budget,
        });
        self.wake.notify_one();
        true
    }

    /// Test-only point-dab path used by legacy adapter and lifecycle fixtures.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn push_apply(
        &self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
    ) -> bool {
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
        if queued_applies >= APPLY_QUEUE_CAPACITY_PER_STROKE
            || state.commands.len() >= MAX_QUEUED_COMMANDS - 1
        {
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
            tip,
            axis,
            stroke,
            mode,
            dose,
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
        if state.open_stroke.is_none() {
            return true;
        }
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return false;
        }
        let _stroke_id = state.open_stroke.take();
        state.commands.push_back(SculptCommand::Finish);
        self.wake.notify_one();
        true
    }

    fn pop(&self) -> Option<SculptCommand> {
        let Ok(mut state) = self.state.lock() else {
            self.report_failure();
            return None;
        };
        loop {
            if state.shutdown {
                return None;
            }
            #[cfg(test)]
            if self.pause_pop.load(Ordering::Acquire) {
                state = if let Ok(state) = self.wake.wait(state) {
                    state
                } else {
                    self.report_failure();
                    return None;
                };
                continue;
            }
            if let Some(command) = state.commands.pop_front() {
                self.active.store(true, Ordering::Release);
                return Some(command);
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

    #[cfg(test)]
    fn set_paused_for_tests(&self, paused: bool) {
        self.pause_pop.store(paused, Ordering::Release);
        self.wake.notify_all();
    }

    fn is_empty(&self) -> bool {
        // Fail active on contention: a skipped drain retries on the repaint
        // this returns, while a false quiet would stall the worker's output.
        self.state
            .try_lock()
            .is_ok_and(|state| state.commands.is_empty() && !self.active.load(Ordering::Acquire))
    }
}

struct WorkerState {
    shadow: Arc<RwLock<Vec<Vertex>>>,
    pick: Arc<RwLock<SculptPickState>>,
    pending_touched: Mutex<Vec<usize>>,
    full_sync: AtomicBool,
    /// Ordered local topology publications; each delta's base counts match
    /// the preceding publication.
    topology_deltas: Mutex<VecDeque<SculptTopologyDelta>>,
    completions: Mutex<VecDeque<SculptCompletion>>,
    /// Serializes publication and batch-draining of geometry updates and
    /// completions.
    publish_boundary: Mutex<()>,
    /// Odd while the worker mutates the shared display geometry.
    geometry_revision: AtomicU64,
    /// The scene still needs to be reconciled with live sculpt output.
    geometry_dirty: AtomicBool,
    completion_wake: Condvar,
    stopping: AtomicBool,
    error: Arc<Mutex<Option<SculptFailure>>>,
    /// Test-only trigger: when set, the worker body panics as soon as it takes a
    /// command, exercising the real `catch_unwind` boundary in `spawn`. Never
    /// set on a production worker.
    #[cfg(test)]
    panic_on_next_command: AtomicBool,
    #[cfg(test)]
    input_trace: Mutex<Vec<SculptWorkerInput>>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SculptWorkerInput {
    Ray {
        mode: BrushMode,
        hold: bool,
        preserve_skirt: bool,
        elapsed_ms: f64,
    },
    BreakPath,
    Finish,
}

/// Dose timing belongs to applied dabs, not raw pointer events. The clock is
/// kept for the life of one prepared worker, including Finish/path breaks.
#[derive(Default)]
pub(crate) struct DabDispatchClock {
    last_dispatch_started_at: Option<Instant>,
}

impl DabDispatchClock {
    pub(crate) fn next_elapsed_ms(&mut self, now: Instant) -> f64 {
        let elapsed_ms = self
            .last_dispatch_started_at
            .map_or(occluview_sculpt::DWELL_FULL_DOSE_MS, |last| {
                now.saturating_duration_since(last).as_secs_f64() * 1000.0
            });
        self.last_dispatch_started_at = Some(now);
        elapsed_ms.min(occluview_sculpt::DWELL_FULL_DOSE_MS)
    }
}

type SculptOutputSnapshot = (
    VecDeque<SculptTopologyDelta>,
    VecDeque<SculptCompletion>,
    Option<SculptUpdate>,
);

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
    /// The final sculpted topology could not be built as a mesh.
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
    fn begin_geometry_update(&self) {
        self.geometry_revision.fetch_add(1, Ordering::AcqRel);
    }

    fn finish_geometry_update(&self) {
        self.geometry_revision.fetch_add(1, Ordering::Release);
    }

    #[cfg(test)]
    fn record_input(&self, input: SculptWorkerInput) {
        if let Ok(mut trace) = self.input_trace.lock() {
            trace.push(input);
        }
    }

    fn reset_pick_geometry(&self, mesh: Arc<Mesh>, indices: Vec<u32>) {
        match self.pick.write() {
            Ok(mut pick) => {
                pick.mesh = mesh;
                pick.indices = indices;
                pick.dirty_triangles.clear();
            }
            Err(_) => self.set_error(SculptFailure::WorkerStatePoisoned),
        }
    }

    /// Publish one local vertex and face patch to the UI thread.
    fn record_topology(&self, delta: SculptTopologyDelta) {
        let Ok(_publish) = self.publish_boundary.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        let Ok(mut pick) = self.pick.write() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        if pick.indices.len() != delta.base_index_count {
            self.set_error(SculptFailure::ShadowShapeMismatch);
            return;
        }
        pick.indices.resize(delta.live_index_count, 0);
        for update in &delta.face_updates {
            let start = update.triangle as usize * 3;
            let Some(row) = pick.indices.get_mut(start..start + 3) else {
                self.set_error(SculptFailure::ShadowShapeMismatch);
                return;
            };
            row.copy_from_slice(&update.indices);
        }
        let Ok(shadow) = pick.shadow.read() else {
            self.set_error(SculptFailure::ShadowPoisoned);
            return;
        };
        let shadow_len = shadow.len();
        drop(shadow);
        if pick
            .indices
            .iter()
            .any(|&index| index as usize >= shadow_len)
        {
            self.set_error(SculptFailure::InvalidVertexIndex);
            return;
        }
        pick.dirty_triangles
            .extend(delta.dirty_triangles.iter().copied());
        pick.dirty_triangles.sort_unstable();
        pick.dirty_triangles.dedup();
        if pick.dirty_triangles.len() > MAX_DYNAMIC_PICK_TRIANGLES {
            let shadow = if let Ok(shadow) = pick.shadow.read() {
                shadow.clone()
            } else {
                self.set_error(SculptFailure::ShadowPoisoned);
                return;
            };
            let Some(refreshed) = pick
                .mesh
                .with_sculpted_geometry(shadow, pick.indices.clone())
            else {
                self.set_error(SculptFailure::ShadowShapeMismatch);
                return;
            };
            refreshed.warm_bvh();
            pick.mesh = Arc::new(refreshed);
            pick.dirty_triangles.clear();
        }
        let Ok(mut topology_deltas) = self.topology_deltas.lock() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        topology_deltas.push_back(delta);
        self.geometry_dirty.store(true, Ordering::Release);
    }

    fn record_touched(&self, touched: Vec<usize>, dirty_triangles: Vec<usize>) {
        let has_touched = !touched.is_empty();
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
        if has_touched {
            self.geometry_dirty.store(true, Ordering::Release);
        }

        let Ok(mut pick) = self.pick.write() else {
            self.set_error(SculptFailure::WorkerStatePoisoned);
            return;
        };
        let Ok(shadow) = pick.shadow.read() else {
            self.set_error(SculptFailure::ShadowPoisoned);
            return;
        };
        let shadow_len = shadow.len();
        drop(shadow);
        if shadow_len < pick.mesh.vertices().len()
            || pick
                .indices
                .iter()
                .any(|&index| index as usize >= shadow_len)
        {
            self.set_error(SculptFailure::ShadowShapeMismatch);
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
                pick.mesh
                    .with_sculpted_geometry(shadow.clone(), pick.indices.clone())
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
    fn take_ordered_outputs(&self) -> Result<SculptOutputSnapshot, ()> {
        let _publish = match self.publish_boundary.try_lock() {
            Ok(publish) => publish,
            Err(TryLockError::WouldBlock) => return Err(()),
            Err(TryLockError::Poisoned(_)) => {
                self.set_error(SculptFailure::WorkerStatePoisoned);
                return Err(());
            }
        };
        let mut topology_deltas = match self.topology_deltas.try_lock() {
            Ok(deltas) => deltas,
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
        let topology_deltas = std::mem::take(&mut *topology_deltas);
        let completions = std::mem::take(&mut *completions);
        let full_sync = self.full_sync.swap(false, Ordering::AcqRel);
        let touched = std::mem::take(&mut *pending);
        self.completion_wake.notify_all();
        let update =
            (full_sync || !touched.is_empty()).then_some(SculptUpdate { touched, full_sync });
        Ok((topology_deltas, completions, update))
    }
}

/// A completed stroke that is ready to become one undoable scene edit.
pub(crate) struct SculptCompletion {
    /// Mesh state before this stroke, prepared off the UI thread for undo.
    pub(crate) before: Arc<Mesh>,
    /// Mesh state after this stroke, ready for scene commit.
    pub(crate) mesh: Arc<Mesh>,
}

/// Sparse live update accumulated by the worker between UI frames.
pub(crate) struct SculptUpdate {
    pub(crate) touched: Vec<usize>,
    pub(crate) full_sync: bool,
}

/// Persistent worker for one prepared layer.
pub(crate) struct SculptWorker {
    pub(crate) layer_id: SceneMeshId,
    /// Identity of the committed layer geometry this worker is authoritative
    /// for. The UI advances it when a stroke commits changed topology.
    pub(crate) topology_id: u64,
    pub(crate) topology: PreparedSceneTopology,
    pub(crate) world_to_local: Affine3A,
    pub(crate) local_per_world: f32,
    state: Arc<WorkerState>,
    queue: Arc<SculptCommandQueue>,
    /// Avoid sending repeated hover-prime commands for the same local patch.
    last_wall_prime: Mutex<Option<(DVec3, f64)>>,
    worker_thread: Option<JoinHandle<()>>,
}

impl SculptWorker {
    pub(crate) fn spawn(session: SculptSession) -> Self {
        let layer_id = session.layer_id;
        let topology_id = session.topology_id;
        let topology = session.topology;
        let world_to_local = session.world_to_local;
        let local_per_world = session.local_per_world;
        let indices = session.session.indices().to_vec();
        let error = Arc::new(Mutex::new(None));
        let state = Arc::new(WorkerState {
            shadow: Arc::clone(&session.shadow),
            pick: Arc::new(RwLock::new(SculptPickState {
                mesh: Arc::clone(&session.base_mesh),
                shadow: Arc::clone(&session.shadow),
                dirty_triangles: Vec::new(),
                indices,
            })),
            pending_touched: Mutex::new(Vec::new()),
            full_sync: AtomicBool::new(false),
            topology_deltas: Mutex::new(VecDeque::new()),
            completions: Mutex::new(VecDeque::new()),
            publish_boundary: Mutex::new(()),
            geometry_revision: AtomicU64::new(0),
            geometry_dirty: AtomicBool::new(false),
            completion_wake: Condvar::new(),
            stopping: AtomicBool::new(false),
            error: Arc::clone(&error),
            #[cfg(test)]
            panic_on_next_command: AtomicBool::new(false),
            #[cfg(test)]
            input_trace: Mutex::new(Vec::new()),
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
                        // The shipped desktop profiles use `panic = "unwind"`,
                        // so a worker panic is converted into a typed failure
                        // instead of taking the whole viewer down. The default
                        // abort profile still keeps this guard for tests and
                        // local callers that opt into it.
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
            last_wall_prime: Mutex::new(None),
            worker_thread,
        }
    }

    /// A worker whose body demonstrably panics as soon as it picks up a
    /// command, so a test can prove the `catch_unwind` boundary in [`spawn`]
    /// converts a dead worker thread into a typed failure.
    ///
    /// The panic is raised inside `run_worker` on the worker thread, so it is
    /// the real entry guard — not a mock around it — that is being exercised.
    /// No kernel failure mode unwinds, so this test-only trigger is the only
    /// way to exercise the guard.
    ///
    /// [`spawn`]: SculptWorker::spawn
    #[cfg(test)]
    pub(crate) fn spawn_panicking(session: SculptSession) -> Self {
        let worker = Self::spawn(session);
        worker
            .state
            .panic_on_next_command
            .store(true, Ordering::Release);
        worker
    }

    /// Test-only shorthand for a ball dab with no stroke bearing.
    #[cfg(test)]
    pub(crate) fn try_apply(&self, stroke: BrushStroke, mode: BrushMode) -> bool {
        self.queue
            .push_apply(stroke, mode, SculptTip::Ball, None, DabDose::FULL)
    }

    /// Submit one transformed viewport ray sample. Queued movement coalesces
    /// to the latest endpoint while the kernel traces from its last processed
    /// ray; a false return leaves the caller's sample unconsumed.
    pub(crate) fn try_apply_ray_step(&self, step: BrushRayStep) -> bool {
        let accepted = self.queue.push_ray_step(step);
        if accepted {
            if let Ok(mut last_prime) = self.last_wall_prime.lock() {
                *last_prime = None;
            }
        }
        accepted
    }

    #[cfg(test)]
    pub(crate) fn set_queue_paused_for_tests(&self, paused: bool) {
        self.queue.set_paused_for_tests(paused);
    }

    #[cfg(test)]
    pub(crate) fn take_input_trace_for_tests(&self) -> Vec<SculptWorkerInput> {
        self.state
            .input_trace
            .lock()
            .map_or_else(|_| Vec::new(), |mut trace| std::mem::take(&mut *trace))
    }

    /// Queue an ordered ray-path reset before the next physical sample.
    pub(crate) fn try_break_ray_path(&self) -> bool {
        self.queue.push_break_path()
    }

    /// Warm a bounded local opposite-wall cache from the idle hover cursor.
    pub(crate) fn try_prime_wall_region(
        &self,
        center: DVec3,
        radius_mm: f64,
        budget: usize,
    ) -> bool {
        if !center.is_finite() || !radius_mm.is_finite() || radius_mm <= 0.0 || budget == 0 {
            return false;
        }
        if !self.is_quiescent() {
            return false;
        }
        let Ok(mut last_prime) = self.last_wall_prime.lock() else {
            return false;
        };
        if last_prime.is_some_and(|(previous, previous_radius)| {
            previous.distance_squared(center) <= (radius_mm * 0.5).powi(2)
                && (previous_radius - radius_mm).abs() <= radius_mm * 0.1
        }) {
            return true;
        }
        let accepted = self.queue.push_prime_wall_region(center, radius_mm, budget);
        if accepted {
            *last_prime = Some((center, radius_mm));
        }
        accepted
    }

    pub(crate) fn finish_stroke(&self) -> bool {
        self.queue.push_finish()
    }

    pub(crate) fn shadow(&self) -> Arc<RwLock<Vec<Vertex>>> {
        Arc::clone(&self.state.shadow)
    }

    pub(crate) fn has_uncommitted_geometry(&self) -> bool {
        self.state.geometry_dirty.load(Ordering::Acquire)
    }

    pub(crate) fn mark_geometry_committed(&self) {
        self.state.geometry_dirty.store(false, Ordering::Release);
    }

    /// Copy the latest display arrays when a prepared scene is recreated.
    /// A worker mutation in flight leaves this cold so the caller retries.
    pub(crate) fn live_geometry(&self) -> Option<(Vec<Vertex>, Vec<u32>)> {
        let before = self.state.geometry_revision.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            return None;
        }
        let pick = self.state.pick.try_read().ok()?;
        let shadow = pick.shadow.try_read().ok()?;
        let vertices = shadow.clone();
        let indices = pick.indices.clone();
        drop(shadow);
        drop(pick);
        let after = self.state.geometry_revision.load(Ordering::Acquire);
        (before == after).then_some((vertices, indices))
    }

    /// Pick the current live Sculpt surface in mesh-local coordinates. A
    /// contended read means the worker is publishing a dab; returning `None`
    /// for that frame keeps the UI non-blocking and retries on repaint.
    pub(crate) fn pick_local_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        keep: impl Fn(Vec3) -> bool,
    ) -> Option<(usize, Vec3)> {
        let pick = self.state.pick.try_read().ok()?;
        let shadow = pick.shadow.try_read().ok()?;
        pick.mesh.pick_ray_local_with_vertices(
            occluview_core::LiveRayPick::new(&shadow, &pick.dirty_triangles, origin, direction)
                .with_indices(&pick.indices),
            keep,
        )
    }

    pub(crate) fn local_triangle_normal(&self, triangle: usize) -> Option<Vec3> {
        let pick = self.state.pick.try_read().ok()?;
        let shadow = pick.shadow.try_read().ok()?;
        pick.mesh
            .triangle_normal_local_with_geometry(&shadow, &pick.indices, triangle)
    }

    /// Test-only: whether a sparse vertex update is queued and still undrained.
    /// Used to build the frame where a rebuild and a sparse update land
    /// together, without consuming either.
    #[cfg(test)]
    pub(crate) fn has_pending_sparse_update(&self) -> bool {
        self.state.full_sync.load(Ordering::Acquire)
            || self
                .state
                .pending_touched
                .try_lock()
                .is_ok_and(|touched| !touched.is_empty())
    }

    /// Test-only: queue sparse vertex ids the way a dab's outcome does, so a
    /// frame can be set up with both a rebuild and a sparse update waiting.
    #[cfg(test)]
    pub(crate) fn queue_sparse_for_tests(&self, touched: Vec<usize>) {
        self.state.record_touched(touched, Vec::new());
    }

    /// Test-only: whether a topology delta is queued and still undrained.
    #[cfg(test)]
    pub(crate) fn has_pending_topology_delta(&self) -> bool {
        self.state
            .topology_deltas
            .try_lock()
            .is_ok_and(|deltas| !deltas.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn queue_topology_delta_for_tests(&self, delta: SculptTopologyDelta) {
        self.state.record_topology(delta);
    }

    #[cfg(test)]
    pub(crate) fn take_update(&self) -> Option<SculptUpdate> {
        // Non-blocking: a contended backlog (and its full-sync flag) stays
        // queued for the next frame instead of stalling the egui frame.
        let Ok(_publish) = self.state.publish_boundary.try_lock() else {
            return None;
        };
        let Ok(mut pending) = self.state.pending_touched.try_lock() else {
            return None;
        };
        let full_sync = self.state.full_sync.swap(false, Ordering::AcqRel);
        let touched = std::mem::take(&mut *pending);
        (full_sync || !touched.is_empty()).then_some(SculptUpdate { touched, full_sync })
    }

    /// Take one pending topology delta without blocking.
    #[cfg(test)]
    pub(crate) fn try_take_topology_delta(&self) -> Result<Option<SculptTopologyDelta>, ()> {
        let _publish = self.state.publish_boundary.try_lock().map_err(|_| ())?;
        let mut deltas = self.state.topology_deltas.try_lock().map_err(|_| ())?;
        Ok(deltas.pop_front())
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

    /// Poison this worker's publication boundary, the way a panicking frame
    /// path does.
    ///
    /// Test-only: the real failure path is a panic inside a worker-side lock,
    /// which cannot be provoked from outside without poisoning one here.
    #[cfg(test)]
    #[allow(clippy::expect_used, clippy::panic)]
    pub(crate) fn poison_publication_for_tests(&self) {
        let state = Arc::clone(&self.state);
        let _ = thread::spawn(move || {
            let _guard = state.publish_boundary.lock().expect("publication lock");
            panic!("poison the sculpt publication boundary");
        })
        .join();
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
                .topology_deltas
                .try_lock()
                .is_ok_and(|deltas| deltas.is_empty())
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
        // The worker never owns this handle, so this branch is defensive: it
        // avoids a self-join if the drop ever runs on the worker thread.
        if worker_thread.thread().id() != thread::current().id() {
            let _ = worker_thread.join();
        }
    }
}

// A `#[path]` child module so the tests reach this file's private items.
#[cfg(test)]
#[path = "sculpt_worker_tests.rs"]
mod tests;
