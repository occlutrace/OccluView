#![allow(clippy::expect_used, clippy::float_cmp, clippy::unwrap_used)]

use super::app_align_display::AlignOverlay;
use super::*;
use crate::app::app_test_support::test_app;
use crate::contact::{ContactLayerField, ContactRequest};
use crate::contact_worker::{ContactCompletion, ContactFailure, ContactOutcome, ContactWorker};
use glam::Vec3;
use occluview_contact::ContactStats;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};
use occluview_render::ContactFieldTexels;
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
/// resumes"; if the override is not cleared, the bar keeps saying it
/// indefinitely, and when the override replaced a failure, the retry it hid
/// never comes back.
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
        "with the drag over, the dropped answer must not be treated as still \
         pending: nothing is running and nothing would ever submit one"
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
        "and it is a new measurement, not the request whose answer was dropped"
    );
}

/// The same rule for the Align worker: a dead one is replaced, not latched.
///
/// `AlignWorker::submit` refuses every job once its thread has failed, so
/// without a replacement one panic inside the refinement would leave Align dead
/// for the rest of the session: the tool armed, the button responsive, and no
/// job ever running.
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

/// Opening a reading takes the align heatmap down with it.
///
/// The deviation map and the contact map measure the same two scans and are
/// painted through the same measured-map treatment, so a layer can only wear
/// one of them: with both up, the panel's legend describes a ramp the surface
/// is not wearing. The reading opens anyway — but the align map's colours, its
/// flag, and the toggle that claims a map is visible all have to go.
#[test]
fn opening_a_reading_clears_the_align_heatmap() {
    use crate::app::app_align_display::AlignOverlay;

    let mut app = test_app("contact-clears-align-heatmap");
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    // The state a landed Best fit leaves behind: a map up on the moving scan.
    app.tools.align.settings.show_deviation = true;
    assert!(
        app.attach_overlay_colors(first, vec![[12, 34, 56, 255]; 3], AlignOverlay::Map),
        "the map attaches to a layer whose vertex count it matches"
    );
    assert!(
        app.align_overlay_is_up(),
        "the map is up before the reading opens, or this proves nothing"
    );

    // The scene value the layer menu hands the action (production passes the
    // same `draft` copy). A second `Arc<Scene>` handle would trip
    // `live_scene_mut`'s sole-owner assertion instead of testing this contract.
    let draft = app.document.scene.as_ref().expect("scene").as_ref().clone();
    assert!(
        app.begin_contacts_from_layer(&draft, first),
        "the reading opens on the nearest eligible antagonist"
    );

    assert!(
        !app.tools.align.settings.show_deviation,
        "the heatmap toggle must not keep claiming a map is visible"
    );
    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "the overlay must say it is gone, not still be a map"
    );
    assert!(
        !app.align_overlay_is_up(),
        "the map's colour arrays must be dropped, not only the flag"
    );
    assert!(
        app.document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .iter()
            .all(|entry| entry.overlay_colors().is_none()),
        "no layer may still carry the align map's colours under the reading"
    );
}

/// Opening a contact reading from the layer menu while a Best-fit heatmap is up
/// must not edit the live scene while another handle to it is alive.
///
/// This is the path the operator actually takes: the clone lives in
/// `apply_layer_overlay_changes` and the edit happens two calls below it. With
/// a second handle alive, a debug build fires the assertion in `live_scene_mut`
/// on a normal gesture, and a release build silently copies the scene so the
/// caller's handle goes stale for the rest of the action.
///
/// The test drives the real entry point with the menu's own `Arc` — the same
/// shape `show_layers_overlay` and the viewport right-click menu pass — so an
/// extra clone under this path fails here.
#[test]
fn opening_contacts_from_the_menu_does_not_edit_the_scene_under_a_second_handle() {
    let (scene, first, _second, _third) = three_layer_scene();
    let mut app = test_app("contacts-menu-heatmap-up");
    // Install the scene as the sole handle so the setup's own in-place edits
    // (attaching the map colours) are legal, then take the menu's clone.
    let vertex_count = scene.meshes()[0].mesh.vertices().len();
    app.document.scene = Some(std::sync::Arc::new(scene));
    app.tools.align.settings.show_deviation = true;
    app.tools.align.overlay = AlignOverlay::Map;
    assert!(
        app.attach_overlay_colors(first, vec![[1, 2, 3, 4]; vertex_count], AlignOverlay::Map),
        "fixture: the heatmap has colours to clear"
    );
    // The menu takes the document's handle the way `show_layers_overlay` and the
    // viewport right-click menu do: one clone, which is then moved into the
    // dispatcher. Modeling an extra clone here would be stricter than the real
    // path and would fail for a reason the product does not have.
    let scene = app.document.scene.as_ref().expect("scene").clone();
    let scene_ptr = std::sync::Arc::as_ptr(&scene);

    // The menu's request, through the real dispatcher, with the menu's own
    // scene handle still alive as `show_layers_overlay` holds it.
    let index = 0usize;
    let layer_id = first;
    let request = LayerContextRequest {
        index,
        layer_id,
        action: LayerContextAction::Contacts,
    };
    let ctx = egui::Context::default();
    app.apply_layer_overlay_changes(
        scene,
        &[],
        LayerOverlayChanges {
            context_request: Some(request),
            layer_edits: Vec::new(),
        },
        &ctx,
    );

    assert!(
        app.tools.contacts.is_open(),
        "the reading the operator asked for opens"
    );
    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "and the heatmap gives way to it, which is the in-place edit under test"
    );
    // The second handle must still be looking at the same scene, i.e. nothing
    // copied it out from under the caller.
    assert_eq!(
        std::sync::Arc::as_ptr(app.document.scene.as_ref().expect("scene")),
        scene_ptr,
        "the document must still hold the very scene the caller passed, not a copy"
    );
}

/// The shader reads a vertex's field texel as `(index % width, index / width)`,
/// with `width` taken from the uniform's `contact_field_width`. The uniform is
/// built from the packed field's own row length, so a field narrower than the
/// 1024-texel ceiling must reach the GPU with its real width. A hardcoded
/// ceiling here makes every vertex past the first row decode the wrong texel and
/// paint a plausible but wrong map — which is worse than painting none.
#[test]
fn the_shader_is_told_the_width_the_field_was_packed_with() {
    let mut app = test_app("contact-field-width-wiring");
    let (scene, first, _second, _third) = three_layer_scene();
    app.document.scene = Some(Arc::new(scene));

    assert!(open_contacts_on(&mut app, first));
    let request = pending(&app);

    // A 7-texel-wide packed field, chosen to differ from the 1024 ceiling.
    let texels =
        Arc::new(ContactFieldTexels::new(vec![0u8; 7 * 4], 7, 1).expect("a 7x1 packed field"));
    let subject_field = ContactLayerField {
        layer: request.pair.subject,
        signed_mm: Arc::new(vec![0.0; 3]),
        texels: Arc::clone(&texels),
        revision: 1,
    };
    let antagonist_field = ContactLayerField {
        layer: request.pair.antagonist,
        signed_mm: Arc::new(vec![0.0; 3]),
        texels: Arc::new(ContactFieldTexels::new(vec![0u8; 4], 1, 1).expect("a 1x1 field")),
        revision: 2,
    };
    assert!(
        app.tools.contacts.store_measured(
            request,
            subject_field,
            antagonist_field,
            ContactStats::default(),
        ),
        "the reading must be accepted before it can be painted"
    );

    let scene = app.document.scene.clone().expect("scene");
    let updates = app.prepared_scene_updates(&scene);
    // `prepared_scene_updates` walks the scene in layer order, and the subject
    // is the first layer of `three_layer_scene`.
    let subject_update = updates.first().expect("the scene has layers");
    assert_eq!(
        subject_update.uniform.contact_field_width, 7.0,
        "the shader must be told the row length the field was packed with, \
         not the 1024 ceiling"
    );
}
