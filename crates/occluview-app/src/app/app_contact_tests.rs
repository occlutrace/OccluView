//! Delivery-boundary tests for the contact reading.
//!
//! `contact_tests.rs` unit-tests the state's rules. These tests drive the
//! application's own entry points — `begin_contacts_from_layer`,
//! `submit_contacts_job`, `drain_contacts_worker` — because what has to be
//! proved here is a *sequence*: a measurement finishes for one reading and its
//! answer is delivered after the operator has moved past it, either to another
//! pair or by moving a scan under a held reading. The numbers in that answer
//! describe surfaces that are not on screen, so a panel that takes them reports
//! a reading nobody asked for.
//!
//! No sleeps. The answer is placed in the worker's publication slot with
//! `ContactWorker::publish_for_tests`, which is the same slot a real compute
//! fills, so the delivery is observed exactly where a race would be resolved
//! rather than after a guessed delay.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::app::app_test_support::test_app;
use crate::contact::ContactRequest;
use crate::contact_worker::{ContactCompletion, ContactFailure, ContactOutcome, ContactWorker};
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};
use std::sync::Arc;

/// A smallest readable surface: one triangle, non-empty bounding box.
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

/// Three surfaces: two close enough to read against each other, and a third
/// far enough that it is never the nearest candidate.
fn three_layer_scene() -> (Scene, SceneMeshId, SceneMeshId, SceneMeshId) {
    let mut scene = Scene::new();
    let first = scene.add(SceneMesh::new(slab(0.0, 0.0)));
    let second = scene.add(SceneMesh::new(slab(0.0, 0.1)));
    let third = scene.add(SceneMesh::new(slab(0.0, 5.0)));
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    (scene, ids[first], ids[second], ids[third])
}

/// Open a reading on `layer`.
///
/// Opening a reading also clears the align heatmap, and that restore path owns a
/// separate contract with its own test. Leaving the heatmap armed here would
/// make this file fail on that contract instead of the one it is about, so it is
/// turned off first.
fn open_contacts_on(app: &mut OccluViewApp, layer: SceneMeshId) -> bool {
    app.tools.align.settings.show_deviation = false;
    let scene = app.document.scene.clone().expect("scene");
    app.begin_contacts_from_layer(&scene, layer)
}

/// The request the reading is currently waiting for.
fn pending(app: &OccluViewApp) -> ContactRequest {
    app.tools
        .contacts
        .pending_request()
        .expect("a submitted reading waits for its answer")
}

/// Deliver an answer for `request`, exactly where the worker thread places one.
///
/// The outcome is a refusal rather than a measurement because what is under test
/// is the delivery decision, not the packing of a field: a refusal is the form
/// that changes the panel most visibly — it clears the map and writes a sentence
/// — so a path that mishandles delivery fails here loudly.
fn deliver_answer(app: &OccluViewApp, request: ContactRequest, failure: ContactFailure) {
    let worker = app.tools.contacts.worker().expect("a worker");
    worker.publish_for_tests(ContactCompletion {
        generation: worker.generation(),
        request_id: request.id,
        keys: request.keys,
        outcome: ContactOutcome::Failed(failure),
    });
}

/// The defect this file is about: a reading is opened on one pair, the operator
/// changes their mind and opens a reading on another, and the first
/// measurement's answer is delivered afterwards.
///
/// It describes other layers, so it may not be recorded, painted, or reported as
/// this pair's reading.
#[test]
fn an_answer_for_a_superseded_reading_is_not_applied_to_the_current_one() {
    let mut app = test_app("contact-superseded-reading");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let superseded = pending(&app);

    // The operator asks about a different layer before the answer is collected.
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

/// The same rule under a hand drag, and the half the request identity cannot
/// catch.
///
/// A drag rewrites a pose every frame while the reading is held, and the reading
/// keeps the same request in flight across all of it. So an answer can carry the
/// identity of the request that is still pending and still describe a pose that
/// is no longer on screen. Identity answers "which submission", not "which
/// scene", and the live scene has to be asked separately.
#[test]
fn an_answer_measured_before_the_scan_moved_is_not_applied() {
    let mut app = test_app("contact-moved-under-hold");
    let ctx = app.ui.repaint_ctx.clone();
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let request = pending(&app);

    // The operator starts dragging the scan. This is the frame-by-frame state of
    // that gesture: the pose has moved, the reading is held, and the request the
    // worker is still computing was built from where the scan used to be.
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

/// A worker with no thread must not leave the reading waiting for an answer that
/// cannot come.
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

    // The frame loop's question: the same inputs must not be queued every frame
    // against a worker that cannot run them.
    let scene = app.document.scene.clone().expect("scene");
    let pair = app.tools.contacts.pair().expect("an open reading");
    let keys = crate::contact::contact_job_keys(&scene, pair, false).expect("a pair");
    assert!(
        !app.tools.contacts.needs_measurement(keys),
        "a worker that cannot run must not be retried every frame"
    );
}
