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
use crate::sculpt_tool::{SculptSession, SculptToolKind, StrokeState};
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

/// Length of the worker's live display shadow, read without blocking the
/// kernel. A densifying dab replaces this array, so a growth means the dab's
/// whole-layer rebuild has been computed.
fn sculpt_shadow_len(app: &OccluViewApp) -> usize {
    app.tools
        .sculpt
        .worker
        .as_ref()
        .and_then(|worker| {
            worker
                .shadow()
                .try_read()
                .ok()
                .map(|vertices| vertices.len())
        })
        .unwrap_or(0)
}

fn wait_for_shadow_growth(app: &OccluViewApp, above: usize) -> usize {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let len = sculpt_shadow_len(app);
        if len > above {
            return len;
        }
        assert!(
            Instant::now() < deadline,
            "the densifying dab never published (shadow stayed at {above})"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Poll the worker until the app owes a live stroke nothing more.
fn pump_sculpt_worker_until_idle(app: &mut OccluViewApp) {
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll_sculpt_worker(&ctx);
        app.settle_sculpt_work_marker();
        if !app.sculpt_has_live_work() {
            return;
        }
        assert!(Instant::now() < deadline, "the sculpt work never settled");
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

/// A brush-mode switch during a drag must FINISH the stroke. Aborting instead
/// throws away every dab the operator has already laid and reverts the layer
/// to the pre-stroke mesh.
#[test]
fn switching_brush_mode_finishes_a_live_stroke_instead_of_aborting_it() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-mode-switch");
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    let densified_vertices = layer_mesh(&app, layer_id).vertices().len();
    assert!(
        densified_vertices > committed_vertices,
        "the fixture must densify: {committed_vertices} -> {densified_vertices}"
    );
    assert!(
        app.tools.sculpt.stroke.is_some(),
        "the drag is still open when the brush is switched"
    );

    let ctx = app.ui.repaint_ctx.clone();
    app.toggle_sculpt_tool(SculptToolKind::Smooth, &ctx);

    assert_eq!(
        app.tools.sculpt.armed,
        Some(SculptToolKind::Smooth),
        "the new brush is armed"
    );
    pump_sculpt_worker_until_idle(&mut app);

    assert_eq!(
        layer_mesh(&app, layer_id).vertices().len(),
        densified_vertices,
        "switching brushes must finish the drag, not revert its geometry"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "the finished stroke is work the operator still has to save"
    );
    assert_eq!(
        app.document.edit_mode.undo_len(),
        1,
        "and it lands as one undoable edit"
    );
}

/// Turning the armed brush off ends the drag too, but the off-toggle must not
/// drop a worker that still owes the released stroke's completion: dropping it
/// discards the operator's geometry and leaves the busy guards latched.
#[test]
fn toggling_off_does_not_drop_a_worker_with_a_queued_finish() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-toggle-off");
    app.tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    let densified_vertices = layer_mesh(&app, layer_id).vertices().len();
    assert!(
        densified_vertices > committed_vertices,
        "the fixture must densify"
    );

    let ctx = app.ui.repaint_ctx.clone();
    app.toggle_sculpt_tool(SculptToolKind::AddRemove, &ctx);

    assert_eq!(app.tools.sculpt.armed, None, "the brush is off");
    assert!(
        app.tools.sculpt.worker.is_some(),
        "the worker still owes the finished stroke's completion, so the \
         off-toggle must keep it instead of dropping the work"
    );

    pump_sculpt_worker_until_idle(&mut app);

    assert_eq!(
        layer_mesh(&app, layer_id).vertices().len(),
        densified_vertices,
        "the stroke in flight when the brush went off must still land"
    );
    assert!(app.document.has_unsaved_mesh_edits());
    assert_eq!(app.document.edit_mode.undo_len(), 1);
    assert!(
        !app.sculpt_has_live_work(),
        "and the guards are not left latched on a stroke that is over"
    );
}

/// Ctrl+Z during a released-but-unfinished stroke: the drag is gone but the
/// worker still holds the finish. Abort has to revert that work too, or the
/// "undone" stroke lands a frame later.
#[test]
fn abort_also_reverts_a_released_stroke_waiting_in_the_worker() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-abort-released");
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    let committed_triangles = committed.triangle_count();
    let committed_topology = committed.topology_id();

    lay_densifying_dab(&mut app);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > committed_vertices,
        "the released stroke has a densified preview in the document"
    );

    let ctx = app.ui.repaint_ctx.clone();
    assert!(app.commit_sculpt_stroke(&ctx), "the drag is released");
    assert!(
        app.tools.sculpt.stroke.is_none(),
        "no drag is held any more, so only the worker remembers it"
    );
    assert!(
        app.tools.sculpt.worker_has_pending_work(),
        "but the worker still owes the released stroke's completion"
    );

    app.abort_sculpt_stroke();

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "an abort must revert the work waiting in the worker, not just the drag"
    );
    assert_eq!(after.triangle_count(), committed_triangles);
    assert_eq!(after.topology_id(), committed_topology);
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "the reverted stroke is not work the operator has to save"
    );
    assert_eq!(app.document.edit_mode.undo_len(), 0);
    assert!(app.tools.sculpt.worker.is_none(), "the session is dropped");
}

/// The worker queue is bounded, so a finish can be refused while older dabs
/// drain. A refused finish must keep the drag and retry it, not silently drop
/// the released stroke.
#[test]
fn a_rejected_finish_keeps_the_stroke_for_a_later_retry() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-finish-retry");
    // Park the kernel thread behind the completion backlog, then fill its
    // bounded command queue so the next finish is refused.
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        for _ in 0..3 {
            assert!(
                worker.try_apply(densifying_stroke(), BrushMode::Smooth),
                "pressure dab queued"
            );
            assert!(worker.finish_stroke(), "pressure stroke released");
        }
        let mut refused = false;
        for _ in 0..1024 {
            if !worker.finish_stroke() {
                refused = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            refused,
            "the bounded command queue must eventually refuse a finish"
        );
    }

    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        !app.commit_sculpt_stroke(&ctx),
        "queue backpressure refuses the finish"
    );
    assert!(
        app.tools.sculpt.stroke.is_some(),
        "a refused finish must keep the drag for a later retry"
    );
    assert!(app.tools.sculpt.finish_retry, "and arm that retry");
    assert_eq!(
        app.ui.status_message.as_deref(),
        Some(app.ui.locale.text("sculpt-worker-unavailable").as_str())
    );

    // Once the pressure clears, the worker poll retries the finish.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll_sculpt_worker(&ctx);
        if app.tools.sculpt.stroke.is_none() && !app.tools.sculpt.finish_retry {
            break;
        }
        assert!(Instant::now() < deadline, "the kept stroke never retried");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        app.ui.app_error.is_none(),
        "the retry must not end in a terminal failure"
    );
}

/// A worker that is gone cannot finish the stroke. The finish must say so and
/// drop the stale preview, not leave the densified shadow on screen as if it
/// were the operator's committed work.
#[test]
fn worker_loss_invalidates_an_active_sculpt_stroke() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-worker-loss");
    let committed = layer_mesh(&app, layer_id);
    let committed_vertices = committed.vertices().len();
    let committed_topology = committed.topology_id();
    lay_densifying_dab(&mut app);
    assert!(
        layer_mesh(&app, layer_id).vertices().len() > committed_vertices,
        "the stroke has a densified preview in the document"
    );

    app.tools.sculpt.worker = None;
    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        !app.commit_sculpt_stroke(&ctx),
        "a stroke cannot finish without its worker"
    );

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "losing the worker must revert the live shadow, not freeze it on screen"
    );
    assert_eq!(after.topology_id(), committed_topology);
    assert!(!app.document.has_unsaved_mesh_edits());
    assert_eq!(app.document.edit_mode.undo_len(), 0);
    assert_eq!(
        app.ui.status_message.as_deref(),
        Some(app.ui.locale.text("sculpt-worker-unavailable").as_str()),
        "the operator is told why the stroke could not finish"
    );
    assert!(
        app.tools.sculpt.stroke.is_none(),
        "the dead drag is not left latched"
    );
}

/// A frame's completions must be committed through their topology chain
/// before any leftover rebuild is installed. Installing a later stroke's
/// rebuild first would advance the worker past the completion's topology, and
/// the completed stroke would be dropped and its session invalidated.
#[test]
fn completions_walk_the_topology_chain_before_leftover_rebuilds_install() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-topology-chain");
    let base_len = sculpt_shadow_len(&app);

    // Stroke 1 densifies and is released, so its rebuild AND its completion
    // are waiting for the frame.
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
        assert!(worker.finish_stroke());
    }
    let after_first = wait_for_shadow_growth(&app, base_len);

    // Stroke 2 densifies but is still open, so its rebuild is a leftover with
    // no completion behind it yet.
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
    }
    let after_second = wait_for_shadow_growth(&app, after_first);

    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll_sculpt_worker(&ctx);
        if app.document.edit_mode.undo_len() == 1
            && layer_mesh(&app, layer_id).vertices().len() == after_second
        {
            break;
        }
        assert!(Instant::now() < deadline, "stroke 1 was never committed");
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(app.ui.app_error.is_none(), "no terminal failure");
    assert!(
        app.tools.sculpt.worker.is_some(),
        "committing stroke 1 must not invalidate the session it belongs to"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "stroke 1 is work the operator has to save"
    );
    assert_eq!(
        layer_mesh(&app, layer_id).topology_id(),
        app.tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker")
            .topology_id,
        "the leftover rebuild installs after the completion, matching the worker"
    );
}

/// Each completion must have its matching rebuild installed before it is
/// committed. With two finished densifying strokes in one frame, committing
/// without walking the chain leaves the second rebuild to install over a mesh
/// the commit already replaced, which invalidates the session.
#[test]
fn a_same_topology_completion_installs_its_rebuild_before_commit() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-completion-chain");
    let mut shadow_len = sculpt_shadow_len(&app);

    // Two released densifying strokes: two rebuilds and two completions.
    for _ in 0..2 {
        {
            let worker = app.tools.sculpt.worker.as_ref().expect("worker");
            assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
            assert!(worker.finish_stroke());
        }
        shadow_len = wait_for_shadow_growth(&app, shadow_len);
    }
    // A third, still-open stroke. Its rebuild is published after the second
    // completion, so waiting for it proves both completions are waiting too.
    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
    }
    let final_len = wait_for_shadow_growth(&app, shadow_len);

    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll_sculpt_worker(&ctx);
        if app.document.edit_mode.undo_len() == 2
            && layer_mesh(&app, layer_id).vertices().len() == final_len
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "both completions never committed"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(app.ui.app_error.is_none(), "no terminal failure");
    assert!(
        app.tools.sculpt.worker.is_some(),
        "the session must survive both commits"
    );
    assert!(app.document.has_unsaved_mesh_edits());
}
