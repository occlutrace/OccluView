use super::{
    clamp_bridge_split_kerf_mm, next_nonzero_session_id, BridgeSplitController, BridgeSplitGuard,
    BridgeSplitJobInput, BridgeSplitJobOutput, BridgeSplitMode, BridgeSplitPose,
    BridgeSplitSession, BridgeSplitTarget, BridgeSplitToolError, BridgeSplitWorker,
    DEFAULT_BRIDGE_SPLIT_KERF_MM, MAX_BRIDGE_SPLIT_KERF_MM, MIN_BRIDGE_SPLIT_KERF_MM,
};
use glam::{Affine3A, Vec3};
use occluview_core::{
    BridgeSplitError, BridgeSplitReport, BridgeSplitRequest, CoreBridgeSplitResult, Mesh, Scene,
    SceneMesh,
};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[test]
fn default_kerf_and_clamp_use_one_source_of_truth() {
    let session = BridgeSplitSession::default();
    assert_eq!(
        session.kerf_mm().to_bits(),
        DEFAULT_BRIDGE_SPLIT_KERF_MM.to_bits()
    );
    assert_eq!(DEFAULT_BRIDGE_SPLIT_KERF_MM.to_bits(), 0.05_f32.to_bits());
    assert_eq!(
        clamp_bridge_split_kerf_mm(-1.0).to_bits(),
        MIN_BRIDGE_SPLIT_KERF_MM.to_bits()
    );
    assert_eq!(
        clamp_bridge_split_kerf_mm(5.0).to_bits(),
        MAX_BRIDGE_SPLIT_KERF_MM.to_bits()
    );
}

#[test]
fn session_id_increment_wraps_without_producing_zero() {
    assert_eq!(next_nonzero_session_id(0), 1);
    assert_eq!(next_nonzero_session_id(1), 2);
    assert_eq!(next_nonzero_session_id(u64::MAX), 1);
}

#[test]
fn completed_preview_preserves_the_operator_selected_disc_radius() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let guard = session
        .plant(sample_pose(4.0))
        .unwrap_or(sample_guard(1, 0, target));

    assert!(session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard,
            result: Ok(sample_result(9.0)),
        },
    ));

    assert_eq!(
        session.pose().map(|pose| pose.radius_mm.to_bits()),
        Some(4.0_f32.to_bits()),
        "a preview may report the minimum viable size, but must not move or resize the disc the operator placed"
    );
}

#[test]
fn session_request_uses_the_current_operator_disc_radius() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let _ = session
        .plant(sample_pose(4.0))
        .unwrap_or(sample_guard(1, 0, target));

    assert_eq!(
        session
            .current_request()
            .map(|request| request.disc_radius_mm.to_bits()),
        Some(4.0_f32.to_bits())
    );

    let _ = session
        .update_pose(sample_pose(12.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(
        session
            .current_request()
            .map(|request| request.disc_radius_mm.to_bits()),
        Some(12.0_f32.to_bits())
    );
}

#[test]
fn state_source_has_no_renderer_imports_or_file_level_dead_code_allow() {
    let source = include_str!("state.rs");

    assert!(!source.contains("egui"));
    assert!(!source.contains("wgpu"));
    assert!(!source.contains("#![allow(dead_code)]"));
}

#[test]
fn start_captures_stable_target_without_scene_mutation() {
    let scene = sample_scene();
    let entry = &scene.meshes()[0];
    let original_id = entry.id();
    let original_topology = entry.mesh.topology_id();
    let original_transform = entry.transform;

    let mut session = BridgeSplitSession::default();
    session.start(BridgeSplitTarget::capture(entry));
    assert!(session.set_follow_pose(Some(sample_pose(7.0))));

    assert_eq!(session.mode(), BridgeSplitMode::Following);
    assert_eq!(session.target(), Some(BridgeSplitTarget::capture(entry)));
    assert_eq!(session.pose(), Some(sample_pose(7.0)));
    assert_eq!(scene.meshes()[0].id(), original_id);
    assert_eq!(scene.meshes()[0].mesh.topology_id(), original_topology);
    assert_eq!(scene.meshes()[0].transform, original_transform);
}

#[test]
fn submit_rejects_wrong_layer_even_with_matching_mesh_and_transform() {
    let entry = sample_entry();
    let mut controller = planted_controller(&entry);
    let wrong_layer = SceneMesh::new(entry.mesh.clone()).with_transform(entry.transform);
    assert_ne!(wrong_layer.id(), entry.id());
    assert_eq!(wrong_layer.mesh.topology_id(), entry.mesh.topology_id());
    assert_eq!(wrong_layer.transform, entry.transform);

    assert!(!submit_scene_entry(&mut controller, &wrong_layer));
}

#[test]
fn submit_rejects_wrong_topology_on_same_layer() {
    let entry = sample_entry();
    let mut controller = planted_controller(&entry);
    let mut wrong_topology = entry.clone();
    wrong_topology.mesh = sample_mesh();
    assert_eq!(wrong_topology.id(), entry.id());
    assert_ne!(wrong_topology.mesh.topology_id(), entry.mesh.topology_id());
    assert_eq!(wrong_topology.transform, entry.transform);

    assert!(!submit_scene_entry(&mut controller, &wrong_topology));
}

#[test]
fn submit_rejects_wrong_transform_on_same_layer_and_topology() {
    let entry = sample_entry();
    let mut controller = planted_controller(&entry);
    let wrong_transform = entry
        .clone()
        .with_transform(Affine3A::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    assert_eq!(wrong_transform.id(), entry.id());
    assert_eq!(wrong_transform.mesh.topology_id(), entry.mesh.topology_id());
    assert_ne!(wrong_transform.transform, entry.transform);

    assert!(!submit_scene_entry(&mut controller, &wrong_transform));
}

#[test]
fn job_source_contract_uses_arc_mesh_and_moves_queued_input() {
    let source = include_str!("job.rs");

    assert!(source.contains("mesh: Arc<Mesh>"));
    assert!(!source.contains("send_to_worker(next.clone())"));
}

#[test]
fn submits_share_snapshot_until_direct_restart_replaces_it() {
    let timeout = Duration::from_secs(5);
    let entry = sample_entry();
    let target = BridgeSplitTarget::capture(&entry);
    let (started_tx, started_rx) = mpsc::channel::<Arc<Mesh>>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut controller =
        BridgeSplitController::with_worker(BridgeSplitWorker::spawn_with_compute({
            let release_rx = Arc::clone(&release_rx);
            move |input| {
                let _ = started_tx.send(Arc::clone(&input.mesh));
                if let Ok(receiver) = release_rx.lock() {
                    let _ = receiver.recv();
                }
                Ok(sample_result(input.request.max_disc_radius_mm))
            }
        }));
    controller.start(&entry);
    let _ = controller
        .session_mut()
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));

    assert!(submit_scene_entry(&mut controller, &entry));
    let first_snapshot = started_rx.recv_timeout(timeout);
    assert!(release_tx.send(()).is_ok());
    assert!(poll_controller_until(&mut controller, Some(target)));

    let _ = controller
        .session_mut()
        .update_pose(sample_pose(9.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert!(submit_scene_entry(&mut controller, &entry));
    let second_snapshot = started_rx.recv_timeout(timeout);
    assert!(matches!(
        (&first_snapshot, &second_snapshot),
        (Ok(first), Ok(second)) if Arc::ptr_eq(first, second)
    ));

    assert!(release_tx.send(()).is_ok());
    assert!(poll_controller_until(&mut controller, Some(target)));

    controller.start(&entry);
    let _ = controller
        .session_mut()
        .plant(sample_pose(10.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert!(submit_scene_entry(&mut controller, &entry));
    let restarted_snapshot = started_rx.recv_timeout(timeout);
    assert!(matches!(
        (&second_snapshot, &restarted_snapshot),
        (Ok(second), Ok(restarted)) if !Arc::ptr_eq(second, restarted)
    ));

    assert!(release_tx.send(()).is_ok());
    assert!(poll_controller_until(&mut controller, Some(target)));
}

#[test]
fn pose_and_thickness_changes_increment_generation_and_invalidate_apply() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);

    let first_guard = session
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(first_guard.generation, 1);
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);

    assert!(session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: first_guard,
            result: Ok(sample_result(9.0)),
        },
    ));
    assert!(session.can_apply());

    let second_guard = session
        .update_pose(sample_pose(10.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(second_guard.session_id, first_guard.session_id);
    assert_eq!(second_guard.generation, 2);
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);
    assert!(!session.can_apply());
    assert!(session.preview().is_none());

    let third_guard = session
        .set_kerf_mm(2.5)
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(third_guard.session_id, first_guard.session_id);
    assert_eq!(third_guard.generation, 3);
    assert_eq!(
        session.kerf_mm().to_bits(),
        MAX_BRIDGE_SPLIT_KERF_MM.to_bits()
    );
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);
}

#[test]
fn only_latest_generation_becomes_ready() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let stale_guard = session
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    let latest_guard = session
        .update_pose(sample_pose(11.0))
        .unwrap_or(sample_guard(1, 0, target));

    assert!(!session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: stale_guard,
            result: Ok(sample_result(8.5)),
        },
    ));
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);
    assert!(session.preview().is_none());

    assert!(session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: latest_guard,
            result: Ok(sample_result(11.5)),
        },
    ));
    assert_eq!(session.mode(), BridgeSplitMode::PlantedReady);
    assert!(session.can_apply());
}

#[test]
fn stale_generation_layer_topology_and_transform_results_are_discarded() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let active_guard = session
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));

    let layer_mismatch = BridgeSplitTarget {
        layer_id: sample_entry().id(),
        ..target
    };
    assert!(!session.apply_job_output(
        Some(layer_mismatch),
        BridgeSplitJobOutput {
            guard: active_guard,
            result: Ok(sample_result(8.0)),
        },
    ));
    assert!(!session.apply_job_output(
        Some(BridgeSplitTarget {
            topology_id: target.topology_id.saturating_add(1),
            ..target
        }),
        BridgeSplitJobOutput {
            guard: active_guard,
            result: Ok(sample_result(8.0)),
        },
    ));
    let translated =
        sample_entry().with_transform(Affine3A::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    assert!(!session.apply_job_output(
        Some(BridgeSplitTarget::capture(&translated)),
        BridgeSplitJobOutput {
            guard: active_guard,
            result: Ok(sample_result(8.0)),
        },
    ));
    assert!(!session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: BridgeSplitGuard {
                session_id: active_guard.session_id,
                generation: active_guard.generation.saturating_sub(1),
                target,
            },
            result: Ok(sample_result(8.0)),
        },
    ));
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);
    assert!(session.preview().is_none());
}

#[test]
fn cancel_drops_pending_and_product_visible_state() {
    let entry = sample_entry();
    let target = BridgeSplitTarget::capture(&entry);
    let mut controller = BridgeSplitController::default();
    controller.start(&entry);
    let guard = controller
        .session_mut()
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert!(controller.session_mut().apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard,
            result: Ok(sample_result(8.0)),
        },
    ));
    assert!(controller.session().can_apply());

    controller.cancel();
    let session = controller.session();

    assert_eq!(session.mode(), BridgeSplitMode::Off);
    assert!(session.target().is_none());
    assert!(session.pose().is_none());
    assert!(session.preview().is_none());
    assert!(session.failure().is_none());
    assert!(!session.can_apply());
}

#[test]
fn cancel_restart_same_target_rejects_prior_session_result() {
    let timeout = Duration::from_secs(5);
    let entry = sample_entry();
    let target = BridgeSplitTarget::capture(&entry);
    let (started_tx, started_rx) = mpsc::channel::<BridgeSplitGuard>();
    let (snapshot_tx, snapshot_rx) = mpsc::channel::<Arc<Mesh>>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut controller =
        BridgeSplitController::with_worker(BridgeSplitWorker::spawn_with_compute({
            let release_rx = Arc::clone(&release_rx);
            move |input| {
                let _ = started_tx.send(input.guard);
                let _ = snapshot_tx.send(Arc::clone(&input.mesh));
                if let Ok(receiver) = release_rx.lock() {
                    let _ = receiver.recv();
                }
                Ok(sample_result(input.request.max_disc_radius_mm))
            }
        }));

    controller.start(&entry);
    let prior_guard = controller
        .session_mut()
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert!(controller.submit_current_request(&entry));
    assert_eq!(started_rx.recv_timeout(timeout), Ok(prior_guard));
    let prior_snapshot = snapshot_rx.recv_timeout(timeout);
    let references_before_cancel = prior_snapshot.as_ref().ok().map(Arc::strong_count);
    assert!(matches!(references_before_cancel, Some(count) if count >= 3));

    controller.cancel();
    assert_eq!(
        prior_snapshot
            .as_ref()
            .ok()
            .map(|snapshot| Arc::strong_count(snapshot).saturating_add(1)),
        references_before_cancel
    );
    controller.start(&entry);
    let latest_guard = controller
        .session_mut()
        .plant(sample_pose(12.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(prior_guard.generation, latest_guard.generation);
    assert_eq!(
        latest_guard.session_id,
        next_nonzero_session_id(prior_guard.session_id)
    );
    assert!(controller.submit_current_request(&entry));

    assert!(release_tx.send(()).is_ok());
    assert_eq!(
        poll_controller_until_job_started(&mut controller, target, &started_rx),
        Some(latest_guard)
    );
    let latest_snapshot = snapshot_rx.recv_timeout(timeout);
    assert!(matches!(
        (&prior_snapshot, &latest_snapshot),
        (Ok(prior), Ok(latest)) if !Arc::ptr_eq(prior, latest)
    ));
    assert_eq!(controller.session().mode(), BridgeSplitMode::PlantedPending);
    assert!(controller.session().preview().is_none());
    assert!(!controller.session().can_apply());

    assert!(release_tx.send(()).is_ok());
    assert!(poll_controller_until(&mut controller, Some(target)));
    assert_eq!(controller.session().mode(), BridgeSplitMode::PlantedReady);
    assert!(controller.session().can_apply());
}

#[test]
fn direct_restart_same_target_rejects_prior_session_result() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let prior_guard = session
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert!(session.current_request().is_some());

    session.start(target);
    let latest_guard = session
        .plant(sample_pose(12.0))
        .unwrap_or(sample_guard(1, 0, target));
    assert_eq!(prior_guard.generation, latest_guard.generation);
    assert_eq!(
        latest_guard.session_id,
        next_nonzero_session_id(prior_guard.session_id)
    );

    assert!(!session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: prior_guard,
            result: Ok(sample_result(8.5)),
        },
    ));
    assert_eq!(session.mode(), BridgeSplitMode::PlantedPending);
    assert!(!session.can_apply());

    assert!(session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard: latest_guard,
            result: Ok(sample_result(12.5)),
        },
    ));
    assert!(session.can_apply());
}

#[test]
fn preview_report_never_overwrites_the_placed_disc_radius() {
    let target = sample_target();
    let mut session = BridgeSplitSession::default();
    session.start(target);
    let guard = session
        .plant(sample_pose(6.0))
        .unwrap_or(sample_guard(1, 0, target));

    assert!(session.apply_job_output(
        Some(target),
        BridgeSplitJobOutput {
            guard,
            result: Ok(sample_result(12.5)),
        },
    ));

    assert_eq!(session.mode(), BridgeSplitMode::PlantedReady);
    assert_eq!(session.generation(), 1);
    assert!(session.current_request().is_none());
    assert_eq!(session.preview().map(|preview| preview.guard), Some(guard));
    assert_eq!(
        session
            .preview()
            .map(|preview| preview.result.report.disc_radius_mm.to_bits()),
        Some(12.5_f32.to_bits())
    );
    assert_eq!(
        session.pose().map(|pose| pose.radius_mm.to_bits()),
        Some(6.0_f32.to_bits())
    );
}

#[test]
fn worker_failure_becomes_typed_tool_failure() {
    let entry = sample_entry();
    let live_target = BridgeSplitTarget::capture(&entry);
    let mut controller =
        BridgeSplitController::with_worker(BridgeSplitWorker::spawn_with_compute(|_input| {
            Err(BridgeSplitToolError::Kernel(
                BridgeSplitError::NoIntersection,
            ))
        }));
    controller.start(&entry);
    let planted = controller
        .session_mut()
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, live_target));
    assert_eq!(planted.generation, 1);
    assert!(controller.submit_current_request(&entry));

    assert!(poll_controller_until(&mut controller, Some(live_target)));
    assert_eq!(controller.session().mode(), BridgeSplitMode::Failed);
    assert!(matches!(
        controller.session().failure(),
        Some(BridgeSplitToolError::Kernel(
            BridgeSplitError::NoIntersection
        ))
    ));
}

#[test]
fn worker_coalesces_newest_request_with_one_active_and_one_queued_slot() {
    let (started_tx, started_rx) = mpsc::channel::<(u64, u64)>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut worker = BridgeSplitWorker::spawn_with_compute({
        let release_rx = Arc::clone(&release_rx);
        move |input| {
            let _ = started_tx.send((input.guard.session_id, input.guard.generation));
            if let Ok(receiver) = release_rx.lock() {
                let _ = receiver.recv();
            }
            Ok(sample_result(input.request.max_disc_radius_mm))
        }
    });

    assert!(worker.submit(sample_input(7, 1, 8.0)).is_ok());
    assert!(worker.submit(sample_input(7, 2, 9.0)).is_ok());
    assert!(worker.submit(sample_input(7, 3, 10.0)).is_ok());
    assert_eq!(
        worker
            .active_guard()
            .map(|guard| (guard.session_id, guard.generation)),
        Some((7, 1))
    );
    assert_eq!(
        worker
            .queued_guard()
            .map(|guard| (guard.session_id, guard.generation)),
        Some((7, 3))
    );
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)), Ok((7, 1)));

    assert!(release_tx.send(()).is_ok());
    let first_result = poll_worker_until(&mut worker);
    assert_eq!(first_result.len(), 1);
    assert_eq!(
        (
            first_result[0].guard.session_id,
            first_result[0].guard.generation
        ),
        (7, 1)
    );
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)), Ok((7, 3)));

    assert!(release_tx.send(()).is_ok());
    let second_result = poll_worker_until(&mut worker);
    assert_eq!(second_result.len(), 1);
    assert_eq!(
        (
            second_result[0].guard.session_id,
            second_result[0].guard.generation
        ),
        (7, 3)
    );
    assert!(worker.active_guard().is_none());
    assert!(worker.queued_guard().is_none());
}

#[test]
fn dropping_worker_does_not_wait_for_active_compute() {
    let timeout = Duration::from_secs(5);
    let (compute_started_tx, compute_started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (worker_exited_tx, worker_exited_rx) = mpsc::channel::<()>();
    let exit_signal = ThreadExitSignal(worker_exited_tx);
    let mut worker = BridgeSplitWorker::spawn_with_compute({
        let release_rx = Arc::clone(&release_rx);
        move |input| {
            let _keep_exit_signal_alive = &exit_signal;
            let _ = compute_started_tx.send(());
            if let Ok(receiver) = release_rx.lock() {
                let _ = receiver.recv();
            }
            Ok(sample_result(input.request.max_disc_radius_mm))
        }
    });

    assert!(worker.submit(sample_input(9, 1, 8.0)).is_ok());
    assert_eq!(compute_started_rx.recv_timeout(timeout), Ok(()));

    let (drop_started_tx, drop_started_rx) = mpsc::channel::<()>();
    let (drop_done_tx, drop_done_rx) = mpsc::channel::<()>();
    let dropper = std::thread::spawn(move || {
        let _ = drop_started_tx.send(());
        drop(worker);
        let _ = drop_done_tx.send(());
    });
    assert_eq!(drop_started_rx.recv_timeout(timeout), Ok(()));

    let drop_before_release = drop_done_rx.recv_timeout(timeout);
    assert!(release_tx.send(()).is_ok());
    if drop_before_release.is_err() {
        assert_eq!(drop_done_rx.recv_timeout(timeout), Ok(()));
    }
    assert_eq!(worker_exited_rx.recv_timeout(timeout), Ok(()));
    assert!(dropper.join().is_ok());
    assert_eq!(drop_before_release, Ok(()));
}

#[test]
fn public_shapes_are_constructible_for_app_wiring() {
    let _ = (
        BridgeSplitController::default(),
        BridgeSplitMode::Off,
        BridgeSplitToolError::Kernel(BridgeSplitError::NoIntersection),
        BridgeSplitJobOutput {
            guard: sample_guard(1, 1, sample_target()),
            result: Ok(sample_result(7.0)),
        },
    );
}

fn sample_scene() -> Scene {
    let mut scene = Scene::new();
    scene.add(sample_entry());
    scene
}

fn sample_entry() -> SceneMesh {
    SceneMesh::new(sample_mesh())
}

fn sample_target() -> BridgeSplitTarget {
    let scene = sample_scene();
    BridgeSplitTarget::capture(&scene.meshes()[0])
}

fn sample_pose(radius_mm: f32) -> BridgeSplitPose {
    BridgeSplitPose {
        center: Vec3::new(1.0, 2.0, 3.0),
        normal: Vec3::Y,
        radius_mm,
    }
}

fn planted_controller(entry: &SceneMesh) -> BridgeSplitController {
    let mut controller =
        BridgeSplitController::with_worker(BridgeSplitWorker::spawn_with_compute(|input| {
            Ok(sample_result(input.request.max_disc_radius_mm))
        }));
    let target = BridgeSplitTarget::capture(entry);
    controller.start(entry);
    let _ = controller
        .session_mut()
        .plant(sample_pose(8.0))
        .unwrap_or(sample_guard(1, 0, target));
    controller
}

fn submit_scene_entry(controller: &mut BridgeSplitController, entry: &SceneMesh) -> bool {
    controller.submit_current_request(entry)
}

fn sample_guard(session_id: u64, generation: u64, target: BridgeSplitTarget) -> BridgeSplitGuard {
    BridgeSplitGuard {
        session_id,
        generation,
        target,
    }
}

fn sample_input(session_id: u64, generation: u64, radius_mm: f32) -> BridgeSplitJobInput {
    BridgeSplitJobInput {
        mesh: Arc::new(sample_mesh()),
        transform: Affine3A::IDENTITY,
        request: BridgeSplitRequest {
            center: Vec3::ZERO,
            normal: Vec3::X,
            kerf_mm: DEFAULT_BRIDGE_SPLIT_KERF_MM,
            disc_radius_mm: radius_mm,
            max_disc_radius_mm: radius_mm,
        },
        guard: sample_guard(session_id, generation, sample_target()),
    }
}

fn sample_mesh() -> Mesh {
    Mesh::empty()
}

fn sample_result(disc_radius_mm: f32) -> CoreBridgeSplitResult {
    CoreBridgeSplitResult {
        part_a: sample_mesh(),
        part_b: sample_mesh(),
        report: BridgeSplitReport {
            disc_radius_mm,
            ..BridgeSplitReport::default()
        },
    }
}

fn poll_controller_until(
    controller: &mut BridgeSplitController,
    live_target: Option<BridgeSplitTarget>,
) -> bool {
    for _ in 0..100 {
        if controller.poll(live_target) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn poll_controller_until_job_started(
    controller: &mut BridgeSplitController,
    live_target: BridgeSplitTarget,
    started_rx: &mpsc::Receiver<BridgeSplitGuard>,
) -> Option<BridgeSplitGuard> {
    for _ in 0..100 {
        let _ = controller.poll(Some(live_target));
        match started_rx.try_recv() {
            Ok(guard) => return Some(guard),
            Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    None
}

fn poll_worker_until(worker: &mut BridgeSplitWorker) -> Vec<BridgeSplitJobOutput> {
    for _ in 0..100 {
        let outputs = worker.poll();
        if !outputs.is_empty() {
            return outputs;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Vec::new()
}

struct ThreadExitSignal(mpsc::Sender<()>);

impl Drop for ThreadExitSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
