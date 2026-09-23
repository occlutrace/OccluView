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

fn session_for(mesh: &Mesh) -> SculptSession {
    let entry = SceneMesh::new(mesh.clone());
    let layer_id = entry.id();
    mesh.warm_bvh();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(mesh)).expect("prepare");
    SculptSession {
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
    }
}

fn worker_for(mesh: &Mesh) -> SculptWorker {
    SculptWorker::spawn(session_for(mesh))
}

/// The four-vertex quad every stroke test below sculpts on.
fn test_mesh() -> Mesh {
    Mesh::new(
        Some("worker-test".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh")
}

fn test_worker() -> SculptWorker {
    worker_for(&test_mesh())
}

fn a_dab() -> BrushStroke {
    BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    }
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
fn live_picker_follows_a_triangle_that_left_the_original_bvh_bounds() {
    let worker = test_worker();
    {
        let shadow = worker.shadow();
        let mut shadow = shadow.write().expect("live shadow");
        for vertex in &mut *shadow {
            vertex.position[0] += 10.0;
        }
    }
    worker.state.record_touched(vec![0, 1, 2, 3], vec![0, 1]);

    let (triangle, point) = worker
        .pick_local_ray(Vec3::new(10.25, 0.25, 10.0), -Vec3::Z)
        .expect("the dirty live triangles must remain pickable");
    assert!(triangle < 2);
    assert!((point.x - 10.25).abs() < 1e-5);
    assert!((point.z).abs() < 1e-5);
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

/// A dirty stroke whose undo baseline was lost, and one whose display shadow no
/// longer has the kernel's shape, are terminal. Each has to latch an error the
/// UI can show and stop consuming commands: publishing either one would hand
/// the scene a mesh that no undo step can describe.
#[test]
fn terminal_finish_invariant_errors_stop_the_worker_command_loop() {
    // A stroke that changed geometry with nothing recorded to undo back to.
    let mut session = session_for(&test_mesh());
    session.dirty_stroke = true;
    session.stroke_start_mesh = None;
    let worker = SculptWorker::spawn(session);
    assert!(worker.finish_stroke(), "the finish marker is accepted");
    assert_eq!(
        wait_for_error(&worker),
        Some(SculptFailure::MissingUndoBaseline),
        "a stroke with no undo boundary must be reported, not committed"
    );
    assert_no_further_output(&worker);

    // A display shadow that no longer has the shape of the kernel mesh: the
    // committed mesh would disagree with the vertices already on the GPU.
    let mesh = test_mesh();
    let mut session = session_for(&mesh);
    session.dirty_stroke = true;
    session.stroke_start_mesh = Some(Arc::new(mesh.clone()));
    session.shadow = Arc::new(RwLock::new(vec![mesh.vertices()[0]]));
    let worker = SculptWorker::spawn(session);
    assert!(worker.finish_stroke());
    assert_eq!(
        wait_for_error(&worker),
        Some(SculptFailure::VertexCountChanged),
        "a shadow with the wrong shape must be reported, not committed"
    );
    assert_no_further_output(&worker);
}

/// The worker hands its shutdown token into the kernel, so a dab already
/// running stops instead of finishing a traversal nobody will ever read. The
/// session is dropped mid-dab on every undo, layer removal and scene replace.
#[test]
fn worker_passes_its_cancellation_token_into_the_kernel() {
    let worker = test_worker();
    let shadow = worker.shadow();
    let before = shadow.read().expect("the display shadow").clone();

    // Hold the display shadow so the dab parks inside the session's baseline
    // snapshot: past the worker's own top-of-loop check, before the kernel.
    let held = shadow
        .write()
        .expect("the test holds the shadow write lock");
    assert!(worker.try_apply(a_dab(), BrushMode::Add));
    for _ in 0..2_000 {
        if worker.queue.active.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        worker.queue.active.load(Ordering::Acquire),
        "the worker has to pick the dab up"
    );
    thread::sleep(Duration::from_millis(200));
    // Exactly what `Drop` does when the session goes away.
    worker.state.stopping.store(true, Ordering::Release);
    drop(held);

    for _ in 0..2_000 {
        if worker
            .worker_thread
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        worker
            .worker_thread
            .as_ref()
            .is_some_and(|handle| handle.is_finished()),
        "a cancelled dab must let the worker stop"
    );

    let after = shadow.read().expect("the display shadow").clone();
    assert_eq!(after.len(), before.len());
    assert!(
        after
            .iter()
            .zip(&before)
            .all(|(now, then)| now.position == then.position),
        "a dab cancelled inside the kernel must not patch the display shadow"
    );
    assert!(
        worker.take_update().is_none(),
        "and must publish no sparse update"
    );
    assert!(worker.take_completion().is_none());
}

/// Wait for a terminal worker failure, or give up so a wedged worker fails the
/// test instead of hanging the suite.
fn wait_for_error(worker: &SculptWorker) -> Option<SculptFailure> {
    for _ in 0..2_000 {
        if let Some(failure) = worker.take_error() {
            return Some(failure);
        }
        thread::sleep(Duration::from_millis(1));
    }
    None
}

/// After a terminal failure the loop has stopped: a command queued behind it is
/// never consumed and never published.
fn assert_no_further_output(worker: &SculptWorker) {
    assert!(worker.try_apply(a_dab(), BrushMode::Add));
    assert!(worker.finish_stroke());
    for _ in 0..60 {
        assert!(
            worker.take_completion().is_none(),
            "a stopped worker must consume no later command"
        );
        assert!(worker.take_update().is_none());
        thread::sleep(Duration::from_millis(10));
    }
}

#[path = "sculpt_worker_output_tests.rs"]
mod output_tests;
