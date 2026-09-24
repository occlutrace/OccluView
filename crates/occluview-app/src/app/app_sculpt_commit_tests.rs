//! Tests for the sculpt commit boundary: what a committed stroke revokes, and
//! what a terminal worker failure has to reach the operator with.
//!
//! A `#[path]` child module of `app_sculpt.rs`, so it drives the same
//! `poll_sculpt_worker` the frame path uses.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used
)]

use super::*;
use crate::app::app_align_display::AlignOverlay;
use crate::app::app_test_support::test_app;
use crate::sculpt_tool::SculptSession;
use glam::{Affine3A, Vec3};
use occluview_core::{mesh_edit_buffers_from_mesh, BrushSession, Mesh, Scene, SceneMesh, Vertex};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// A 5x3 lattice at 4mm spacing folded along a sharp ridge: coarse enough that
/// the brush has somewhere to work, small enough to finish immediately.
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

fn a_stroke_that_moves_geometry() -> BrushStroke {
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

/// An app with one sculptable layer, an edit session over it, an armed brush
/// and a live worker — the state the frame path is in when it polls.
fn app_with_a_sculpt_worker(name: &str) -> (OccluViewApp, SceneMeshId) {
    let mut app = test_app(name);
    let mesh = coarse_ridge_mesh();
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
    let base = Arc::clone(&entry.mesh);
    drop(scene);

    app.tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.tools.sculpt.worker = Some(worker_for(&base, layer_id));
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

/// Poll the frame path until the worker owes the stroke nothing more, exactly
/// as the viewport does once per frame.
fn pump_sculpt_worker(app: &mut OccluViewApp) {
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

/// Poll the frame path until the operator has been shown a failure.
fn pump_until_failure_is_shown(app: &mut OccluViewApp) {
    let ctx = app.ui.repaint_ctx.clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.ui.app_error.is_none() {
        app.poll_sculpt_worker(&ctx);
        assert!(
            Instant::now() < deadline,
            "a terminal worker failure never reached the operator"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Every terminal failure exit has to leave the operator with the dialog and
/// the brush off. A failure that is only logged leaves the tool armed over a
/// session that cannot produce anything, and the next stroke silently does
/// nothing.
#[test]
fn every_terminal_failure_exit_raises_the_dialog_and_disarms() {
    // Exit one: the drain itself fails, before any output could be read.
    let (mut app, _layer_id) = app_with_a_sculpt_worker("sculpt-failure-drain-exit");
    app.tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .poison_publication_for_tests();
    pump_until_failure_is_shown(&mut app);
    assert_eq!(
        app.tools.sculpt.armed, None,
        "the brush must be off: the session it was armed over is gone"
    );
    assert!(
        app.tools.sculpt.worker.is_none(),
        "and the dead session must be dropped, not left to refuse every dab"
    );
    assert!(
        app.ui.status_message.is_some(),
        "the status line says so too"
    );
    let detail = app.ui.locale.text("sculpt-failure-worker-state-poisoned");
    assert!(
        app.ui
            .app_error
            .as_ref()
            .expect("the dialog")
            .summary
            .contains(&detail),
        "the dialog has to carry the typed reason: {:?}",
        app.ui
            .app_error
            .as_ref()
            .map(|dialog| dialog.summary.clone())
    );

    // Exit two: the worker reports its failure after the drain succeeded, which
    // is the ordinary path for a dab that hits a poisoned shadow.
    let (mut app, _layer_id) = app_with_a_sculpt_worker("sculpt-failure-late-exit");
    let shadow = app.tools.sculpt.worker.as_ref().expect("worker").shadow();
    let poison = std::thread::spawn(move || {
        let _guard = shadow.write().expect("shadow lock");
        // The panic IS the mechanism under test: it poisons the worker's
        // publication boundary, which is what the real code sees when a panic
        // unwinds through a worker-side lock.
        panic!("test poison");
    });
    assert!(poison.join().is_err());
    assert!(
        app.tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker")
            .try_apply(a_stroke_that_moves_geometry(), BrushMode::Add),
        "the dab that trips the failure is queued"
    );
    pump_until_failure_is_shown(&mut app);
    assert_eq!(app.tools.sculpt.armed, None);
    assert!(app.tools.sculpt.worker.is_none());
    let detail = app.ui.locale.text("sculpt-failure-shadow-poisoned");
    assert!(
        app.ui
            .app_error
            .as_ref()
            .expect("the dialog")
            .summary
            .contains(&detail),
        "the dialog has to carry the typed reason"
    );
}

/// A committed sculpt replaces the layer's surface in place, so it is the one
/// structural edit that does not pass through `set_scene`. The fit measured
/// against the surface that just disappeared must not stay authoritative: the
/// operator would read a percentage for geometry that no longer exists.
#[test]
fn a_sculpt_commit_revokes_the_alignment_measured_against_the_old_mesh() {
    let (mut app, layer_id) = app_with_a_sculpt_worker("sculpt-commit-revokes-align");
    app.tools.align.tool.arm();
    app.tools.align.tool.imply_pair(&[layer_id, layer_id]);
    app.tools.align.refined_match_ready = true;
    app.tools.align.settings.show_deviation = true;
    app.tools.align.rejected = vec![0, 1];
    let vertex_count = layer_mesh(&app, layer_id).vertices().len();
    assert!(
        app.apply_deviation_colors(vec![[30, 120, 200, 255]; vertex_count]),
        "the map of the old surface is up before the stroke"
    );
    assert_eq!(app.tools.align.overlay, AlignOverlay::Map);

    {
        let worker = app.tools.sculpt.worker.as_ref().expect("worker");
        assert!(worker.try_apply(a_stroke_that_moves_geometry(), BrushMode::Add));
        assert!(worker.finish_stroke());
    }
    pump_sculpt_worker(&mut app);

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "the stroke really committed, so the surface really changed"
    );
    assert!(
        !app.tools.align.refined_match_ready,
        "a fit measured against the replaced surface is not a refined match for it"
    );
    assert!(!app.tools.align.settings.show_deviation);
    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "the colours describe the surface the stroke replaced"
    );
    assert!(!app.align_overlay_is_up());
    assert!(
        app.tools.align.rejected.is_empty(),
        "the outlier marks index pairs of a fit that no longer describes this scan"
    );
    let reason = app.ui.locale.tr("align-status-scan-changed");
    assert_eq!(
        app.tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .tr_with("align-status-remeasure", &[("reason", &reason)])
                .as_str()
        ),
        "and the operator is told to measure again"
    );
}
