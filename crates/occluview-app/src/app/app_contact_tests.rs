#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::app::app_test_support::test_app;
use crate::contact::ContactRequest;
use crate::contact_worker::{ContactCompletion, ContactFailure, ContactOutcome, ContactWorker};
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};
use std::sync::Arc;

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

#[test]
fn a_reading_with_no_executor_reports_a_failure_instead_of_measuring_forever() {
    let mut app = test_app("contact-no-executor");
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));
    app.tools
        .contacts
        .install_worker_for_tests(ContactWorker::spawn_failing());

    assert!(open_contacts_on(&mut app, first));

    assert!(
        !app.tools.contacts.is_busy(),
        "nothing is running, so the panel must not say a reading is"
    );
    assert_eq!(
        app.tools.contacts.status(),
        Some(crate::contact::ContactStatus::Failed(
            ContactFailure::Worker
        )),
        "a reading with no executor has failed, not started"
    );
    assert!(
        app.tools.contacts.pending_request().is_none(),
        "and there is no request to wait for"
    );
    assert!(
        app.tools.contacts.refused(),
        "the operator is offered the retry a refusal earns"
    );

    let scene = app.document.scene.clone().expect("scene");
    let pair = app.tools.contacts.pair().expect("an open reading");
    let keys = crate::contact::contact_job_keys(&scene, pair, false).expect("a pair");
    assert!(
        !app.tools.contacts.needs_measurement(keys),
        "a worker that cannot run must not be retried every frame"
    );
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
