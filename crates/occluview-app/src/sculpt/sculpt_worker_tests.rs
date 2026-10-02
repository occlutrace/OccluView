//! Tests for [`super`]: worker ordering, topology identity, pick-readiness,
//! and the retry/full-sync recovery contract.
//!
//! A `#[path]` child module of `sculpt_worker.rs`.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic
)]

use super::*;
use crate::edit_mode::{BusyFinish, EditModeCommand, EditModeController};
use crate::sculpt::sculpt_kernel::BrushSession;
use crate::sculpt::sculpt_tool::mean_uniform_scale;
use crate::sculpt::sculpt_tool::SculptTip;
use glam::{DVec3, Vec3};
use occluview_core::{Mesh, Scene, SceneMesh};
use occluview_mesh_edit::mesh_edit_buffers_from_mesh;
use std::time::{Duration, Instant};

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
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    }
}

#[test]
fn dispatch_clock_doses_elapsed_time_once_and_survives_stroke_boundaries() {
    let start = Instant::now();
    let mut clock = DabDispatchClock::default();
    assert_eq!(
        clock.next_elapsed_ms(start),
        occluview_sculpt::DWELL_FULL_DOSE_MS,
        "the first worker-applied dab matches the donor's lastDabAt=0"
    );
    assert_eq!(
        clock.next_elapsed_ms(start + Duration::from_millis(30)),
        30.0,
        "the next dose is measured between dispatch starts"
    );
    // Finish and BreakPath do not reset or advance the clock; only another
    // actual ray dispatch does.
    assert_eq!(
        clock.next_elapsed_ms(start + Duration::from_millis(80)),
        50.0
    );
    assert_eq!(
        clock.next_elapsed_ms(start + Duration::from_millis(500)),
        occluview_sculpt::DWELL_FULL_DOSE_MS,
        "long kernel intervals are capped by the kernel dose window"
    );
}

fn worker_for(mesh: &Mesh) -> SculptWorker {
    SculptWorker::spawn(session_for(mesh))
}

fn queue_face_delta(worker: &SculptWorker, triangle: u32, indices: [u32; 3]) {
    let pick = worker.state.pick.read().expect("pick state");
    let delta = SculptTopologyDelta {
        base_vertex_count: pick.shadow.read().expect("display shadow").len(),
        appended_vertices: Vec::new(),
        updated_vertices: Vec::new(),
        base_index_count: pick.indices.len(),
        live_index_count: pick.indices.len(),
        face_updates: vec![occluview_render::SculptFaceUpdate { triangle, indices }],
        dirty_triangles: vec![triangle as usize],
    };
    drop(pick);
    worker.queue_topology_delta_for_tests(delta);
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

fn a_ray_step(x: f32, mode: BrushMode, radius_mm: f32, dose: DabDose) -> BrushRayStep {
    BrushRayStep {
        origin: [x, 0.0, 5.0],
        direction: [0.0, 0.0, -1.0],
        near_mm: 0.0,
        far_mm: 10.0,
        clip_plane: None,
        radius_mm,
        strength: 0.5,
        mode,
        tip: SculptTip::Ball,
        axis: None,
        hold: dose.hold,
        preserve_skirt: false,
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
fn a_full_stroke_backlog_refuses_the_new_dab_and_keeps_the_earlier_ones() {
    let queue = SculptCommandQueue::new();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    for _ in 0..APPLY_QUEUE_CAPACITY_PER_STROKE {
        assert!(queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL));
    }
    assert!(
        !queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL),
        "a full stroke backlog refuses the new dab"
    );
    let state = queue.state.lock().expect("queue state");
    let queued = state
        .commands
        .iter()
        .filter(|command| matches!(command, SculptCommand::Apply { .. }))
        .count();
    assert_eq!(
        queued, APPLY_QUEUE_CAPACITY_PER_STROKE,
        "a refused dab must not evict a queued one"
    );
}

#[test]
fn ray_queue_preserves_parameters_and_a_path_break_between_samples() {
    let queue = SculptCommandQueue::new();
    assert!(queue.push_ray_step(a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_ray_step(a_ray_step(1.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_break_path());
    assert!(queue.push_ray_step(a_ray_step(2.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_finish());

    let state = queue.state.lock().expect("queue state");
    assert_eq!(state.commands.len(), 5);
    let mut commands = state.commands.iter();
    assert!(matches!(
        commands.next(),
        Some(SculptCommand::RayStep { step, first: true, .. }) if step.origin[0] == 0.0
    ));
    assert!(matches!(
        commands.next(),
        Some(SculptCommand::RayStep { step, first: false, .. }) if step.origin[0] == 1.0
    ));
    assert!(matches!(
        commands.next(),
        Some(SculptCommand::BreakPath { .. })
    ));
    assert!(matches!(
        commands.next(),
        Some(SculptCommand::RayStep { step, first: false, .. }) if step.origin[0] == 2.0
    ));
    assert!(matches!(commands.next(), Some(SculptCommand::Finish)));
}

#[test]
fn preserve_skirt_is_a_captured_parameter_and_prevents_cross_modifier_coalescing() {
    let queue = SculptCommandQueue::new();
    let first = a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL);
    let plain_add = a_ray_step(1.0, BrushMode::Add, 1.0, DabDose::FULL);
    let mut add = a_ray_step(2.0, BrushMode::Add, 1.0, DabDose::FULL);
    add.preserve_skirt = true;
    let mut latest_add = add.clone();
    latest_add.origin[0] = 3.0;

    assert!(queue.push_ray_step(first));
    assert!(queue.push_ray_step(plain_add));
    assert!(queue.push_ray_step(add));
    assert!(queue.push_ray_step(latest_add));
    let state = queue.state.lock().expect("queue state");
    let samples = state
        .commands
        .iter()
        .filter_map(|command| match command {
            SculptCommand::RayStep { step, .. } => Some((step.origin[0], step.preserve_skirt)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(samples, vec![(0.0, false), (1.0, false), (3.0, true)]);
}

#[test]
fn repeated_held_samples_at_one_ray_coalesce_without_preadding_dose() {
    let queue = SculptCommandQueue::new();
    let first = a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL);
    let held = a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::dwell(30.0));
    let mut latest_held = held.clone();
    latest_held.origin[0] = 0.0005;

    assert!(queue.push_ray_step(first));
    assert!(queue.push_ray_step(held));
    assert!(queue.push_ray_step(latest_held.clone()));

    let state = queue.state.lock().expect("queue state");
    assert_eq!(state.commands.len(), 2);
    assert!(matches!(
        state.commands.back(),
        Some(SculptCommand::RayStep { step, first: false, .. })
            if step.hold && step.origin[0] == 0.0005
    ));
}

#[test]
fn full_ray_backlog_refuses_a_changed_sample_without_replacing_the_tail() {
    let queue = SculptCommandQueue::new();
    for (x, mode) in [
        (0.0, BrushMode::Add),
        (1.0, BrushMode::Remove),
        (2.0, BrushMode::Smooth),
        (3.0, BrushMode::Relax),
    ] {
        assert!(queue.push_ray_step(a_ray_step(x, mode, 1.0, DabDose::FULL)));
    }
    assert!(
        !queue.push_ray_step(a_ray_step(4.0, BrushMode::Add, 1.0, DabDose::FULL)),
        "a changed-mode endpoint must wait when the per-stroke queue is full"
    );

    let state = queue.state.lock().expect("queue state");
    let samples = state
        .commands
        .iter()
        .filter_map(|command| match command {
            SculptCommand::RayStep { step, .. } => Some((step.origin[0], step.mode)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        samples,
        vec![
            (0.0, BrushMode::Add),
            (1.0, BrushMode::Remove),
            (2.0, BrushMode::Smooth),
            (3.0, BrushMode::Relax),
        ],
        "refusal leaves all previously accepted modifier boundaries intact"
    );
}

#[test]
fn a_path_break_refused_by_queue_pressure_can_be_retried_after_one_command_drains() {
    let queue = SculptCommandQueue::new();
    let stroke = a_dab();
    for _ in 0..31 {
        assert!(queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL));
        assert!(queue.push_finish());
    }
    assert!(queue.push_ray_step(a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(
        !queue.push_break_path(),
        "the last slot stays reserved for Finish"
    );
    assert!(queue.pop().is_some(), "one older command has drained");
    assert!(
        queue.push_break_path(),
        "the caller can retry the break in order"
    );

    let state = queue.state.lock().expect("queue state");
    assert!(matches!(
        state.commands.back(),
        Some(SculptCommand::BreakPath { .. })
    ));
    assert!(matches!(
        state.commands.iter().find(|command| matches!(command, SculptCommand::RayStep { .. })),
        Some(SculptCommand::RayStep { step, first: true, .. }) if step.origin[0] == 0.0
    ));
}

#[test]
fn queued_samples_with_changed_brush_mode_or_radius_are_not_coalesced() {
    let queue = SculptCommandQueue::new();
    assert!(queue.push_ray_step(a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_ray_step(a_ray_step(1.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_ray_step(a_ray_step(2.0, BrushMode::Remove, 1.0, DabDose::FULL)));
    assert!(queue.push_ray_step(a_ray_step(3.0, BrushMode::Remove, 2.0, DabDose::FULL)));

    let state = queue.state.lock().expect("queue state");
    let samples = state
        .commands
        .iter()
        .filter_map(|command| match command {
            SculptCommand::RayStep { step, .. } => {
                Some((step.origin[0], step.mode, step.radius_mm))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        samples,
        vec![
            (0.0, BrushMode::Add, 1.0),
            (1.0, BrushMode::Add, 1.0),
            (2.0, BrushMode::Remove, 1.0),
            (3.0, BrushMode::Remove, 2.0),
        ]
    );
}

#[test]
fn full_queue_still_coalesces_a_compatible_latest_travel_sample() {
    let queue = SculptCommandQueue::new();
    assert!(queue.push_prime_wall_region(DVec3::ZERO, 1.0, 1));
    let stroke = a_dab();
    for _ in 0..12 {
        for _ in 0..APPLY_QUEUE_CAPACITY_PER_STROKE {
            assert!(queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL));
        }
        assert!(queue.push_finish());
    }
    assert!(queue.push_ray_step(a_ray_step(0.0, BrushMode::Add, 1.0, DabDose::FULL)));
    assert!(queue.push_ray_step(a_ray_step(1.0, BrushMode::Add, 1.0, DabDose::FULL)));
    {
        let state = queue.state.lock().expect("queue state");
        assert_eq!(state.commands.len(), MAX_QUEUED_COMMANDS - 1);
    }
    assert!(queue.push_ray_step(a_ray_step(2.0, BrushMode::Add, 1.0, DabDose::FULL)));

    let state = queue.state.lock().expect("queue state");
    assert_eq!(state.commands.len(), MAX_QUEUED_COMMANDS - 1);
    assert!(matches!(
        state.commands.back(),
        Some(SculptCommand::RayStep { step, first: false, .. }) if step.origin[0] == 2.0
    ));
}

#[test]
fn a_full_queue_retires_only_the_finishing_strokes_own_dab() {
    let queue = SculptCommandQueue::new();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    // The real shape of queue pressure: whole strokes of four dabs and their
    // end marker, until the bound is reached.
    for _ in 0..13 {
        for _ in 0..APPLY_QUEUE_CAPACITY_PER_STROKE {
            let _ = queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL);
        }
        assert!(
            queue.push_finish(),
            "a stroke end is admitted while the queue has room"
        );
    }

    let state = queue.state.lock().expect("queue state");
    assert_eq!(state.commands.len(), MAX_QUEUED_COMMANDS);
    let first_stroke = state
        .commands
        .iter()
        .filter(
            |command| matches!(command, SculptCommand::Apply { stroke_id, .. } if *stroke_id == 1),
        )
        .count();
    assert_eq!(
        first_stroke, APPLY_QUEUE_CAPACITY_PER_STROKE,
        "admitting one stroke's end must not retire an older stroke's dab"
    );
}

#[test]
fn the_command_queue_stays_bounded_under_sustained_strokes() {
    let queue = SculptCommandQueue::new();
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    for _ in 0..32 {
        for _ in 0..APPLY_QUEUE_CAPACITY_PER_STROKE {
            let _ = queue.push_apply(stroke, BrushMode::Add, SculptTip::Ball, None, DabDose::FULL);
        }
        let admitted = queue.push_finish();
        let state = queue.state.lock().expect("queue state");
        assert!(
            state.commands.len() <= MAX_QUEUED_COMMANDS,
            "the command queue grew past its bound"
        );
        assert!(
            admitted || state.commands.len() == MAX_QUEUED_COMMANDS,
            "a stroke end is only refused by a queue that is actually full"
        );
    }
}

#[test]
fn a_rejected_new_ray_stroke_does_not_leave_a_phantom_open_stroke() {
    let queue = SculptCommandQueue::new();
    let step = a_ray_step(1000.0, BrushMode::Add, 0.75, DabDose::FULL);
    for _ in 0..(MAX_QUEUED_COMMANDS / 2) {
        assert!(
            queue.push_ray_step(step.clone()),
            "one ray opens the stroke"
        );
        assert!(queue.push_finish(), "the stroke boundary is queued");
    }

    assert!(
        !queue.push_ray_step(step),
        "a genuinely full queue must reject a new ray"
    );
    let state = queue.state.lock().expect("queue state");
    assert_eq!(state.commands.len(), MAX_QUEUED_COMMANDS);
    assert!(
        state.open_stroke.is_none(),
        "a rejected ray must not claim the next stroke id"
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
        .pick_local_ray(Vec3::new(10.25, 0.25, 10.0), -Vec3::Z, |_| true)
        .expect("the dirty live triangles must remain pickable");
    assert!(triangle < 2);
    assert!((point.x - 10.25).abs() < 1e-5);
    assert!((point.z).abs() < 1e-5);
}

#[test]
fn ordered_output_snapshot_keeps_topology_and_completion_together() {
    let worker = test_worker();
    queue_face_delta(&worker, 0, [0, 2, 1]);
    let mesh = Arc::new(coarse_ridge_mesh());
    assert!(worker.state.push_completion(SculptCompletion {
        before: Arc::clone(&mesh),
        mesh,
    }));

    let (deltas, completions, update) = worker
        .take_ordered_outputs()
        .expect("the ordered output boundary must be available");
    assert_eq!(deltas.len(), 1, "the local topology patch must be present");
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

fn wait_for_topology_delta(worker: &SculptWorker) -> SculptTopologyDelta {
    for _ in 0..2_000 {
        if let Ok(Some(delta)) = worker.try_take_topology_delta() {
            return delta;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("the densifying dab never produced a topology delta");
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
    let mut committed = 0;
    for _ in 0..4 {
        for _ in 0..16 {
            let _ = worker.try_apply(stroke, BrushMode::Add);
        }
        assert!(worker.finish_stroke());
        let completion = wait_for_completion(&worker);
        assert!(
            completion.mesh.vertices().len() >= 4,
            "every stroke must commit a layer"
        );
        committed += 1;
    }
    assert_eq!(
        committed, 4,
        "consuming one completion must not disable the next stroke"
    );
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
            .mesh = Arc::clone(&mesh);
        edit_mode.sync_to_scene(&scene);
        assert_eq!(
            edit_mode.finish_layer_edit_success(token),
            BusyFinish::Applied
        );
    }
}

/// Densification publishes the appended rows and rewired faces while the
/// committed mesh receives a new topology identity.
#[test]
fn a_densifying_stroke_publishes_local_rows_and_keeps_a_coarse_undo_baseline() {
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
    let delta = wait_for_topology_delta(&worker);
    let (live_vertices, live_indices) = worker.live_geometry().expect("live geometry");
    assert!(live_vertices.len() > original_vertices);
    assert!(live_indices.len() > mesh.indices().len());
    assert_eq!(delta.base_vertex_count, original_vertices);
    assert_eq!(delta.base_index_count, mesh.indices().len());
    assert!(!delta.appended_vertices.is_empty());
    assert!(!delta.face_updates.is_empty());
    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);

    assert!(completion.mesh.vertices().len() > original_vertices);
    assert!(completion.mesh.triangle_count() > original_triangles);
    assert_ne!(
        completion.mesh.topology_id(),
        original_topology_id,
        "a changed mesh must receive a new topology identity"
    );

    assert_eq!(completion.before.vertices().len(), original_vertices);
    assert_eq!(completion.before.triangle_count(), original_triangles);
    assert_eq!(completion.before.topology_id(), original_topology_id);
}

#[test]
fn second_ray_stroke_undo_baseline_matches_the_previous_add_commit() {
    let mesh = coarse_ridge_mesh();
    let worker = worker_for(&mesh);
    let mut add = a_ray_step(0.0, BrushMode::Add, 3.5, DabDose::FULL);
    add.strength = 1.0;

    assert!(worker.try_apply_ray_step(add));
    let delta = wait_for_topology_delta(&worker);
    assert!(
        !delta.appended_vertices.is_empty(),
        "the first Add must exercise a topology-changing ray commit"
    );
    assert!(worker.finish_stroke());
    let first = wait_for_completion(&worker);
    assert!(first.mesh.vertices().len() > mesh.vertices().len());

    let mut next_add = a_ray_step(0.0, BrushMode::Add, 3.5, DabDose::FULL);
    next_add.strength = 1.0;
    assert!(worker.try_apply_ray_step(next_add));
    assert!(worker.finish_stroke());
    let second = wait_for_completion(&worker);

    assert_eq!(
        second.before.vertices(),
        first.mesh.vertices(),
        "a later stroke's undo snapshot must preserve every committed vertex attribute"
    );
    assert_eq!(second.before.indices(), first.mesh.indices());
    assert_eq!(second.before.topology_id(), first.mesh.topology_id());
}

/// A densified layer must arrive pick-ready and stay pick-ready across the
/// commit, or no later stroke can land.
///
/// The viewport lays a dab only where the cursor hits the surface, and the
/// hit test refuses to build a scan-sized BVH on the egui thread; session
/// preparation warms the base tree while changed live faces use the local
/// dirty-face list.
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
    let delta = wait_for_topology_delta(&worker);
    assert!(!delta.dirty_triangles.is_empty());
    assert!(
        worker
            .pick_local_ray(Vec3::new(0.0, 0.0, 20.0), -Vec3::Z, |_| true)
            .is_some(),
        "the live remeshed sheet stays pickable"
    );

    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);
    assert!(completion.mesh.bvh_is_ready());
}

/// A live-remesh dab subdivides the surface through an append and face patch.
#[test]
fn a_topology_changing_dab_publishes_a_local_delta() {
    let worker = test_worker();
    assert!(worker.try_apply(a_dab(), BrushMode::Add));
    let delta = wait_for_topology_delta(&worker);
    assert!(
        !delta.appended_vertices.is_empty(),
        "the live remesh must append vertices in a facet coarser than the brush target"
    );
    assert!(!delta.face_updates.is_empty());
    assert!(worker.live_geometry().is_some());
    assert!(worker.finish_stroke());
    let completion = wait_for_completion(&worker);
    assert_ne!(
        completion.mesh.topology_id(),
        completion.before.topology_id(),
        "a topology change mints a new GPU buffer token"
    );
}

/// The worker keeps its committed buffer token until the UI accepts the
/// finished mesh, while live rows remain available for display.
#[test]
fn a_pending_topology_delta_leaves_the_worker_token_frozen() {
    let worker = test_worker();
    let frozen = worker.topology_id;
    assert!(worker.try_apply(a_dab(), BrushMode::Add));
    let delta = wait_for_topology_delta(&worker);
    assert_eq!(
        worker.topology_id, frozen,
        "the UI owns the committed topology token"
    );
    assert_eq!(delta.base_vertex_count, 4);
    assert!(worker.live_geometry().expect("live geometry").0.len() > 4);
}

/// A dirty stroke whose undo baseline was lost, and one whose display shadow no
/// longer has the kernel's shape, are terminal. Each has to latch an error the
/// UI can show and stop consuming commands: publishing either one would hand
/// the scene a mesh that no undo step can describe.
#[test]
fn terminal_finish_invariant_errors_stop_the_worker_command_loop() {
    let open_queued_stroke = |worker: &SculptWorker| {
        let Ok(mut state) = worker.queue.state.lock() else {
            panic!("worker queue state");
        };
        state.next_stroke_id = state.next_stroke_id.wrapping_add(1);
        state.open_stroke = Some(state.next_stroke_id);
    };

    // A stroke that changed geometry with nothing recorded to undo back to.
    let mut session = session_for(&test_mesh());
    session.dirty_stroke = true;
    session.stroke_start_mesh = None;
    let worker = SculptWorker::spawn(session);
    open_queued_stroke(&worker);
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
    open_queued_stroke(&worker);
    assert!(worker.finish_stroke());
    assert_eq!(
        wait_for_error(&worker),
        Some(SculptFailure::VertexCountChanged),
        "a shadow with the wrong shape must be reported, not committed"
    );
    assert_no_further_output(&worker);
}

/// The worker hands its shutdown token into the kernel, so a dab already
/// running stops instead of finishing a traversal whose result is discarded.
/// The session is dropped mid-dab on every undo, layer removal and scene
/// replace.
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
    // The same store `Drop` makes when the session goes away.
    worker.state.stopping.store(true, Ordering::Release);
    drop(held);

    for _ in 0..2_000 {
        if worker
            .worker_thread
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        worker
            .worker_thread
            .as_ref()
            .is_some_and(JoinHandle::is_finished),
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

/// A topology patch must match the current index prefix before it changes the
/// live picker or enters the frame output queue.
#[test]
fn malformed_topology_delta_is_rejected_before_publication() {
    let worker = test_worker();
    let before = worker.live_geometry().expect("base live geometry");
    worker.state.record_topology(SculptTopologyDelta {
        base_vertex_count: before.0.len(),
        appended_vertices: Vec::new(),
        updated_vertices: Vec::new(),
        base_index_count: before.1.len() + 3,
        live_index_count: before.1.len(),
        face_updates: Vec::new(),
        dirty_triangles: Vec::new(),
    });

    assert_eq!(
        worker.take_error(),
        Some(SculptFailure::ShadowShapeMismatch)
    );
    assert_eq!(
        worker.live_geometry().expect("geometry stays valid"),
        before
    );
    assert!(!worker.has_pending_topology_delta());
}

/// A panic in the worker body must not take the viewer down or leave the stroke
/// looking busy forever: the `catch_unwind` boundary in `spawn` has to latch a
/// typed failure the UI can show. The kernel cannot unwind through a normal dab,
/// so the test-only trigger panics the worker at its command boundary.
#[test]
fn worker_entry_converts_panics_to_a_visible_failure() {
    let worker = SculptWorker::spawn_panicking(session_for(&test_mesh()));

    // Any command drives the worker into its body; the panic happens there.
    assert!(worker.try_apply(a_dab(), BrushMode::Add));

    let failure = wait_for_error(&worker);
    assert!(
        matches!(failure, Some(SculptFailure::WorkerPanicked { ref message }) if !message.is_empty()),
        "a panicking worker body must latch a visible WorkerPanicked failure: {failure:?}"
    );
    assert!(
        worker.take_completion().is_none(),
        "a panicked worker publishes no completion"
    );
    assert!(
        worker.take_error().is_none(),
        "the failure is reported exactly once, not relatched forever"
    );
}

#[path = "sculpt_worker_output_tests.rs"]
mod output_tests;
