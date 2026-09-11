//! Tests for [`super`]: worker ordering, topology identity, pick-readiness,
//! and the retry/full-sync recovery contract.
//!
//! A `#[path]` child module of `sculpt_worker.rs`, split out to hold the
//! workspace's 800-line file budget.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic
)]

use super::*;
use crate::edit_mode::{BusyFinish, EditModeCommand, EditModeController};
use crate::sculpt_tool::mean_uniform_scale;
use glam::Vec3;
use occluview_core::{mesh_edit_buffers_from_mesh, BrushSession, Mesh, Scene, SceneMesh};
use std::time::Duration;

fn worker_for(mesh: &Mesh) -> SculptWorker {
    let entry = SceneMesh::new(mesh.clone());
    let layer_id = entry.id();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(mesh)).expect("prepare");
    SculptWorker::spawn(SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: mean_uniform_scale(&Affine3A::IDENTITY),
        dirty_stroke: false,
        stroke_start_mesh: None,
    })
}

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
    worker_for(&mesh)
}

#[test]
fn poisoned_live_shadow_stops_worker_without_publishing_a_stale_update() {
    let worker = test_worker();
    let shadow = worker.shadow();
    let poison = thread::spawn(move || {
        let _guard = shadow.write().expect("shadow lock");
        panic!("test poison");
    });
    assert!(poison.join().is_err());

    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(stroke, BrushMode::Add));
    for _ in 0..2_000 {
        if let Some(failure) = worker.take_error() {
            assert_eq!(failure, SculptFailure::ShadowPoisoned);
            assert!(worker.take_update().is_none());
            assert!(worker.take_completion().is_none());
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("poisoned shadow did not stop the sculpt worker");
}

#[test]
fn poisoned_worker_publication_is_reported_instead_of_retried_forever() {
    let worker = test_worker();
    thread::scope(|scope| {
        let poison = scope.spawn(|| {
            let _guard = worker
                .state
                .publish_boundary
                .lock()
                .expect("publication lock");
            panic!("test poison");
        });
        assert!(poison.join().is_err());
    });

    assert!(worker.take_ordered_outputs().is_err());
    assert_eq!(
        worker.take_error(),
        Some(SculptFailure::WorkerStatePoisoned)
    );
}

#[test]
fn poisoned_command_queue_is_reported_to_the_worker_owner() {
    let error = Arc::new(Mutex::new(None));
    let queue = SculptCommandQueue::with_error(Arc::clone(&error));
    thread::scope(|scope| {
        let poison = scope.spawn(|| {
            let _guard = queue.state.lock().expect("queue lock");
            panic!("test poison");
        });
        assert!(poison.join().is_err());
    });
    queue.wake.notify_one();

    assert!(queue.pop().is_none());
    assert_eq!(
        error.lock().expect("worker error").as_ref(),
        Some(&SculptFailure::WorkerStatePoisoned)
    );
}

#[test]
fn command_queue_has_a_global_bound_across_rapid_strokes() {
    let queue = SculptCommandQueue::new();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    for _ in 0..32 {
        for _ in 0..APPLY_QUEUE_CAPACITY_PER_STROKE {
            assert!(queue.push_apply(stroke, BrushMode::Add));
        }
        assert!(queue.push_finish());
    }

    let state = queue.state.lock().expect("queue state");
    assert!(
        state.commands.len() <= MAX_QUEUED_COMMANDS,
        "rapid strokes must not grow the command queue without bound"
    );
}

#[test]
fn a_rejected_new_stroke_does_not_leave_a_phantom_open_stroke() {
    let queue = SculptCommandQueue::new();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    for _ in 0..MAX_QUEUED_COMMANDS {
        assert!(queue.push_finish());
    }

    assert!(
        !queue.push_apply(stroke, BrushMode::Add),
        "a full queue of finish markers must reject a new apply"
    );
    let state = queue.state.lock().expect("queue state");
    assert!(
        state.open_stroke.is_none(),
        "a rejected apply must not claim the next stroke id"
    );
}

#[test]
fn worker_passes_its_cancellation_token_into_the_kernel() {
    let source = crate::primary_ui_tests::production_source(include_str!("sculpt_worker.rs"));
    assert!(source.contains("apply_dab_cancellable"));
    assert!(source.contains("&state.stopping"));
}

#[test]
fn terminal_finish_invariant_errors_stop_the_worker_command_loop() {
    let source = crate::primary_ui_tests::production_source(include_str!("sculpt_worker.rs"));
    let start = source
        .rfind("SculptCommand::Finish")
        .expect("the Finish command branch must exist");
    let finish = &source[start..(start + 2_000).min(source.len())];
    for failure in [
        "MissingUndoBaseline",
        "ShadowPoisoned",
        "VertexCountChanged",
    ] {
        let marker = format!("state.set_error(SculptFailure::{failure})");
        let error = finish
            .find(&marker)
            .unwrap_or_else(|| panic!("terminal failure {failure} must be reported"));
        let tail = &finish[error..(error + 600).min(finish.len())];
        assert!(
            tail.contains("queue.mark_idle();") && tail.contains("break;"),
            "terminal failure {failure} must stop command consumption"
        );
    }
}

#[test]
fn sculpt_preparation_counts_as_busy_before_the_worker_exists() {
    let source = crate::primary_ui_tests::production_source(include_str!("sculpt_tool.rs"));
    let start = source
        .find("pub(crate) fn is_busy")
        .expect("the sculpt busy predicate must exist");
    let body = &source[start..(start + 500).min(source.len())];
    assert!(
        body.contains("self.pending.is_some()"),
        "mesh edits must wait while background preparation is still pending"
    );
}

#[test]
fn ordered_output_snapshot_keeps_rebuild_and_completion_together() {
    let worker = test_worker();
    let mesh = coarse_ridge_mesh();
    let topology = PreparedSceneTopology::from_mesh(&mesh);
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology,
            mesh: mesh.clone(),
        },
    );
    assert!(worker.state.push_completion(SculptCompletion {
        before: Arc::new(mesh.clone()),
        mesh,
    }));

    let (rebuilds, completions, update) = worker
        .take_ordered_outputs()
        .expect("the ordered output boundary must be available");
    assert_eq!(
        rebuilds.len(),
        1,
        "the topology replacement must be present"
    );
    assert_eq!(
        completions.len(),
        1,
        "the matching completion must be present"
    );
    assert!(update.is_none(), "the fixture has no sparse update");
    assert!(worker
        .take_ordered_outputs()
        .expect("the boundary must remain usable")
        .0
        .is_empty());
}

/// A 5x3 lattice at 4mm spacing folded along a sharp ridge — far coarser
/// than the 3.5mm brush below, so a Smooth dab has to densify before it can
/// relax anything.
fn coarse_ridge_mesh() -> Mesh {
    let mut vertices = Vec::new();
    for j in 0..3usize {
        for i in 0..5usize {
            let x = i as f32 * 4.0 - 8.0;
            let y = j as f32 * 4.0 - 4.0;
            let z = if j == 1 { 4.0 } else { 0.0 };
            vertices.push(Vertex::at(Vec3::new(x, y, z)));
        }
    }
    let mut indices = Vec::new();
    let idx = |i: usize, j: usize| (j * 5 + i) as u32;
    for j in 0..2usize {
        for i in 0..4usize {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Mesh::new(Some("coarse-ridge".to_string()), vertices, indices).expect("ridge mesh")
}

fn wait_for_rebuild(worker: &SculptWorker) -> SculptRebuild {
    for _ in 0..2_000 {
        if let Ok(Some(rebuild)) = worker.try_take_rebuild() {
            return rebuild;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("the densifying dab never produced a layer rebuild");
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
            .mesh = Arc::new(mesh);
        edit_mode.sync_to_scene(&scene);
        assert_eq!(
            edit_mode.finish_layer_edit_success(token),
            BusyFinish::Applied
        );
    }
}

/// Densification changes the topology, and the ids must say so honestly:
/// the rebuilt layer gets a FRESH `topology_id` (the renderer's cue to drop
/// its exactly-sized buffers), while the undo baseline keeps the PRE-stroke
/// identity and the pre-stroke triangle list.
#[test]
fn a_densifying_stroke_mints_a_new_topology_id_and_keeps_a_coarse_undo_baseline() {
    let mesh = coarse_ridge_mesh();
    let original_vertices = mesh.vertices().len();
    let original_triangles = mesh.triangle_count();
    let original_topology_id = mesh.topology_id();
    let worker = worker_for(&mesh);
    let stroke = BrushStroke {
        center: [0.0, 0.0, 4.0],
        radius_mm: 3.5,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(stroke, BrushMode::Smooth));
    let rebuild = wait_for_rebuild(&worker);
    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);

    // The rebuilt layer really grew, and its token describes ITSELF — a
    // mismatch here is exactly how a stale GPU buffer gets written.
    assert!(rebuild.mesh.vertices().len() > original_vertices);
    assert!(rebuild.mesh.triangle_count() > original_triangles);
    assert_ne!(
        rebuild.mesh.topology_id(),
        original_topology_id,
        "a grown mesh must NOT reuse the frozen sculpt topology id"
    );
    assert_eq!(
        rebuild.topology,
        PreparedSceneTopology::from_mesh(&rebuild.mesh)
    );

    // Undo goes back to the coarse mesh, not to the dense one with old
    // coordinates.
    assert_eq!(completion.before.vertices().len(), original_vertices);
    assert_eq!(completion.before.triangle_count(), original_triangles);
    assert_eq!(completion.before.topology_id(), original_topology_id);

    // The committed mesh matches the geometry already on the GPU, so the
    // commit is a content swap and not another re-upload.
    assert_eq!(
        completion.mesh.vertices().len(),
        rebuild.mesh.vertices().len()
    );
    assert_eq!(completion.mesh.indices(), rebuild.mesh.indices());
    assert_eq!(completion.mesh.topology_id(), rebuild.mesh.topology_id());
}

/// A densified layer must arrive PICK-READY, and stay pick-ready across the
/// commit — or the brush dies for good.
///
/// The failure chain: the viewport lays a dab only where the cursor
/// hits the surface, the hit test refuses to build a scan-sized BVH on the
/// egui thread, and the only thing that ever warmed one was the session
/// preparation. A densifying dab swapped in a rebuilt mesh with a cold BVH,
/// the session still "matched" so no re-preparation ever ran, and from that
/// moment every stroke on the layer found no surface and silently did
/// nothing. First stroke worked, everything after it was dead.
#[test]
fn a_densified_layer_is_still_pickable_so_the_next_stroke_can_land() {
    let mesh = coarse_ridge_mesh();
    let worker = worker_for(&mesh);
    let stroke = BrushStroke {
        center: [0.0, 0.0, 4.0],
        radius_mm: 3.5,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(stroke, BrushMode::Smooth));
    let rebuild = wait_for_rebuild(&worker);
    assert!(
        rebuild.mesh.bvh_is_ready(),
        "the rebuilt layer goes into the scene as-is; a cold BVH there kills \
         the hit test, and nothing downstream ever warms it again"
    );

    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);
    assert!(
        completion.mesh.bvh_is_ready(),
        "the committed mesh replaces the layer after the stroke; it has to \
         stay pick-ready or the SECOND stroke is the one that dies"
    );
}

/// The un-densified path is untouched: a stroke that changes no topology
/// still streams sparsely and still freezes the topology id.
#[test]
fn a_stroke_that_does_not_densify_still_freezes_the_topology_id() {
    let worker = test_worker();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(stroke, BrushMode::Add));
    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_none(),
        "Add must not densify"
    );
    assert_eq!(completion.mesh.vertices().len(), 4);
    assert_eq!(
        completion.mesh.topology_id(),
        completion.before.topology_id(),
        "a positions-only sculpt keeps the GPU buffer token frozen"
    );
}

/// P1 lost-update regression: contention on the frame path must not drop
/// the newest CPU state. A drained-but-unapplied update is restored, so a
/// later poll applies authoritative latest state. Non-blocking throughout.
#[test]
fn drained_update_is_not_lost_on_shadow_contention() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2]);
    let write_guard = worker
        .state
        .shadow
        .write()
        .expect("test holds the shadow write lock");
    let drained = worker.take_update().expect("pending update must drain");
    assert!(
        worker.state.shadow.try_read().is_err(),
        "held write lock must make the flush unavailable"
    );
    // Fixed `flush_sculpt_update` path: restore instead of dropping.
    worker.restore_update(drained);
    drop(write_guard);
    let retry = worker.take_update();
    assert!(
        retry.is_some(),
        "lost update: a drained-but-unapplied sculpt update vanished; \
         contention must leave a pending retry or authoritative full-sync"
    );
    let retry = retry.expect("checked above");
    assert!(
        retry.full_sync || !retry.touched.is_empty(),
        "retry must carry authoritative latest state"
    );
}

/// Quiescence must count undrained worker output, not just the command
/// queue: Done/undo gate on it, and a false quiet lets the session
/// invalidate out from under unflushed deltas.
#[test]
fn quiescence_counts_undrained_deltas_not_just_the_queue() {
    let worker = test_worker();
    // No commands queued; a restored (drained-but-unapplied) update is
    // still live work the next poll owes a flush.
    worker.restore_update(SculptUpdate {
        touched: vec![0],
        full_sync: false,
    });
    assert!(
        !worker.is_quiescent(),
        "a restored update must read as pending work"
    );
    let _ = worker.take_update();
    assert!(worker.is_quiescent(), "a drained worker must read quiet");
}

/// Same for the full-sync flag: an authoritative resync owed to the GPU
/// is pending work even with an empty queue and no touched ids.
#[test]
fn quiescence_counts_a_pending_full_sync() {
    let worker = test_worker();
    worker.request_full_sync();
    assert!(
        !worker.is_quiescent(),
        "an owed full sync must read as pending work"
    );
    let update = worker.take_update().expect("the flag must drain");
    assert!(update.full_sync);
    assert!(worker.is_quiescent());
}

/// Same for a densify rebuild: no Finish, no drain, yet the new topology
/// (and its touches) is still owed to the UI.
#[test]
fn quiescence_counts_a_pending_layer_rebuild() {
    let worker = worker_for(&coarse_ridge_mesh());
    let dab = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 3.5,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(dab, BrushMode::Smooth));
    thread::sleep(Duration::from_millis(200));
    assert!(
        !worker.is_quiescent(),
        "an undrained densify rebuild must read as pending work"
    );
    worker
        .try_take_rebuild()
        .expect("the densifying dab must have produced a layer rebuild");
}

/// Coalescing keeps only the newest state: overflow escalates to one
/// authoritative full sync instead of an unbounded delta backlog.
#[test]
fn touched_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker
        .state
        .record_touched(vec![0; MAX_PENDING_TOUCHES + 1]);
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(
        update.full_sync,
        "pending-touch overflow must become an authoritative full sync"
    );
}

/// A contended backlog defers the drain instead of blocking the frame: the
/// pending ids and the full-sync flag stay queued for the next poll.
#[test]
fn take_update_defers_when_backlog_locked() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2]);
    worker.state.request_full_sync();
    let held = worker
        .state
        .pending_touched
        .lock()
        .expect("test holds the backlog lock");
    assert!(
        worker.take_update().is_none(),
        "a contended drain must defer, not block or half-drain"
    );
    drop(held);
    let retry = worker.take_update().expect("deferred update must redrain");
    assert!(
        retry.full_sync,
        "the full-sync flag must survive the deferred drain"
    );
}

/// A contended rebuild slot defers the drain the same way.
#[test]
fn take_rebuild_defers_when_slot_locked() {
    let worker = test_worker();
    let mesh = Mesh::new(
        Some("rebuild-defer".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    let held = worker
        .state
        .rebuild
        .lock()
        .expect("test holds the rebuild lock");
    assert!(
        worker.try_take_rebuild().is_err(),
        "a contended rebuild drain must defer"
    );
    drop(held);
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the deferred rebuild must redrain"
    );
}

/// A densifying rebuild supersedes queued sparse ids, which index the
/// pre-rebuild array. Rebuilds themselves stay ordered so every topology
/// transition remains available to the UI-side completion chain.
#[test]
fn rebuild_supersedes_queued_sparse_updates() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2]);
    let mesh = Mesh::new(
        Some("rebuild-supersede".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    assert!(
        worker.take_update().is_none(),
        "stale sparse ids must not survive a topology rebuild"
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the authoritative rebuild must remain queued"
    );
}

#[test]
fn rebuild_queue_preserves_every_topology_transition_in_order() {
    let worker = test_worker();
    let first = Mesh::new(
        Some("rebuild-first".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("first rebuild mesh");
    let second = Mesh::new(
        Some("rebuild-second".to_string()),
        vec![
            Vertex::at(Vec3::new(-2.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-2.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("second rebuild mesh");
    let first_id = first.topology_id();
    let second_id = second.topology_id();
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&first),
            mesh: first,
        },
    );
    worker.state.record_rebuild(
        2,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&second),
            mesh: second,
        },
    );
    assert_eq!(
        worker
            .try_take_rebuild()
            .expect("first rebuild")
            .expect("first rebuild present")
            .mesh
            .topology_id(),
        first_id
    );
    assert_eq!(
        worker
            .try_take_rebuild()
            .expect("second rebuild")
            .expect("second rebuild present")
            .mesh
            .topology_id(),
        second_id
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_none(),
        "the FIFO must be fully drained"
    );
}

/// Restoring a drained update is a lossless round-trip: the next drain
/// returns the same sparse ids when no rebuild intervened.
#[test]
fn restored_update_redrains_identical_sparse_ids() {
    let worker = test_worker();
    worker.state.record_touched(vec![3, 1, 2, 1]);
    let drained = worker.take_update().expect("pending update must drain");
    worker.restore_update(drained);
    let retry = worker.take_update().expect("restored update must redrain");
    assert!(!retry.full_sync, "sparse restore must not escalate");
    let mut ids = retry.touched;
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids, vec![1, 2, 3]);
}

/// A restore that would overflow the backlog escalates to a full sync at the
/// same site as the record path, not just in `record_touched`.
#[test]
fn restore_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker.restore_update(SculptUpdate {
        touched: vec![0; MAX_PENDING_TOUCHES + 1],
        full_sync: false,
    });
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(
        update.full_sync,
        "restore overflow must become an authoritative full sync"
    );
}

/// A drained sparse update restored after a rebuild queued must not come
/// back as stale ids: it escalates to a full sync of the latest shadow.
#[test]
fn restore_after_rebuild_escalates_to_full_sync() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2]);
    let drained = worker.take_update().expect("pending update must drain");
    let mesh = Mesh::new(
        Some("restore-rebuild".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    worker.restore_update(drained);
    assert!(
        worker.take_update().is_none(),
        "a stale sparse restore must stay behind the queued topology rebuild"
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the authoritative rebuild must be drained before its full sync"
    );
    let retry = worker.take_update().expect("restore must stay visible");
    assert!(
        retry.full_sync,
        "post-rebuild restore must escalate, never resurrect stale ids"
    );
}
