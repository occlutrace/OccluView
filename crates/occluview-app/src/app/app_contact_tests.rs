#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::app::app_test_support::test_app;
use crate::contact::ContactRequest;
use crate::contact_worker::{ContactCompletion, ContactFailure, ContactOutcome, ContactWorker};
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};
use std::sync::Arc;
use std::time::Duration;

fn slab(x: f32, z: f32) -> Mesh {
    Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::new(x, 0.0, z)),
            Vertex::at(Vec3::new(x + 1.0, 0.0, z)),
            Vertex::at(Vec3::new(x, 1.0, z)),
        ],
        vec![0, 1, 2],
    )
    .expect("a triangle is a mesh")
}

fn three_layer_scene() -> (Scene, SceneMeshId, SceneMeshId, SceneMeshId) {
    let mut scene = Scene::new();
    let first = scene.add(SceneMesh::new(slab(0.0, 0.0)));
    let second = scene.add(SceneMesh::new(slab(0.0, 0.1)));
    let third = scene.add(SceneMesh::new(slab(0.0, 5.0)));
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    (scene, ids[first], ids[second], ids[third])
}

fn open_contacts_on(app: &mut OccluViewApp, layer: SceneMeshId) -> bool {
    app.tools.align.settings.show_deviation = false;
    let scene = app.document.scene.clone().expect("scene");
    app.begin_contacts_from_layer(&scene, layer)
}

fn pending(app: &OccluViewApp) -> ContactRequest {
    app.tools
        .contacts
        .pending_request()
        .expect("a submitted reading waits for its answer")
}

fn deliver_answer(app: &OccluViewApp, request: ContactRequest, failure: ContactFailure) {
    let worker = app.tools.contacts.worker().expect("a worker");
    worker.publish_for_tests(ContactCompletion {
        generation: worker.generation(),
        request_id: request.id,
        keys: request.keys,
        outcome: ContactOutcome::Failed(failure),
    });
}

#[test]
fn an_answer_for_a_superseded_reading_is_not_applied_to_the_current_one() {
    let mut app = test_app("contact-superseded-reading");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let superseded = pending(&app);

    assert!(open_contacts_on(&mut app, third));
    let current = pending(&app);
    assert_ne!(superseded.id, current.id, "asking again is a new request");
    assert_ne!(superseded.keys, current.keys, "about different surfaces");

    deliver_answer(&app, superseded, ContactFailure::NoSurface);
    app.drain_contacts_worker(&ctx);

    assert_eq!(
        app.tools.contacts.status(),
        Some(crate::contact::ContactStatus::Measuring),
        "the new reading is still measuring; a refusal that belongs to the \
         previous one must not write its sentence"
    );
    assert!(
        !app.tools.contacts.refused(),
        "and must not offer the operator a retry for it"
    );
    assert_eq!(
        app.tools
            .contacts
            .pending_request()
            .map(|request| request.id),
        Some(current.id),
        "this reading is still waiting for the answer it asked for"
    );
}

#[test]
fn an_answer_measured_before_the_scan_moved_is_not_applied() {
    let mut app = test_app("contact-moved-under-hold");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let request = pending(&app);

    app.document.live_scene_mut().expect("scene").meshes_mut()[0].transform =
        glam::Affine3A::from_translation(Vec3::new(0.0, 0.0, 2.0));
    app.tools.contacts.hold_for_drag();
    assert!(
        app.tools.contacts.fields().is_empty(),
        "the hold clears them"
    );

    deliver_answer(&app, request, ContactFailure::NoSurface);
    app.drain_contacts_worker(&ctx);

    assert_eq!(
        app.tools.contacts.status(),
        Some(crate::contact::ContactStatus::Remeasuring),
        "the reading is still held, and an answer from before the move must not \
         replace what the panel is saying"
    );
    assert!(
        !app.tools.contacts.refused(),
        "nor leave a refusal behind for a pose the operator has already left"
    );
    assert!(app.tools.contacts.fields().is_empty());
}

/// A worker that could not start is not kept: the next reading gets a new
/// thread. Keeping it would turn one failed `spawn` into a session where
/// contacts never work again and the only cure is a restart.
#[test]
fn a_reading_replaces_a_worker_that_could_not_start() {
    let mut app = test_app("contact-no-executor");
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));
    app.tools
        .contacts
        .install_worker_for_tests(ContactWorker::spawn_failing());

    assert!(open_contacts_on(&mut app, first));

    assert!(
        app.tools
            .contacts
            .worker()
            .is_some_and(|worker| !worker.has_failed()),
        "the reading must reach a worker that can run"
    );
    assert!(
        app.tools.contacts.pending_request().is_some(),
        "and the measurement it submitted must be waiting, not abandoned"
    );
}

#[test]
fn a_worker_that_dies_with_a_job_in_flight_releases_the_reading() {
    let mut app = test_app("contact-worker-died");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));
    app.tools
        .contacts
        .install_worker_for_tests(ContactWorker::spawn_panicking());

    assert!(open_contacts_on(&mut app, first));
    let keys = pending(&app).keys;

    let mut waited = Duration::ZERO;
    while app.tools.contacts.is_busy() && waited < Duration::from_secs(10) {
        app.drain_contacts_worker(&ctx);
        std::thread::sleep(Duration::from_millis(5));
        waited += Duration::from_millis(5);
    }

    assert!(
        !app.tools.contacts.is_busy(),
        "the bar must stop spinning when its worker is gone"
    );
    assert_eq!(
        app.tools.contacts.status(),
        Some(crate::contact::ContactStatus::Failed(
            ContactFailure::Worker
        )),
        "and it must say the reading failed"
    );
    assert!(
        app.tools.contacts.refused(),
        "with the retry a refusal earns, instead of an endless spinner"
    );
    assert!(
        !app.tools.contacts.needs_measurement(keys),
        "a dead worker must not be retried every frame"
    );
}

/// "Read again" has to actually read again. The worker that died cannot run a
/// new job, so the retry must reach a fresh one; offering a button that only
/// re-reports the same failure is worse than offering nothing.
#[test]
fn read_again_after_a_worker_death_reaches_a_new_worker() {
    let mut app = test_app("contact-retry-after-death");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));
    app.tools
        .contacts
        .install_worker_for_tests(ContactWorker::spawn_panicking());

    assert!(open_contacts_on(&mut app, first));
    let mut waited = Duration::ZERO;
    while app.tools.contacts.is_busy() && waited < Duration::from_secs(10) {
        app.drain_contacts_worker(&ctx);
        std::thread::sleep(Duration::from_millis(5));
        waited += Duration::from_millis(5);
    }
    assert!(app.tools.contacts.refused(), "the death is reported");

    // The retry chip's action, as the bar wires it.
    app.tools.contacts.forget_failure();
    app.submit_contacts_job();

    assert!(
        app.tools.contacts.pending_request().is_some(),
        "the retry must reach a worker that can run it"
    );
    assert!(
        !app.tools.contacts.refused(),
        "and it must not report the old worker's failure again"
    );
}

/// The sentence that explains why nothing is being measured must go away when
/// the reason does. Hiding a scan says "show it again and the reading
/// resumes"; nothing cleared the override, so the bar kept saying it forever —
/// and when the override replaced a failure, the retry it hid never came back.
#[test]
fn showing_a_scan_again_clears_the_unusable_sentence() {
    let mut app = test_app("contact-unusable-clears");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let mut waited = Duration::ZERO;
    while app.tools.contacts.is_busy() && waited < Duration::from_secs(10) {
        app.drain_contacts_worker(&ctx);
        std::thread::sleep(Duration::from_millis(5));
        waited += Duration::from_millis(5);
    }

    // Hide the antagonist: the reading cannot be measured right now.
    app.document.live_scene_mut().expect("scene").meshes_mut()[1].visible = false;
    app.sync_contacts_with_scene(&ctx);
    assert_eq!(
        app.tools.contacts.status(),
        Some(crate::contact::ContactStatus::AntagonistUnusable),
        "hiding the other scan explains why nothing is measured"
    );

    // Show it again: the explanation is no longer true.
    app.document.live_scene_mut().expect("scene").meshes_mut()[1].visible = true;
    app.sync_contacts_with_scene(&ctx);
    assert!(
        !matches!(
            app.tools.contacts.status(),
            Some(crate::contact::ContactStatus::AntagonistUnusable)
        ),
        "the sentence must not outlive the reason for it"
    );
    assert!(
        !app.tools.contacts.has_unusable_override(),
        "and the override must be gone, not merely overwritten"
    );
    let _ = second;
}

#[test]
fn a_dropped_answer_releases_the_request_so_the_scene_can_be_measured_again() {
    let mut app = test_app("contact-dropped-answer-resubmits");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let request = pending(&app);
    let keys = request.keys;

    app.document.live_scene_mut().expect("scene").meshes_mut()[0].transform =
        glam::Affine3A::from_translation(Vec3::new(0.0, 0.0, 2.0));
    app.tools.contacts.hold_for_drag();
    deliver_answer(&app, request, ContactFailure::NoSurface);
    app.drain_contacts_worker(&ctx);

    assert!(
        app.tools.contacts.pending_request().is_none(),
        "a dropped answer must release the request it belonged to"
    );

    app.document.live_scene_mut().expect("scene").meshes_mut()[0].transform =
        glam::Affine3A::IDENTITY;
    app.tools.contacts.resume_after_drag();
    assert!(
        app.tools.contacts.needs_measurement(keys),
        "with the drag over, the dropped answer must not be treated as still          pending: nothing is running and nothing would ever submit one"
    );
    app.sync_contacts_with_scene(&ctx);

    let resubmitted = pending(&app);
    assert_eq!(
        resubmitted.keys, keys,
        "the scene is back to the keys the answer described, so those are what \
         the reading asks for"
    );
    assert_ne!(
        resubmitted.id, request.id,
        "and it is a NEW measurement, not the request whose answer was dropped"
    );
}

/// The same rule for the Align worker: a dead one is replaced, not latched.
///
/// `AlignWorker::submit` refuses every job once its thread has failed, and
/// nothing used to replace it, so one panic inside the refinement left Align
/// dead for the rest of the session — the tool armed, the button responded, and
/// no job ever ran again.
#[test]
fn the_align_worker_is_replaced_after_it_dies() {
    let mut app = test_app("align-worker-respawn");
    assert!(!app.align_worker_mut().has_failed());

    app.align_worker_mut().poison_queue_for_tests();
    for _ in 0..200 {
        if app
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(crate::align_worker::AlignWorker::has_failed)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.tools
            .align
            .worker
            .as_ref()
            .is_some_and(crate::align_worker::AlignWorker::has_failed),
        "the worker must actually be marked failed, or this test proves nothing"
    );

    assert!(
        !app.align_worker_mut().has_failed(),
        "asking for the worker again must hand back a live one"
    );
}
