//! Background execution for interactive sculpting.
//!
//! The viewport must never wait for a mesh kernel. This module owns the
//! bounded command queue and the worker-side [`SculptSession`]; the UI only
//! submits the newest brush samples and drains sparse GPU updates/completions.

use crate::sculpt_tool::SculptSession;
use glam::Affine3A;
use occluview_core::{BrushMode, BrushStroke, Mesh, SceneMeshId, Vertex};
use occluview_render::PreparedSceneTopology;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;

const APPLY_QUEUE_CAPACITY_PER_STROKE: usize = 4;
const MAX_PENDING_TOUCHES: usize = 250_000;

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
}

impl SculptCommandQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                commands: VecDeque::new(),
                shutdown: false,
                next_stroke_id: 0,
                open_stroke: None,
            }),
            wake: Condvar::new(),
            active: AtomicBool::new(false),
        }
    }

    /// Keep each stroke's APPLY backlog bounded by replacing its oldest queued
    /// dab when the worker is busy. The stroke id is essential: a global cap
    /// would evict all dabs between two Finish markers when the operator makes
    /// two quick strokes, leaving the second stroke with no geometry to apply.
    fn push_apply(&self, stroke: BrushStroke, mode: BrushMode) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.shutdown {
            return false;
        }
        let stroke_id = if let Some(stroke_id) = state.open_stroke {
            stroke_id
        } else {
            state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
            let stroke_id = state.next_stroke_id;
            state.open_stroke = Some(stroke_id);
            stroke_id
        };
        let queued_applies = state
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
            .count();
        if queued_applies >= APPLY_QUEUE_CAPACITY_PER_STROKE {
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
            return false;
        };
        if state.shutdown {
            return false;
        }
        let stroke_id = state.open_stroke.take().unwrap_or_else(|| {
            state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
            state.next_stroke_id
        });
        // Never evict an Apply here. Finish is a stroke boundary; evicting an
        // Apply without knowing which stroke owns it was the reason rapid
        // second strokes vanished.
        state
            .commands
            .push_back(SculptCommand::Finish { stroke_id });
        self.wake.notify_one();
        true
    }

    fn pop(&self) -> Option<SculptCommand> {
        let Ok(mut state) = self.state.lock() else {
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
            state = match self.wake.wait(state) {
                Ok(state) => state,
                Err(_) => return None,
            };
        }
    }

    fn shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.shutdown = true;
            state.commands.clear();
            self.wake.notify_one();
        }
    }

    fn mark_idle(&self) {
        self.active.store(false, Ordering::Release);
    }

    fn is_empty(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.commands.is_empty() && !self.active.load(Ordering::Acquire))
            .unwrap_or(true)
    }
}

struct WorkerState {
    shadow: Arc<RwLock<Vec<Vertex>>>,
    pending_touched: Mutex<Vec<usize>>,
    full_sync: AtomicBool,
    completions: Mutex<VecDeque<SculptCompletion>>,
    error: Mutex<Option<String>>,
}

impl WorkerState {
    fn record_touched(&self, touched: Vec<usize>) {
        if touched.is_empty() || self.full_sync.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut pending) = self.pending_touched.lock() else {
            self.full_sync.store(true, Ordering::Release);
            return;
        };
        pending.extend(touched);
        if pending.len() > MAX_PENDING_TOUCHES {
            pending.clear();
            self.full_sync.store(true, Ordering::Release);
        }
    }

    fn push_completion(&self, completion: SculptCompletion) {
        if let Ok(mut completions) = self.completions.lock() {
            completions.push_back(completion);
        }
    }

    fn set_error(&self, message: String) {
        if let Ok(mut error) = self.error.lock() {
            *error = Some(message);
        }
    }
}

/// A completed stroke that is ready to become one undoable scene edit.
pub(crate) struct SculptCompletion {
    /// Mesh state before this stroke, prepared off the UI thread for undo.
    pub(crate) before: Mesh,
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
    pub(crate) topology_id: u64,
    pub(crate) topology: PreparedSceneTopology,
    pub(crate) world_to_local: Affine3A,
    pub(crate) local_per_world: f32,
    state: Arc<WorkerState>,
    queue: Arc<SculptCommandQueue>,
}

impl SculptWorker {
    pub(crate) fn spawn(session: SculptSession) -> Self {
        let layer_id = session.layer_id;
        let topology_id = session.topology_id;
        let topology = session.topology;
        let world_to_local = session.world_to_local;
        let local_per_world = session.local_per_world;
        let state = Arc::new(WorkerState {
            shadow: Arc::clone(&session.shadow),
            pending_touched: Mutex::new(Vec::new()),
            full_sync: AtomicBool::new(false),
            completions: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
        });
        let queue = Arc::new(SculptCommandQueue::new());
        let worker_queue = Arc::clone(&queue);
        let worker_state = Arc::clone(&state);
        let pool_threads = thread::available_parallelism()
            .map(|count| count.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(pool_threads)
            .thread_name(|index| format!("occluview-sculpt-kernel-{index}"))
            .build();
        match pool {
            Ok(pool) => {
                let spawn_result = thread::Builder::new()
                    .name("occluview-sculpt-worker".to_string())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_worker(session, worker_queue.clone(), worker_state.clone(), pool);
                        }));
                        if let Err(payload) = result {
                            worker_state.set_error(format!(
                                "sculpt worker panicked: {}",
                                panic_message(payload)
                            ));
                            worker_queue.mark_idle();
                        }
                    });
                if let Err(error) = spawn_result {
                    state.set_error(format!("could not start sculpt worker: {error}"));
                }
            }
            Err(error) => {
                state.set_error(format!("could not create sculpt kernel pool: {error}"));
            }
        }
        Self {
            layer_id,
            topology_id,
            topology,
            world_to_local,
            local_per_world,
            state,
            queue,
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

    pub(crate) fn take_update(&self) -> Option<SculptUpdate> {
        let full_sync = self.state.full_sync.swap(false, Ordering::AcqRel);
        let touched = self
            .state
            .pending_touched
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        (full_sync || !touched.is_empty()).then_some(SculptUpdate { touched, full_sync })
    }

    pub(crate) fn take_completion(&self) -> Option<SculptCompletion> {
        self.state
            .completions
            .lock()
            .ok()
            .and_then(|mut completions| completions.pop_front())
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.state
            .error
            .lock()
            .ok()
            .and_then(|mut error| error.take())
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.queue.is_empty()
            && self
                .state
                .completions
                .lock()
                .map(|completions| completions.is_empty())
                .unwrap_or(false)
    }
}

impl Drop for SculptWorker {
    fn drop(&mut self) {
        self.queue.shutdown();
    }
}

fn run_worker(
    mut session: SculptSession,
    queue: Arc<SculptCommandQueue>,
    state: Arc<WorkerState>,
    pool: rayon::ThreadPool,
) {
    while let Some(command) = queue.pop() {
        match command {
            SculptCommand::Apply {
                stroke_id: _stroke_id,
                stroke,
                mode,
            } => {
                let touched = pool.install(|| session.apply_dab(stroke, mode));
                state.record_touched(touched);
            }
            SculptCommand::Finish {
                stroke_id: _stroke_id,
            } => {
                let dirty = session.dirty_stroke;
                session.dirty_stroke = false;
                let start_vertices = session.stroke_start_vertices.take();
                if dirty {
                    let Some(start_vertices) = start_vertices else {
                        state.set_error("sculpt stroke has no undo baseline".to_string());
                        queue.mark_idle();
                        continue;
                    };
                    let Ok(shadow) = session.shadow.read() else {
                        state.set_error("sculpt shadow lock was poisoned".to_string());
                        queue.mark_idle();
                        continue;
                    };
                    let vertices = shadow.clone();
                    let before = session.base_mesh.with_sculpted_vertices(start_vertices);
                    let mesh = session.base_mesh.with_sculpted_vertices(vertices);
                    if let (Some(before), Some(mesh)) = (before, mesh) {
                        state.push_completion(SculptCompletion { before, mesh });
                    } else {
                        state.set_error("sculpt result changed the vertex count".to_string());
                    }
                }
            }
        }
        queue.mark_idle();
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

    use super::*;
    use crate::edit_mode::{BusyFinish, EditModeCommand, EditModeController};
    use crate::sculpt_tool::mean_uniform_scale;
    use glam::Vec3;
    use occluview_core::{mesh_edit_buffers_from_mesh, BrushSession, Mesh, Scene, SceneMesh};
    use std::time::Duration;

    fn test_worker() -> SculptWorker {
        let mesh = Mesh::new(
            Some("worker-test".to_string()),
            vec![
                Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
                Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("test mesh");
        let entry = SceneMesh::new(mesh.clone());
        let layer_id = entry.id();
        let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
        SculptWorker::spawn(SculptSession {
            layer_id,
            topology_id: mesh.topology_id(),
            session: brush,
            base_mesh: mesh.clone(),
            shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
            topology: PreparedSceneTopology::from_mesh(&mesh),
            world_to_local: Affine3A::IDENTITY,
            local_per_world: mean_uniform_scale(&Affine3A::IDENTITY),
            dirty_stroke: false,
            stroke_start_vertices: None,
        })
    }

    fn wait_for_completions(worker: &SculptWorker, expected: usize) -> usize {
        let mut completed = 0;
        for _ in 0..2_000 {
            if worker.take_completion().is_some() {
                completed += 1;
                if completed == expected {
                    return completed;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        completed
    }

    fn wait_for_completion(worker: &SculptWorker) -> SculptCompletion {
        for _ in 0..2_000 {
            if let Some(completion) = worker.take_completion() {
                return completion;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("sculpt worker did not complete the stroke");
    }

    #[test]
    fn worker_accepts_two_ordered_strokes_without_repreparing() {
        let worker = test_worker();
        let stroke = BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        };
        assert!(worker.try_apply(stroke, BrushMode::Add));
        assert!(worker.finish_stroke());
        assert!(worker.try_apply(stroke, BrushMode::Add));
        assert!(worker.finish_stroke());
        assert_eq!(wait_for_completions(&worker, 2), 2);
    }

    #[test]
    fn rapid_strokes_keep_a_dab_after_each_finish_barrier() {
        let worker = test_worker();
        let stroke = BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        };
        for _ in 0..4 {
            for _ in 0..32 {
                let _ = worker.try_apply(stroke, BrushMode::Add);
            }
            assert!(worker.finish_stroke());
        }
        assert_eq!(wait_for_completions(&worker, 4), 4);
    }

    #[test]
    fn consuming_each_completion_does_not_disable_the_next_stroke() {
        let worker = test_worker();
        let stroke = BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        };
        for _ in 0..4 {
            for _ in 0..16 {
                let _ = worker.try_apply(stroke, BrushMode::Add);
            }
            assert!(worker.finish_stroke());
            let completion = wait_for_completion(&worker);
            assert_eq!(completion.mesh.vertices().len(), 4);
        }
    }

    #[test]
    fn repeated_completions_survive_scene_and_edit_state_commit() {
        let worker = test_worker();
        let stroke = BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        };
        let mesh = Mesh::new(
            Some("scene-commit-test".to_string()),
            vec![
                Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
                Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("scene mesh");
        let entry = SceneMesh::new(mesh);
        let layer_id = entry.id();
        let mut scene = Scene::new();
        scene.add(entry);
        let mut edit_mode = EditModeController::new(8, 1_000_000);

        for _ in 0..4 {
            assert!(worker.try_apply(stroke, BrushMode::Add));
            assert!(worker.finish_stroke());
            let SculptCompletion { before, mesh } = wait_for_completion(&worker);
            let current = scene
                .meshes()
                .iter()
                .find(|entry| entry.id() == layer_id)
                .expect("scene layer")
                .clone();
            let token = edit_mode
                .begin_layer_edit_with_snapshot(&current, before, EditModeCommand::Sculpt)
                .expect("edit state accepts the next completion");
            scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
                .expect("scene layer")
                .mesh = mesh;
            edit_mode.sync_to_scene(&scene);
            assert_eq!(
                edit_mode.finish_layer_edit_success(token),
                BusyFinish::Applied
            );
        }
    }
}
