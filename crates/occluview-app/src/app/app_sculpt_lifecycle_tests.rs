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

#[test]
fn a_replace_does_not_discard_a_layer_the_operator_is_sculpting() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-vs-replace");
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > committed_vertices,
        "the stroke has visibly densified the layer"
    );

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

#[test]
fn an_empty_stroke_does_not_leave_the_guards_latched() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-empty-stroke");
    assert!(
        app.sculpt_has_live_work(),
        "opening a stroke is already work in progress"
    );

    let ctx = app.ui.repaint_ctx.clone();
    assert!(app.commit_sculpt_stroke(&ctx));
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

#[test]
fn a_stroke_that_ends_without_a_worker_does_not_latch_the_guards() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-marker-no-worker");
    app.tools.sculpt.disarm();
    app.tools.sculpt.worker = None;

    app.settle_sculpt_work_marker();

    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "a stroke that ended is not work the guards have to ask about"
    );
    assert!(!app.sculpt_has_live_work());
}

/// An export started while a Sculpt stroke is still being rebuilt must not
/// write anything. The scene only advances to a stroke's result when its
/// worker lands, so the old code wrote the pre-stroke geometry, reported
/// success, and the operator found out by re-opening the file. All three
/// export entry points read the same scene and must tell the operator to
/// finish the stroke instead.
#[test]
fn every_export_path_refuses_while_a_stroke_is_in_flight() {
    // The layer export path takes the scene directly.
    let (mut app, layer_id) = app_with_a_live_stroke("export-during-stroke-layer");
    let scene = app.document.scene.clone().expect("scene");
    let paths = app.persistence.current_paths.clone();
    let request = LayerContextRequest {
        index: 0,
        layer_id,
        action: LayerContextAction::ExportLayer,
    };
    assert!(
        !app.save_layer_export_dialog(scene.as_ref(), &paths, request),
        "a layer export during a live stroke must be refused"
    );
    assert_eq!(
        app.ui.status_message.as_deref(),
        Some(app.ui.locale.text("edit-session-busy").as_str()),
        "and the operator must be told why"
    );

    // The two scene paths share the same guard.
    let (mut app, _) = app_with_a_live_stroke("export-during-stroke-scene");
    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        app.refuse_export_during_stroke(&ctx),
        "the scene paths must refuse the same way"
    );
    assert_eq!(
        app.ui.status_message.as_deref(),
        Some(app.ui.locale.text("edit-session-busy").as_str())
    );

    // With no stroke in flight the guard is silent, so it cannot block the
    // ordinary export path it is there to protect.
    let (mut app, _) = app_with_a_live_stroke("export-during-stroke-idle");
    app.tools.sculpt.stroke = None;
    app.document.unsaved_sculpt_stroke = false;
    let ctx = app.ui.repaint_ctx.clone();
    assert!(!app.refuse_export_during_stroke(&ctx));
}
