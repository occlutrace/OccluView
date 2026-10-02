#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::unwrap_used
)]

use super::super::*;
use crate::app::app_test_support::{delivered_load, test_app};
use crate::app::mesh_edit::export::PendingLayerExports;
use crate::scene_loading::SceneLoadMode;
use crate::sculpt::sculpt_kernel::{BrushMode, BrushRayStep, BrushSession, BrushStroke};
use crate::sculpt::sculpt_tool::{SculptSession, SculptTip, SculptToolKind, StrokeState};
use crate::sculpt::sculpt_worker::SculptWorker;
use glam::{Affine3A, Vec3};
use occluview_core::test_support::coarse_ridge_mesh;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};
use occluview_mesh_edit::mesh_edit_buffers_from_mesh;
use occluview_render::PreparedSceneTopology;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

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
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    })
}

fn wait_for_worker_idle(worker: &SculptWorker, deadline: Instant) -> bool {
    worker.wait_until_idle(deadline.saturating_duration_since(Instant::now()))
}

fn app_with_a_live_stroke(name: &str) -> (OccluViewApp, SceneMeshId) {
    let mut app = test_app(name);
    let mesh = coarse_ridge_mesh().expect("ridge mesh");
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let entry = &scene.meshes()[index];
    let layer_id = entry.id();
    assert!(app.workspace.scenes[0]
        .document
        .edit_mode
        .begin_face_selection(entry, scene.as_ref()));
    let base = Arc::clone(&entry.mesh);
    drop(scene);

    app.workspace.scenes[0].tools.sculpt.worker = Some(worker_for(&base, layer_id));
    app.workspace.scenes[0].tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_pointer: [0.0, 0.0],
        input_pointer: [0.0, 0.0],
        last_ray: None,
        hold_seconds: 0.0,
        path_break_pending: false,
        release_pending: false,
        retained_samples: VecDeque::new(),
    });
    app.workspace.scenes[0].document.unsaved_sculpt_stroke = true;
    (app, layer_id)
}

fn layer_mesh(app: &OccluViewApp, layer_id: SceneMeshId) -> Arc<Mesh> {
    app.workspace.scenes[0]
        .document
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
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            worker.try_apply(densifying_stroke(), BrushMode::Smooth),
            "the dab must be queued"
        );
    }
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        let Some(worker) = app.workspace.scenes[0].tools.sculpt.worker.as_ref() else {
            return;
        };
        if worker.is_quiescent() {
            app.active_context()
                .expect("live test scene")
                .poll_sculpt_worker(&ctx);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the densifying dab never settled"
        );
        assert!(
            wait_for_worker_idle(worker, deadline),
            "the densifying dab never settled"
        );
    }
}

/// Length of the worker's append-only live display shadow.
fn sculpt_shadow_len(app: &OccluViewApp) -> usize {
    app.workspace.scenes[0]
        .tools
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
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "the densifying dab never published (shadow stayed at {above})"
        );
    }
}

/// Poll the worker until the app owes a live stroke nothing more.
fn pump_sculpt_worker_until_idle(app: &mut OccluViewApp) {
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        app.active_context()
            .expect("live test scene")
            .settle_sculpt_work_marker();
        if !app
            .active_context()
            .expect("live test scene")
            .sculpt_has_live_work()
        {
            return;
        }
        assert!(Instant::now() < deadline, "the sculpt work never settled");
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "the sculpt work never settled"
        );
    }
}

fn sculpt_strength_wheel_frame(app: &mut OccluViewApp, ctx: &egui::Context, delta_y: f32) -> bool {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let center = viewport.center();
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: false,
        ..Default::default()
    };
    let input = egui::RawInput {
        screen_rect: Some(viewport),
        events: vec![
            egui::Event::PointerMoved(center),
            egui::Event::ModifiersChanged(ctrl),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta_y),
                phase: egui::TouchPhase::Move,
                modifiers: ctrl,
            },
        ],
        ..Default::default()
    };
    let mut used = false;
    ctx.run_ui(input, |ui| {
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        used = app
            .active_context()
            .expect("live test scene")
            .adjust_sculpt_brush_from_wheel(ui.ctx(), &response);
    })
    .drop_without_applying_deltas();
    used
}

/// Wait until a topology delta and a sparse vertex update share one frame.
fn wait_for_topology_delta_and_sparse_update(app: &OccluViewApp) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        if worker.has_pending_topology_delta() && worker.has_pending_sparse_update() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the fixture never queued a topology delta and a sparse update together"
        );
        assert!(
            worker.wait_until_idle(deadline.saturating_duration_since(Instant::now())),
            "the fixture never queued a topology delta and a sparse update together"
        );
    }
}

/// The frame applies the append and face patch before sparse writes, then the
/// final committed mesh receives a new topology identity at stroke completion.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the complete gesture and its state assertions in one regression scenario."
)]
fn topology_deltas_flush_before_sparse_writes_and_commit_at_finish() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-delta-before-sparse");
    let initial_topology = layer_mesh(&app, layer_id).topology_id();
    let initial_vertices = layer_mesh(&app, layer_id).vertices().len();

    // A real densifying dab publishes a local topology patch.
    {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            worker.try_apply(densifying_stroke(), BrushMode::Smooth),
            "the densifying dab must be queued"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while !app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .has_pending_topology_delta()
    {
        assert!(
            Instant::now() < deadline,
            "the dab never published its delta"
        );
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "the dab never published its delta"
        );
    }
    // The vertex IDs remain stable across appended topology, so both outputs
    // can safely share one frame.
    app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .queue_sparse_for_tests(vec![0, 1, 2]);
    wait_for_topology_delta_and_sparse_update(&app);

    // Poll until both local output types have reached the prepared scene.
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    while app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .has_pending_topology_delta()
        || app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker")
            .has_pending_sparse_update()
    {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        assert!(
            Instant::now() < deadline,
            "sculpt deltas were never drained"
        );
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "sculpt deltas were never drained"
        );
    }

    let worker = app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker");
    let live_vertices = worker.live_geometry().expect("live geometry").0.len();
    assert!(live_vertices > initial_vertices);
    assert_eq!(
        layer_mesh(&app, layer_id).topology_id(),
        initial_topology,
        "the document commits only when the stroke finishes"
    );
    assert!(app
        .active_context()
        .expect("live test scene")
        .commit_sculpt_stroke(&ctx));
    pump_sculpt_worker_until_idle(&mut app);
    let worker = app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker");
    let scene_topology = layer_mesh(&app, layer_id).topology_id();
    assert_ne!(
        scene_topology, initial_topology,
        "the finished stroke must commit its changed topology"
    );
    assert_eq!(
        scene_topology, worker.topology_id,
        "the document and worker share the committed topology identity"
    );
    assert!(
        !worker.has_pending_sparse_update(),
        "the frame must also have drained the sparse update"
    );
}

#[test]
fn a_replace_does_not_discard_a_layer_the_operator_is_sculpting() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-vs-replace");
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    assert!(
        sculpt_shadow_len(&app) > committed_vertices,
        "the live display geometry has densified"
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
    app.active_context()
        .expect("live test scene")
        .apply_scene_load_result(pending, Ok(incoming), &egui::Context::default());

    assert_eq!(
        app.workspace.scenes[0]
            .document
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
    assert!(sculpt_shadow_len(&app) > layer_mesh(&app, layer_id).vertices().len());
    assert!(
        app.workspace.scenes[0]
            .document
            .unsaved_edit_layer_ids
            .is_empty(),
        "the fixture really is the uncommitted case: nothing is marked unsaved"
    );

    let pending = app
        .active_context()
        .expect("live test scene")
        .pending_layer_exports();

    assert!(
        matches!(pending, PendingLayerExports::StrokeInFlight),
        "a Save must not report nothing to write while a stroke is changing \
         the layer the operator can see — and must not report the plain \
         `Nothing` a caller reads as \"the scene is clean\""
    );
    assert_eq!(
        app.workspace.scenes[0].presentation.status_message,
        Some(
            app.ui
                .locale
                .tr(crate::i18n::message_id!("edit-session-busy"))
        ),
        "and the guard says why the save could not finish yet"
    );
}

#[test]
fn an_empty_stroke_does_not_leave_the_guards_latched() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-empty-stroke");
    assert!(
        app.active_context()
            .expect("live test scene")
            .sculpt_has_live_work(),
        "opening a stroke is already work in progress"
    );

    let ctx = app.ui.repaint_ctx.clone();
    assert!(app
        .active_context()
        .expect("live test scene")
        .commit_sculpt_stroke(&ctx));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        app.active_context()
            .expect("live test scene")
            .settle_sculpt_work_marker();
        if !app
            .active_context()
            .expect("live test scene")
            .sculpt_has_live_work()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "an empty stroke must not hold the guards forever"
        );
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "an empty stroke must not hold the guards forever"
        );
    }

    assert!(
        !app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
        "an empty stroke is not work the operator has to save"
    );
}

#[test]
fn a_stroke_that_ends_without_a_worker_does_not_latch_the_guards() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-marker-no-worker");
    app.workspace.scenes[0].tools.sculpt.disarm();
    app.workspace.scenes[0].tools.sculpt.worker = None;

    app.active_context()
        .expect("live test scene")
        .settle_sculpt_work_marker();

    assert!(
        !app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
        "a stroke that ended is not work the guards have to ask about"
    );
    assert!(!app
        .active_context()
        .expect("live test scene")
        .sculpt_has_live_work());
}

/// An export started while a Sculpt stroke is still changing geometry must not
/// write anything. The scene only advances to a stroke's result when its
/// worker lands, so an export during the stroke would write the pre-stroke
/// geometry and report success. All three export entry points read the same
/// scene and must tell the operator to finish the stroke instead.
#[test]
fn every_export_path_refuses_while_a_stroke_is_in_flight() {
    // The layer export path takes the scene directly.
    let (mut app, layer_id) = app_with_a_live_stroke("export-during-stroke-layer");
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let paths = app.workspace.scenes[0].document.current_paths.clone();
    let request = LayerContextRequest {
        index: 0,
        layer_id,
        action: LayerContextAction::ExportLayer,
    };
    assert!(
        !app.active_context()
            .expect("live test scene")
            .save_layer_export_dialog(scene.as_ref(), &paths, request),
        "a layer export during a live stroke must be refused"
    );
    assert_eq!(
        app.workspace.scenes[0]
            .presentation
            .status_message
            .as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("edit-session-busy"))
                .as_str()
        ),
        "and the operator must be told why"
    );

    // The two scene paths share the same guard.
    let (mut app, _) = app_with_a_live_stroke("export-during-stroke-scene");
    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        app.active_context()
            .expect("live test scene")
            .refuse_export_during_stroke(&ctx),
        "the scene paths must refuse the same way"
    );
    assert_eq!(
        app.workspace.scenes[0]
            .presentation
            .status_message
            .as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("edit-session-busy"))
                .as_str()
        )
    );

    // With no stroke in flight the guard is silent, so it cannot block the
    // ordinary export path it is there to protect.
    let (mut app, _) = app_with_a_live_stroke("export-during-stroke-idle");
    app.workspace.scenes[0].tools.sculpt.stroke = None;
    app.workspace.scenes[0].document.unsaved_sculpt_stroke = false;
    let ctx = app.ui.repaint_ctx.clone();
    assert!(!app
        .active_context()
        .expect("live test scene")
        .refuse_export_during_stroke(&ctx));
}

/// A brush-mode switch during a drag commits the live display geometry as one
/// undoable mesh edit.
#[test]
fn switching_brush_mode_finishes_a_live_stroke_instead_of_aborting_it() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-mode-switch");
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    let densified_vertices = sculpt_shadow_len(&app);
    assert!(
        densified_vertices > committed_vertices,
        "the fixture must densify: {committed_vertices} -> {densified_vertices}"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_some(),
        "the drag is still open when the brush is switched"
    );

    let ctx = app.ui.repaint_ctx.clone();
    app.active_context()
        .expect("live test scene")
        .toggle_sculpt_tool(SculptToolKind::Smooth, &ctx);

    assert_eq!(
        app.workspace.scenes[0].tools.sculpt.armed,
        Some(SculptToolKind::Smooth),
        "the new brush is armed"
    );
    assert_eq!(
        app.persistence.settings.last_sculpt_tool,
        SculptToolKind::Smooth,
        "the selected brush is remembered for the next Sculpt-tab entry"
    );
    pump_sculpt_worker_until_idle(&mut app);

    assert_eq!(
        layer_mesh(&app, layer_id).vertices().len(),
        densified_vertices,
        "switching brushes must finish the drag, not revert its geometry"
    );
    assert!(
        app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
        "the finished stroke is work the operator still has to save"
    );
    assert_eq!(
        app.workspace.scenes[0].document.edit_mode.undo_len(),
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
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let committed_vertices = layer_mesh(&app, layer_id).vertices().len();
    lay_densifying_dab(&mut app);
    let densified_vertices = sculpt_shadow_len(&app);
    assert!(
        densified_vertices > committed_vertices,
        "the fixture must densify"
    );

    let ctx = app.ui.repaint_ctx.clone();
    app.active_context()
        .expect("live test scene")
        .toggle_sculpt_tool(SculptToolKind::AddRemove, &ctx);

    assert_eq!(
        app.workspace.scenes[0].tools.sculpt.armed, None,
        "the brush is off"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.worker.is_some(),
        "the worker still owes the finished stroke's completion, so the \
         off-toggle must keep it instead of dropping the work"
    );

    pump_sculpt_worker_until_idle(&mut app);

    assert_eq!(
        layer_mesh(&app, layer_id).vertices().len(),
        densified_vertices,
        "the stroke in flight when the brush went off must still land"
    );
    assert!(app.workspace.scenes[0].document.has_unsaved_mesh_edits());
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
    assert!(
        !app.active_context()
            .expect("live test scene")
            .sculpt_has_live_work(),
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
        sculpt_shadow_len(&app) > committed_vertices,
        "the released stroke has a densified live preview"
    );

    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        app.active_context()
            .expect("live test scene")
            .commit_sculpt_stroke(&ctx),
        "the drag is released"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_none(),
        "no drag is held any more, so only the worker remembers it"
    );
    assert!(
        app.workspace.scenes[0]
            .tools
            .sculpt
            .worker_has_pending_work(),
        "but the worker still owes the released stroke's completion"
    );

    app.active_context()
        .expect("live test scene")
        .abort_sculpt_stroke();

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "an abort must revert the work waiting in the worker, not just the drag"
    );
    assert_eq!(after.triangle_count(), committed_triangles);
    assert_eq!(after.topology_id(), committed_topology);
    assert!(
        !app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
        "the reverted stroke is not work the operator has to save"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
    assert!(
        app.workspace.scenes[0].tools.sculpt.worker.is_none(),
        "the session is dropped"
    );
}

/// Ray admission reserves the final queue slot for the stroke boundary. Even
/// with 63 older commands queued, the active stroke's Finish must be admitted.
#[test]
fn an_active_stroke_keeps_its_finish_boundary_at_queue_capacity() {
    let (mut app, _layer_id) = app_with_a_live_stroke("sculpt-finish-reserved-slot");
    {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        worker.set_queue_paused_for_tests(true);
        let step = BrushRayStep {
            origin: [1000.0, 1000.0, 10.0],
            direction: [0.0, 0.0, -1.0],
            near_mm: 0.0,
            far_mm: 20.0,
            clip_plane: None,
            radius_mm: 0.75,
            strength: 0.35,
            mode: BrushMode::Add,
            tip: SculptTip::Ball,
            axis: None,
            hold: false,
            preserve_skirt: false,
        };
        // Each completed ray stroke contributes two commands. At 62 queued,
        // the next ray is command 63 and its reserved Finish is command 64.
        for _ in 0..31 {
            assert!(
                worker.try_apply_ray_step(step.clone()),
                "pressure ray queued"
            );
            assert!(worker.finish_stroke(), "pressure stroke boundary queued");
        }
        assert!(
            worker.try_apply_ray_step(step),
            "the last ray slot is admitted"
        );
    }

    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        app.active_context()
            .expect("live test scene")
            .commit_sculpt_stroke(&ctx),
        "the reserved slot admits Finish after the queue reaches 63 commands"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_none(),
        "the UI closes the released stroke after queuing Finish"
    );
    assert!(!app.workspace.scenes[0].tools.sculpt.finish_retry);

    app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .set_queue_paused_for_tests(false);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        if app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .is_some_and(SculptWorker::is_quiescent)
        {
            app.active_context()
                .expect("live test scene")
                .poll_sculpt_worker(&ctx);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the queued boundaries never drained"
        );
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "the queued boundaries never drained"
        );
    }
    assert!(
        app.ui.app_error.is_none(),
        "draining the reserved Finish must not fault the worker"
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
        sculpt_shadow_len(&app) > committed_vertices,
        "the stroke has a densified live preview"
    );

    app.workspace.scenes[0].tools.sculpt.worker = None;
    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        !app.active_context()
            .expect("live test scene")
            .commit_sculpt_stroke(&ctx),
        "a stroke cannot finish without its worker"
    );

    let after = layer_mesh(&app, layer_id);
    assert_eq!(
        after.vertices().len(),
        committed_vertices,
        "losing the worker must revert the live shadow, not freeze it on screen"
    );
    assert_eq!(after.topology_id(), committed_topology);
    assert!(!app.workspace.scenes[0].document.has_unsaved_mesh_edits());
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
    assert_eq!(
        app.workspace.scenes[0]
            .presentation
            .status_message
            .as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("sculpt-worker-unavailable"))
                .as_str()
        ),
        "the operator is told why the stroke could not finish"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_none(),
        "the dead drag is not left latched"
    );
}

/// A frame commits each finished topology in order while keeping the next
/// open stroke's local patches in the live display.
#[test]
fn a_finished_stroke_commits_before_a_later_live_topology_delta() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-topology-chain");
    let base_len = sculpt_shadow_len(&app);

    // Stroke 1 densifies and is released.
    {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
        assert!(worker.finish_stroke());
    }
    let after_first = wait_for_shadow_growth(&app, base_len);

    // Stroke 2 densifies but remains open.
    {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
    }
    let after_second = wait_for_shadow_growth(&app, after_first);

    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        if app.workspace.scenes[0].document.edit_mode.undo_len() == 1
            && layer_mesh(&app, layer_id).vertices().len() == after_first
        {
            break;
        }
        assert!(Instant::now() < deadline, "stroke 1 was never committed");
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "stroke 1 was never committed"
        );
    }

    assert!(app.ui.app_error.is_none(), "no terminal failure");
    assert!(
        app.workspace.scenes[0].tools.sculpt.worker.is_some(),
        "committing stroke 1 must not invalidate the session it belongs to"
    );
    assert!(
        app.workspace.scenes[0].document.has_unsaved_mesh_edits(),
        "stroke 1 is work the operator has to save"
    );
    assert_eq!(
        layer_mesh(&app, layer_id).topology_id(),
        app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker")
            .topology_id,
        "the committed mesh and worker agree while the second stroke stays open"
    );
    assert_eq!(sculpt_shadow_len(&app), after_second);
}

/// Multiple finished strokes commit in sequence while the latest open stroke
/// remains visible only in the live display.
#[test]
fn multiple_completions_commit_before_the_latest_live_delta() {
    let (mut app, layer_id) = app_with_a_live_stroke("sculpt-completion-chain");
    let mut shadow_len = sculpt_shadow_len(&app);

    // Two released densifying strokes produce two ordered completions.
    for _ in 0..2 {
        {
            let worker = app.workspace.scenes[0]
                .tools
                .sculpt
                .worker
                .as_ref()
                .expect("worker");
            assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
            assert!(worker.finish_stroke());
        }
        shadow_len = wait_for_shadow_growth(&app, shadow_len);
    }
    let committed_vertices = shadow_len;
    // A third, still-open stroke remains in the live display.
    {
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(worker.try_apply(densifying_stroke(), BrushMode::Smooth));
    }
    let final_len = wait_for_shadow_growth(&app, shadow_len);

    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.active_context()
            .expect("live test scene")
            .poll_sculpt_worker(&ctx);
        if app.workspace.scenes[0].document.edit_mode.undo_len() == 2
            && layer_mesh(&app, layer_id).vertices().len() == committed_vertices
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "both completions never committed"
        );
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            wait_for_worker_idle(worker, deadline),
            "both completions never committed"
        );
    }

    assert!(app.ui.app_error.is_none(), "no terminal failure");
    assert!(
        app.workspace.scenes[0].tools.sculpt.worker.is_some(),
        "the session must survive both commits"
    );
    assert!(app.workspace.scenes[0].document.has_unsaved_mesh_edits());
    assert_eq!(sculpt_shadow_len(&app), final_len);
}

#[test]
fn entering_sculpt_restores_the_remembered_tool_without_auto_arming_at_startup() {
    let (mut app, _) = app_with_a_live_stroke("sculpt-remembered-tool");
    app.workspace.scenes[0].tools.sculpt.stroke = None;
    app.workspace.scenes[0].tools.sculpt.armed = None;
    app.persistence.settings.last_sculpt_tool = SculptToolKind::Smooth;

    assert!(app.workspace.scenes[0].tools.sculpt.armed.is_none());
    let ctx = app.ui.repaint_ctx.clone();
    app.active_context()
        .expect("live test scene")
        .switch_editor_tab(mesh_editor_overlay::EditorTab::Sculpt, &ctx);

    assert_eq!(
        app.workspace.scenes[0].tools.sculpt.armed,
        Some(SculptToolKind::Smooth)
    );
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "The queued Finish must preserve the exact parameter, and an idle notch selects an exact catalog value."
)]
fn modified_wheel_waits_for_queued_finish_then_works_when_worker_is_idle() {
    let (mut app, _) = app_with_a_live_stroke("sculpt-wheel-finish-boundary");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let worker = app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("prepared worker");
    worker.set_queue_paused_for_tests(true);
    assert!(worker.try_apply_ray_step(BrushRayStep {
        origin: [0.0, 0.0, 20.0],
        direction: [0.0, 0.0, -1.0],
        near_mm: 0.0,
        far_mm: 100.0,
        clip_plane: None,
        radius_mm: 2.0,
        strength: 0.35,
        mode: BrushMode::Add,
        tip: SculptTip::Ball,
        axis: None,
        hold: false,
        preserve_skirt: false,
    }));

    let ctx = app.ui.repaint_ctx.clone();
    assert!(
        app.active_context()
            .expect("live test scene")
            .commit_sculpt_stroke(&ctx),
        "Finish is admitted behind the ray"
    );
    assert!(app.workspace.scenes[0].tools.sculpt.stroke.is_none());
    assert!(app.workspace.scenes[0]
        .tools
        .sculpt
        .worker_has_pending_work());
    assert!(
        sculpt_strength_wheel_frame(&mut app, &ctx, 50.0),
        "modified wheel remains owned while the released stroke drains"
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        SculptToolKind::AddRemove.default_strength(),
        "pending Finish must not change captured brush parameters"
    );

    app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .set_queue_paused_for_tests(false);
    pump_sculpt_worker_until_idle(&mut app);
    assert!(!app.workspace.scenes[0].tools.sculpt.is_busy());

    assert!(sculpt_strength_wheel_frame(&mut app, &ctx, 50.0));
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "the next idle notch changes the selected catalog strength"
    );
}
