//! What a bridge-split job is given, and when it is replaced.
//!
//! Split out of `tests.rs` for the file-size budget. These are the tests about
//! the snapshot the worker receives: shared between submits, refreshed by a
//! restart, and always the layer as it stands when the job is queued.

use super::tests::{
    poll_controller_until, poll_controller_until_job_started, sample_entry, sample_guard,
    sample_pose, sample_result, sample_target, submit_scene_entry,
};
use super::{
    next_nonzero_session_id, BridgeSplitController, BridgeSplitGuard, BridgeSplitJobOutput,
    BridgeSplitMode, BridgeSplitSession, BridgeSplitTarget, BridgeSplitWorker,
    MAX_BRIDGE_SPLIT_KERF_MM,
};
use glam::{Affine3A, Vec3};
use occluview_core::Mesh;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

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
    // Shared geometry: a restart hands over the same `Arc`, so what is checked
    // is that the worker gets the layer as it is now.
    let restarted_snapshot = started_rx.recv_timeout(timeout);
    assert!(matches!(
        &restarted_snapshot,
        Ok(restarted) if Arc::ptr_eq(restarted, &entry.mesh)
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
    // As above; the session id checked earlier is what says this is a new job.
    let latest_snapshot = snapshot_rx.recv_timeout(timeout);
    assert!(matches!(
        &latest_snapshot,
        Ok(latest) if Arc::ptr_eq(latest, &entry.mesh)
    ));
    assert_eq!(controller.session().mode(), BridgeSplitMode::PlantedPending);
    assert!(controller.session().preview().is_none());
    assert!(!controller.session().can_apply());

    assert!(release_tx.send(()).is_ok());
    assert!(poll_controller_until(&mut controller, Some(target)));
    assert_eq!(controller.session().mode(), BridgeSplitMode::PlantedReady);
    assert!(controller.session().can_apply());
}
