#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::unwrap_used
)]

use super::*;
use crate::app::app_test_support::test_app;
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

fn additive_stroke() -> BrushStroke {
    BrushStroke {
        center: [0.0, 0.0, 4.0],
        radius_mm: 3.0,
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

fn app_sculpting(name: &str, mesh: Mesh) -> (OccluViewApp, SceneMeshId) {
    let mut app = test_app(name);
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    app.document.scene = Some(Arc::new(scene));
    let scene = app.document.scene.clone().expect("scene");
    let entry = &scene.meshes()[index];
    let layer_id = entry.id();
    assert!(
        app.document
            .edit_mode
            .begin_face_selection(entry, scene.as_ref()),
        "an edit session over the layer"
    );
    drop(scene);
    (app, layer_id)
}

fn pump_until_quiescent(app: &mut OccluViewApp) {
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
            "the sculpt worker never settled on its stroke"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn lay_and_release_one_dab(
    app: &mut OccluViewApp,
    base: &Mesh,
    stroke: BrushStroke,
    mode: BrushMode,
) {
    start_stroke(app, base);
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(stroke, mode), "the dab must be queued");
        assert!(worker.finish_stroke(), "the dab must be released");
    }
    pump_until_quiescent(app);
}

fn start_stroke(app: &mut OccluViewApp, base: &Mesh) {
    let layer_id = layer_id_of(app);
    app.tools.sculpt.worker = Some(worker_for(base, layer_id));
    app.tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_dab_local: None,
        hold_seconds: 0.0,
    });
}

fn lay_dab_mid_stroke(app: &mut OccluViewApp, stroke: BrushStroke, mode: BrushMode) {
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(stroke, mode), "the dab must be queued");
    }
    pump_until_quiescent(app);
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

fn layer_id_of(app: &OccluViewApp) -> SceneMeshId {
    app.document
        .edit_mode
        .session_layer_id()
        .expect("the session names its layer")
}

#[test]
fn aborting_a_densified_stroke_leaves_no_partial_geometry_in_the_document() {
    let (mut app, layer_id) = app_sculpting("sculpt-abort-after-densify", coarse_ridge_mesh());
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    let committed_triangles = committed.triangle_count();
    let committed_topology = committed.topology_id();

    start_stroke(&mut app, &committed);
    lay_dab_mid_stroke(&mut app, densifying_stroke(), BrushMode::Smooth);
    assert!(
        app.tools.sculpt.stroke.is_some(),
        "the stroke is still open; nothing has been released"
    );
    let densified = layer_mesh(&app, layer_id);
    assert!(
        densified.vertices().len() > committed_vertices,
        "the fixture must densify: {} -> {} vertices",
        committed_vertices,
        densified.vertices().len()
    );
    assert_ne!(
        densified.topology_id(),
        committed_topology,
        "the densified mesh carries a fresh topology identity"
    );

    app.abort_sculpt_stroke();

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "an aborted stroke must not leave the densified vertex array in the document"
    );
    assert_eq!(
        after.triangle_count(),
        committed_triangles,
        "nor the densified triangle list"
    );
    assert_eq!(
        after.topology_id(),
        committed_topology,
        "nor a topology identity that no committed stroke produced"
    );
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "an aborted stroke is not work the operator has to save"
    );
    assert_eq!(
        app.document.edit_mode.undo_len(),
        0,
        "and it leaves no history step behind"
    );
}

#[test]
fn aborting_a_densified_second_stroke_keeps_the_first_stroke_result() {
    let (mut app, layer_id) = app_sculpting("sculpt-abort-keeps-first", coarse_ridge_mesh());

    let first_base = layer_mesh(&app, layer_id);
    lay_and_release_one_dab(&mut app, &first_base, additive_stroke(), BrushMode::Add);
    let first_mesh = layer_mesh(&app, layer_id);
    assert!(
        first_mesh
            .vertices()
            .iter()
            .zip(first_base.vertices())
            .any(|(after, before)| after.position != before.position),
        "the first stroke must actually have moved geometry into the document"
    );
    let first_topology = first_mesh.topology_id();
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a committed stroke is unsaved work"
    );
    assert_eq!(
        app.document.edit_mode.undo_len(),
        1,
        "and it is one history step"
    );

    let second_base = Arc::new(layer_mesh(&app, layer_id).as_ref().clone());
    start_stroke(&mut app, &second_base);
    lay_dab_mid_stroke(&mut app, densifying_stroke(), BrushMode::Smooth);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > second_base.vertices().len(),
        "the second stroke densified the layer"
    );
    assert!(
        app.tools.sculpt.stroke.is_some(),
        "and the second stroke is still open"
    );

    app.abort_sculpt_stroke();

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.topology_id(),
        first_topology,
        "the aborted stroke reverts to the first stroke's finished result, not \
         to the session baseline"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "the first stroke is still work the operator has not saved"
    );
    assert_eq!(
        app.document.edit_mode.undo_len(),
        1,
        "and its single history step survives"
    );
}

#[test]
fn a_committed_densifying_stroke_keeps_its_geometry() {
    let (mut app, layer_id) = app_sculpting("sculpt-commit-after-densify", coarse_ridge_mesh());
    let base = layer_mesh(&app, layer_id);

    lay_and_release_one_dab(&mut app, &base, densifying_stroke(), BrushMode::Smooth);
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    assert!(
        committed_vertices > base.vertices().len(),
        "the stroke densified the layer"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a committed stroke is unsaved work"
    );

    app.abort_sculpt_stroke();

    assert_eq!(
        layer_mesh(&app, layer_id).vertices().len(),
        committed_vertices,
        "aborting a stroke that was already committed must not revert its \
         geometry"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "the committed work is still unsaved"
    );
}

#[test]
fn undo_and_redo_a_committed_densifying_stroke() {
    let (mut app, layer_id) = app_sculpting("sculpt-undo-densify", coarse_ridge_mesh());
    let base = layer_mesh(&app, layer_id);
    let base_vertices = base.vertices().len();
    let base_triangles = base.triangle_count();
    let base_topology = base.topology_id();

    lay_and_release_one_dab(&mut app, &base, densifying_stroke(), BrushMode::Smooth);
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    let committed_triangles = committed.triangle_count();
    assert!(
        committed_vertices > base_vertices,
        "the stroke densified the layer"
    );
    assert!(app.document.has_unsaved_mesh_edits());

    let ctx = app.ui.repaint_ctx.clone();
    app.apply_history_navigation_now(false, &ctx);
    let undone = layer_mesh(&app, layer_id);
    assert_eq!(
        undone.vertices().len(),
        base_vertices,
        "one undo returns the whole densifying stroke, so the document is the \
         mesh the stroke started from"
    );
    assert_eq!(undone.triangle_count(), base_triangles);

    app.apply_history_navigation_now(true, &ctx);
    let redone = layer_mesh(&app, layer_id);
    assert_eq!(
        redone.vertices().len(),
        committed_vertices,
        "and redo puts the densified result back"
    );
    assert_eq!(redone.triangle_count(), committed_triangles);
    let _ = base_topology;
}

#[test]
fn a_worker_failure_after_densification_leaves_no_partial_geometry() {
    use crate::sculpt_worker::SculptFailure;

    let (mut app, layer_id) = app_sculpting("sculpt-failure-after-densify", coarse_ridge_mesh());
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    let committed_triangles = committed.triangle_count();
    let committed_topology = committed.topology_id();

    start_stroke(&mut app, &committed);
    lay_dab_mid_stroke(&mut app, densifying_stroke(), BrushMode::Smooth);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > committed_vertices,
        "the densified preview is in the document"
    );

    app.fail_sculpt_session(
        &SculptFailure::WorkerStatePoisoned,
        &app.ui.repaint_ctx.clone(),
    );

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "a failed stroke leaves the committed geometry, not its own preview"
    );
    assert_eq!(after.triangle_count(), committed_triangles);
    assert_eq!(after.topology_id(), committed_topology);
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "and the failed stroke is not unsaved work"
    );
}
