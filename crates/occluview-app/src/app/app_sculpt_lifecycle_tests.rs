//! Document-lifecycle tests against a live Sculpt stroke.
//!
//! Sculpt is the one edit that keeps geometry in the worker and in the document
//! at the same time: dabs stream into the worker's shadow and into the
//! prepared GPU buffers while the stroke is open, and a densification swaps the
//! layer's whole mesh into the document. The stroke only becomes an edit when it
//! is released and the worker's completion is committed — on release the edit
//! session is still open and no layer is marked unsaved yet.
//!
//! A Replace, a Close, or a Save decided from the unsaved-work set therefore has
//! to ask about the live gesture as well, or it decides against a scene it is
//! about to destroy while the operator is still holding the brush.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::unwrap_used
)]

use super::app_mesh_export::PendingLayerExports;
use super::*;
use crate::app::app_test_support::{delivered_load, test_app};
use crate::scene_loading::SceneLoadMode;
use crate::sculpt_tool::{SculptSession, StrokeState};
use crate::sculpt_worker::SculptWorker;
use glam::{Affine3A, Vec3};
use occluview_core::{
    mesh_edit_buffers_from_mesh, BrushMode, BrushSession, BrushStroke, Mesh, Scene, SceneMesh,
    SceneMeshId, Vertex,
};
use occluview_render::PreparedSceneTopology;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// The coarse lattice the densifying brush needs, so a stroke can change the
/// document while it is still open.
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

fn densifying_stroke() -> BrushStroke {
    BrushStroke {
        center: [0.0, 0.0, 4.0],
        radius_mm: 3.5,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    }
}

fn worker_for(mesh: &Mesh, layer_id: SceneMeshId) -> SculptWorker {
    mesh.warm_bvh();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(mesh)).expect("prepare");
    SculptWorker::spawn(SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        stroke_start_mesh: None,
    })
}

/// An app with one layer, an open edit session, and a live stroke on it.
///
/// "Live" is the state a held mouse button leaves behind: `stroke` is set, the
/// worker exists, and no release has happened, so nothing has been committed and
/// no layer is marked unsaved.
fn app_with_a_live_stroke(name: &str) -> (OccluViewApp, SceneMeshId) {
    let mut app = test_app(name);
    let mesh = coarse_ridge_mesh();
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    app.document.scene = Some(Arc::new(scene));
    let scene = app.document.scene.clone().expect("scene");
    let entry = &scene.meshes()[index];
    let layer_id = entry.id();
    assert!(app
        .document
        .edit_mode
        .begin_face_selection(entry, scene.as_ref()));
    let base = Arc::clone(&entry.mesh);
    drop(scene);

    app.tools.sculpt.worker = Some(worker_for(&base, layer_id));
    app.tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_dab_local: None,
        hold_seconds: 0.0,
    });
    // What the viewport's press path sets when it opens the stroke.
    app.document.unsaved_sculpt_stroke = true;
    (app, layer_id)
}

fn layer_mesh(app: &OccluViewApp, layer_id: SceneMeshId) -> Arc<Mesh> {
    app.document
        .scene
        .as_ref()
        .expect("a scene")
        .meshes()
        .iter()
        .find(|entry| entry.id() == layer_id)
        .expect("the layer")
        .mesh
        .clone()
}

/// Lay one densifying dab into the open stroke and run the frame loop's poll
/// until the worker's output has been handled.
fn lay_densifying_dab(app: &mut OccluViewApp) {
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(
            worker.try_apply(densifying_stroke(), BrushMode::Smooth),
            "the dab must be queued"
        );
    }
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll_sculpt_worker(&ctx);
        let Some(worker) = app.tools.sculpt.worker.as_ref() else {
            return;
        };
        if worker.is_quiescent() {
            app.poll_sculpt_worker(&ctx);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the densifying dab never settled"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// THE DEFECT: a Replace delivered while a stroke is visibly changing the layer
/// is applied on the strength of an authorization that never saw the stroke.
///
/// On release the stroke becomes an edit, but while it is held nothing is marked
/// unsaved and no history step exists. The Replace guard asks
/// `has_unsaved_mesh_edits` and `edit_mode.is_busy`, so it sees nothing at risk
/// and drops the scene the operator is actively sculpting, along with the
/// visible densified geometry.
#[test]
fn a_replace_does_not_discard_a_layer_the_operator_is_sculpting() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-vs-replace");
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > committed_vertices,
        "the stroke has visibly densified the layer"
    );

    // An Open of another case arrives, authorized before the stroke started.
    let pending = delivered_load(
        &app,
        {
            let mut other = Scene::new();
            other.add(SceneMesh::new(
                Mesh::new(
                    Some("scene-b".to_string()),
                    vec![
                        Vertex::at(Vec3::ZERO),
                        Vertex::at(Vec3::X),
                        Vertex::at(Vec3::Y),
                    ],
                    vec![0, 1, 2],
                )
                .expect("mesh"),
            ));
            other
        },
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let incoming = pending
        .receiver
        .recv()
        .expect("the delivered scene")
        .expect("a valid scene");
    app.apply_scene_load_result(pending, Ok(incoming));

    assert_eq!(
        app.document
            .scene
            .as_ref()
            .expect("a scene")
            .meshes()
            .iter()
            .map(|entry| entry.mesh.name().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec!["coarse-ridge".to_string()],
        "a Replace must not destroy the scene a live stroke is sculpting"
    );
    assert!(
        app.ui.pending_replace_open.is_some(),
        "the operator is asked about the work in flight instead"
    );
}

/// The guard's Save must not answer "nothing to save" about a layer whose
/// geometry is already changing on screen.
///
/// `pending_layer_exports` is the step that decides; it built its list from
/// `unsaved_edit_layer_ids` alone, which a live stroke never enters until it is
/// released. The flow then reported `NothingToSave`, and both the close guard
/// and the replace guard read that as "the scene is clean" and proceeded.
#[test]
fn a_save_does_not_call_a_live_stroke_nothing_to_save() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-vs-save");
    lay_densifying_dab(&mut app);
    let _ = layer_id;
    assert!(
        app.document.unsaved_edit_layer_ids.is_empty(),
        "the fixture really is the uncommitted case: nothing is marked unsaved"
    );

    let pending = app.pending_layer_exports();

    assert!(
        matches!(pending, PendingLayerExports::StrokeInFlight),
        "a Save must not report nothing to write while a stroke is changing \
         the layer the operator can see — and must not report the plain \
         `Nothing` a caller reads as \"the scene is clean\""
    );
    assert_eq!(
        app.ui.status_message,
        Some(app.ui.locale.tr("edit-session-busy")),
        "and the guard says why the save could not finish yet"
    );
}

/// A stroke that produced no geometry at all must not leave the app refusing to
/// open anything.
///
/// The marker is set when the stroke opens, before a single dab has landed. An
/// empty stroke — the operator pressed and released without touching the
/// surface, or the press never found one — publishes no completion, so nothing
/// arrives to withdraw the marker unless the poll does it from the worker's own
/// state. A marker left behind here would put the load guard, the close guard,
/// and the guard's Save in front of every action for the rest of the session.
#[test]
fn an_empty_stroke_does_not_leave_the_guards_latched() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-empty-stroke");
    assert!(
        app.sculpt_has_live_work(),
        "opening a stroke is already work in progress"
    );

    // The operator releases without a dab ever having landed.
    let ctx = app.ui.repaint_ctx.clone();
    assert!(app.commit_sculpt_stroke(&ctx));
    // The worker settles and the frame loop withdraws the marker. The two calls
    // are the frame's own order (`state.rs`: `poll_sculpt_worker` then
    // `settle_sculpt_work_marker`), and the settle half is deliberately outside
    // the poll so a session that ends without a worker still clears it.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll_sculpt_worker(&ctx);
        app.settle_sculpt_work_marker();
        if !app.sculpt_has_live_work() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "an empty stroke must not hold the guards forever"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "an empty stroke is not work the operator has to save"
    );
}

/// A stroke whose session ends without a worker must not leave the guards
/// latched.
///
/// A stroke that publishes nothing (the operator pressed and released without
/// touching the surface) has no completion to arrive, and toggling the brush off
/// drops the worker directly. Withdrawing the marker only from inside the worker
/// poll misses that: the poll returns early with no worker, so the marker stayed
/// set for the rest of the session and every Replace and Close then asked about
/// a stroke that no longer existed.
#[test]
fn a_stroke_that_ends_without_a_worker_does_not_latch_the_guards() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-marker-no-worker");
    // The stroke is opened, but it never lays a dab and its session goes away
    // without publishing anything.
    app.tools.sculpt.disarm();
    app.tools.sculpt.worker = None;

    app.settle_sculpt_work_marker();

    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "a stroke that ended is not work the guards have to ask about"
    );
    assert!(!app.sculpt_has_live_work());
}
