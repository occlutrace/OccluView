//! Input ownership for the sculpt brush and the align tool: which keyboard and
//! pointer gestures reach a tool, and what happens to work already in flight
//! when another tool takes over.
//!
//! A `#[cfg(test)]` sibling of `sculpt` and `align`, so it drives the real
//! hotkey and drag entry points rather than copies of them.
#![allow(
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::unwrap_used
)]

use super::app_test_support::{push_named_layer, test_app};
use super::*;
use crate::mesh_editor::mesh_editor_overlay::EditorTab;
use crate::sculpt::sculpt_kernel::{BrushMode, BrushRayStep, BrushSession};
use crate::sculpt::sculpt_tool::{SculptSession, SculptTip, SculptToolKind, StrokeState};
use crate::sculpt::sculpt_worker::{SculptWorker, SculptWorkerInput};
use glam::{Affine3A, Quat, Vec3};
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
use occluview_mesh_edit::mesh_edit_buffers_from_mesh;
use occluview_render::PreparedSceneTopology;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

fn quad(x: f32) -> Mesh {
    Mesh::new(
        Some("quad".to_string()),
        vec![
            Vertex::at(Vec3::new(x, 0.0, 0.0)),
            Vertex::at(Vec3::new(x + 1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(x, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("a triangle is a mesh")
}

/// A live app with an active edit session, which is the gate the hotkeys check.
fn app_with_an_edit_session(name: &str) -> OccluViewApp {
    let mut app = test_app(name);
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(quad(0.0)));
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let entry = &scene.meshes()[0];
    assert!(
        app.workspace.scenes[0]
            .document
            .edit_mode
            .begin_face_selection(entry, scene.as_ref()),
        "the fixture must open an edit session"
    );
    app.workspace.scenes[0].tools.editor_tab = EditorTab::EditMesh;
    app
}

fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Run one frame carrying a single key event and report whether the sculpt
/// hotkey handler consumed it.
fn press_key(app: &mut OccluViewApp, key: egui::Key, modifiers: egui::Modifiers) -> bool {
    let ctx = egui::Context::default();
    let mut consumed = false;
    let raw = egui::RawInput {
        events: vec![key_event(key, modifiers)],
        ..Default::default()
    };
    ctx.run_ui(raw, |ui| {
        consumed = app
            .active_context()
            .expect("live test scene")
            .handle_sculpt_hotkeys(ui.ctx());
    })
    .drop_without_applying_deltas();
    consumed
}

/// Shift+1 and Shift+2 are the same brush switch as the bare digits. A held
/// shift is a modifier on the gesture, not a different shortcut, and letting it
/// change the meaning would make the mode switch silently do nothing for an
/// operator resting a finger on shift.
#[test]
fn brush_hotkeys_survive_a_held_shift() {
    for (key, expected) in [
        (egui::Key::Num1, SculptToolKind::AddRemove),
        (egui::Key::Num2, SculptToolKind::Smooth),
    ] {
        let mut app = app_with_an_edit_session("sculpt-hotkey-shift");

        assert!(
            press_key(&mut app, key, egui::Modifiers::SHIFT),
            "{key:?} with shift must be consumed as the brush hotkey"
        );
        assert_eq!(
            app.workspace.scenes[0].tools.sculpt.armed,
            Some(expected),
            "{key:?} with shift must arm the same brush as the bare digit"
        );
    }
}

/// The hotkey has to work from the Edit Mesh tab: it is the shortcut into the
/// Sculpt tab, so requiring the operator to already be there would make it
/// dead. The tab has to follow the arm.
#[test]
fn sculpt_hotkeys_switch_to_sculpt_from_edit_mesh() {
    let mut app = app_with_an_edit_session("sculpt-hotkey-tab");
    assert_eq!(
        app.workspace.scenes[0].tools.editor_tab,
        EditorTab::EditMesh
    );
    assert!(app.workspace.scenes[0].tools.sculpt.armed.is_none());

    assert!(
        press_key(&mut app, egui::Key::Num1, egui::Modifiers::NONE),
        "the digit must be consumed from the Edit Mesh tab"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.sculpt.armed,
        Some(SculptToolKind::AddRemove),
        "the hotkey arms the brush"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.editor_tab,
        EditorTab::Sculpt,
        "and the tab follows the brush, or the armed tool has no panel"
    );
}

/// Arming align must stand the other tools down. Two tools sharing the primary
/// click would otherwise fight over every gesture, and a cut or measure gesture
/// left active would keep its own on-screen handles after the align tool owns
/// the pointer.
#[test]
fn arming_align_stands_the_other_tools_down() {
    let mut app = test_app("align-arms-alone");
    let mut scene = Scene::new();
    push_named_layer(&mut scene, "lower", 0.0);
    push_named_layer(&mut scene, "upper", 5.0);
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));

    // Arm every tool that competes for the primary click first.
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0]
        .tools
        .measure
        .arm(crate::measure::measure_tool::MeasureMode::Ruler);
    app.workspace.scenes[0].tools.cut_view.enable();
    assert!(
        app.workspace.scenes[0].tools.measure.is_active(),
        "measure is armed to start"
    );
    assert!(
        app.workspace.scenes[0].tools.cut_view.is_active(),
        "cut is armed to start"
    );

    let ctx = egui::Context::default();
    app.active_context()
        .expect("live test scene")
        .arm_align_tool(&ctx);

    assert!(
        app.workspace.scenes[0].tools.align.tool.is_armed(),
        "align takes the pointer"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.armed.is_none(),
        "the sculpt brush is stood down"
    );
    assert!(
        !app.workspace.scenes[0].tools.measure.is_active(),
        "the measure tool is stood down"
    );
    assert!(
        !app.workspace.scenes[0].tools.cut_view.is_active(),
        "the cut tool is stood down"
    );
}

/// A drag paused because the pointer left the viewport keeps the stroke, and
/// samples nothing while it is outside. Ending the stroke instead would discard
/// the dabs already laid, and continuing to sample would paint through whatever
/// the pointer is over.
#[test]
fn an_active_stroke_stops_sampling_when_pointer_leaves_viewport() {
    let mut app = app_with_an_edit_session("sculpt-pointer-left");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let layer_id = scene.meshes()[0].id();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));

    // Frame one presses the primary button with the pointer inside the
    // viewport. No stroke is live yet, so the press edge starts nothing.
    let press = egui::RawInput {
        screen_rect: Some(screen),
        events: vec![
            egui::Event::PointerMoved(screen.center()),
            egui::Event::PointerButton {
                pos: screen.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..Default::default()
    };
    ctx.run_ui(press, |ui| {
        ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag());
    })
    .drop_without_applying_deltas();

    // The drag is already live; the pointer now moves outside the viewport
    // while the button stays held.
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

    let outside = egui::RawInput {
        screen_rect: Some(screen),
        events: vec![egui::Event::PointerMoved(egui::pos2(900.0, 900.0))],
        ..Default::default()
    };
    let mut owned = None;
    ctx.run_ui(outside, |ui| {
        // A viewport occupying only the top-left corner, so a pointer at the
        // far corner is outside it.
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(80.0, 80.0));
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        assert!(
            !response.contains_pointer(),
            "the fixture must put the pointer outside the viewport"
        );
        assert!(
            ui.ctx()
                .input(|input| input.pointer.button_down(egui::PointerButton::Primary)),
            "the primary button must still be held, or this is a release frame"
        );
        owned = Some(
            app.active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false),
        );
    })
    .drop_without_applying_deltas();

    assert_eq!(
        owned,
        Some(true),
        "the held gesture still belongs to the tool while the pointer is out"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_some(),
        "leaving the viewport must pause the drag, not end it"
    );
    let worker_pending = app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .is_some_and(|worker| {
            worker.has_pending_sparse_update() || worker.has_pending_topology_delta()
        });
    assert!(
        !worker_pending,
        "no dab may be committed while the pointer is outside the viewport"
    );
}

#[test]
fn a_press_on_the_layers_overlay_does_not_start_sculpting() {
    let mut app = app_with_an_edit_session("sculpt-layers-overlay-ownership");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0].render.camera = Some(Camera::default());
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let panel_click = layers_overlay::layer_overlay_rect(viewport, 1).center();
    let input = egui::RawInput {
        screen_rect: Some(viewport),
        events: vec![egui::Event::PointerButton {
            pos: panel_click,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };

    ctx.run_ui(input, |ui| {
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        let _ = app
            .active_context()
            .expect("live test scene")
            .handle_sculpt_drag(ui.ctx(), &response, false);
    })
    .drop_without_applying_deltas();

    assert!(app.workspace.scenes[0]
        .tools
        .sculpt
        .pending_presses
        .is_empty());
    assert!(app.workspace.scenes[0].tools.sculpt.stroke.is_none());
}

#[test]
fn a_same_frame_press_move_outside_and_release_retains_the_press_ray() {
    let mut app = app_with_an_edit_session("sculpt-press-release-same-frame");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0].render.camera = Some(Camera::default());
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let press = egui::pos2(350.0, 200.0);
    let outside = egui::pos2(450.0, 450.0);
    let input = egui::RawInput {
        screen_rect: Some(viewport),
        events: vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerMoved(outside),
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..Default::default()
    };

    ctx.run_ui(input, |ui| {
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        let _ = app
            .active_context()
            .expect("live test scene")
            .handle_sculpt_drag(ui.ctx(), &response, false);
    })
    .drop_without_applying_deltas();

    let pending = app.workspace.scenes[0]
        .tools
        .sculpt
        .pending_presses
        .front()
        .expect("the raw press must survive a same-frame release");
    assert!(pending.released);
    assert_eq!(pending.press_pointer, [press.x, press.y]);
    assert_eq!(pending.latest_pointer, [press.x, press.y]);
    assert!(pending.break_before_latest);
}

#[test]
fn a_pending_drag_marks_overlay_gap_before_its_latest_ray() {
    let mut app = app_with_an_edit_session("sculpt-pending-path-break");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0].tools.sculpt.pending_history = Some(false);
    app.workspace.scenes[0].render.camera = Some(Camera::default());
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let press = egui::pos2(150.0, 150.0);
    let reentry = egui::pos2(165.0, 160.0);

    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![
                egui::Event::PointerMoved(press),
                egui::Event::PointerButton {
                    pos: press,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            assert!(app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false));
        },
    )
    .drop_without_applying_deltas();

    let start_ray = app.workspace.scenes[0]
        .tools
        .sculpt
        .pending_presses
        .front()
        .expect("the press waits behind pending history")
        .start_step
        .clone();

    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![egui::Event::PointerMoved(egui::pos2(600.0, 600.0))],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            assert!(ui
                .ctx()
                .input(|input| input.pointer.button_down(egui::PointerButton::Primary)));
            assert!(app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false));
        },
    )
    .drop_without_applying_deltas();

    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![egui::Event::PointerMoved(reentry)],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            assert!(app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false));
        },
    )
    .drop_without_applying_deltas();

    let pending = app.workspace.scenes[0]
        .tools
        .sculpt
        .pending_presses
        .front()
        .expect("the pending gesture remains queued");
    assert!(pending.break_before_latest);
    assert!(pending.moved);
    assert_eq!(pending.press_pointer, [press.x, press.y]);
    assert_eq!(pending.latest_pointer, [reentry.x, reentry.y]);
    assert_eq!(pending.start_step, start_ray, "the first ray stays frozen");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "The event batch and resulting worker samples are asserted in one scenario."
)]
fn active_move_uses_its_modifier_state_even_when_release_clears_shift_same_frame() {
    let mut app = app_with_an_edit_session("sculpt-active-modifier-order");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::Smooth);
    app.workspace.scenes[0].render.camera = Some(Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        orientation: Some(Quat::IDENTITY),
        orthographic_height: 2.0,
        near: 0.1,
        far: 200.0,
        ..Default::default()
    });
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let entry = &scene.meshes()[0];
    let layer_id = entry.id();
    let mesh = Arc::clone(&entry.mesh);
    mesh.warm_bvh();
    let session = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    app.workspace.scenes[0].tools.sculpt.worker = Some(SculptWorker::spawn(SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session,
        base_mesh: Arc::clone(&mesh),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    }));
    drop(scene);

    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let start = egui::pos2(200.0, 200.0);
    let moved = egui::pos2(210.0, 190.0);
    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        },
    )
    .drop_without_applying_deltas();
    app.workspace.scenes[0].tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_pointer: [start.x, start.y],
        input_pointer: [start.x, start.y],
        last_ray: None,
        hold_seconds: 0.0,
        path_break_pending: false,
        release_pending: false,
        retained_samples: VecDeque::new(),
    });
    app.workspace.scenes[0].document.unsaved_sculpt_stroke = true;

    let mut sampled_strength = None;
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![
                egui::Event::ModifiersChanged(shift),
                egui::Event::PointerMoved(moved),
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            assert!(app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false));
            sampled_strength = app.workspace.scenes[0]
                .tools
                .sculpt
                .stroke
                .as_ref()
                .and_then(|stroke| stroke.last_ray.as_ref())
                .map(|step| step.strength);
        },
    )
    .drop_without_applying_deltas();

    assert_eq!(
        sampled_strength,
        Some(SculptToolKind::Smooth.default_strength() * 2.0),
        "the accepted worker ray uses Shift from the move, even though the frame ends unshifted"
    );

    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![egui::Event::PointerButton {
                pos: moved,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            let _ = app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false);
        },
    )
    .drop_without_applying_deltas();
    assert!(app.workspace.scenes[0].tools.sculpt.stroke.is_none());

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
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            worker.wait_until_idle(deadline.saturating_duration_since(Instant::now())),
            "the worker did not finish the ray"
        );
    }
    assert!(app.ui.app_error.is_none(), "the worker accepted the ray");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "One saturated worker scenario covers captured modes, a panel gap, release and ordered Finish."
)]
fn saturated_active_input_retains_modifier_samples_across_a_path_break_before_finish() {
    let mut app = app_with_an_edit_session("sculpt-retained-modifier-boundary");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0].render.camera = Some(Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        orientation: Some(Quat::IDENTITY),
        orthographic_height: 2.0,
        near: 0.1,
        far: 200.0,
        ..Default::default()
    });
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    let entry = &scene.meshes()[0];
    let layer_id = entry.id();
    let mesh = Arc::clone(&entry.mesh);
    mesh.warm_bvh();
    let session = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let worker = SculptWorker::spawn(SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session,
        base_mesh: Arc::clone(&mesh),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    });
    worker.set_queue_paused_for_tests(true);
    let make_step = |mode, x| BrushRayStep {
        origin: [x, 0.25, 10.0],
        direction: [0.0, 0.0, -1.0],
        near_mm: 0.0,
        far_mm: 20.0,
        clip_plane: None,
        radius_mm: 0.75,
        strength: 0.35,
        mode,
        tip: SculptTip::Ball,
        axis: None,
        hold: false,
        preserve_skirt: false,
    };
    let queued = [
        make_step(BrushMode::Add, 0.2),
        make_step(BrushMode::Smooth, 0.3),
        make_step(BrushMode::Relax, 0.4),
        make_step(BrushMode::Remove, 0.5),
    ];
    for step in &queued {
        assert!(worker.try_apply_ray_step(step.clone()));
    }
    app.workspace.scenes[0].tools.sculpt.worker = Some(worker);
    app.workspace.scenes[0].tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_pointer: [200.0, 200.0],
        input_pointer: [200.0, 200.0],
        last_ray: Some(queued[0].clone()),
        hold_seconds: 0.0,
        path_break_pending: false,
        release_pending: false,
        retained_samples: VecDeque::new(),
    });
    app.workspace.scenes[0].document.unsaved_sculpt_stroke = true;
    drop(scene);

    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let start = egui::pos2(200.0, 200.0);
    let remove_point = egui::pos2(208.0, 192.0);
    let outside = egui::pos2(450.0, 220.0);
    let add_point = egui::pos2(220.0, 180.0);
    let final_point = egui::pos2(228.0, 174.0);
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        },
    )
    .drop_without_applying_deltas();

    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    let control = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![
                egui::Event::ModifiersChanged(shift),
                egui::Event::PointerMoved(remove_point),
                egui::Event::PointerMoved(outside),
                egui::Event::ModifiersChanged(control),
                egui::Event::PointerMoved(add_point),
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(final_point),
                egui::Event::PointerButton {
                    pos: final_point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            assert!(app
                .active_context()
                .expect("live test scene")
                .handle_sculpt_drag(ui.ctx(), &response, false));
        },
    )
    .drop_without_applying_deltas();

    let stroke = app.workspace.scenes[0]
        .tools
        .sculpt
        .stroke
        .as_ref()
        .expect("release waits for drain");
    assert!(app.workspace.scenes[0].tools.sculpt.finish_retry);
    assert_eq!(stroke.retained_samples.len(), 3);
    assert_eq!(stroke.retained_samples[0].step.mode, BrushMode::Remove);
    assert!(!stroke.retained_samples[0].break_before);
    assert_eq!(stroke.retained_samples[1].step.mode, BrushMode::Add);
    assert!(stroke.retained_samples[1].step.preserve_skirt);
    assert!(stroke.retained_samples[1].break_before);
    assert_eq!(stroke.retained_samples[2].step.mode, BrushMode::Add);
    assert!(!stroke.retained_samples[2].step.preserve_skirt);
    assert!(!stroke.retained_samples[2].break_before);
    assert_eq!(
        stroke.retained_samples[0].pointer,
        [remove_point.x, remove_point.y]
    );
    assert_eq!(
        stroke.retained_samples[1].pointer,
        [add_point.x, add_point.y]
    );
    assert_eq!(
        stroke.retained_samples[2].pointer,
        [final_point.x, final_point.y]
    );

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
        let finished = app.workspace.scenes[0].tools.sculpt.stroke.is_none()
            && app.workspace.scenes[0]
                .tools
                .sculpt
                .worker
                .as_ref()
                .is_some_and(SculptWorker::is_quiescent);
        if finished {
            app.active_context()
                .expect("live test scene")
                .poll_sculpt_worker(&ctx);
            break;
        }
        let worker = app.workspace.scenes[0]
            .tools
            .sculpt
            .worker
            .as_ref()
            .expect("worker");
        assert!(
            worker.wait_until_idle(deadline.saturating_duration_since(Instant::now())),
            "retained samples did not finish"
        );
    }

    let trace = app.workspace.scenes[0]
        .tools
        .sculpt
        .worker
        .as_ref()
        .expect("worker")
        .take_input_trace_for_tests();
    let suffix: Vec<_> = trace.iter().rev().take(5).copied().collect();
    let suffix = suffix.into_iter().rev().collect::<Vec<_>>();
    assert!(
        matches!(
            suffix.as_slice(),
            [
                SculptWorkerInput::Ray {
                    mode: BrushMode::Remove,
                    preserve_skirt: false,
                    ..
                },
                SculptWorkerInput::BreakPath,
                SculptWorkerInput::Ray {
                    mode: BrushMode::Add,
                    preserve_skirt: true,
                    ..
                },
                SculptWorkerInput::Ray {
                    mode: BrushMode::Add,
                    preserve_skirt: false,
                    ..
                },
                SculptWorkerInput::Finish,
            ]
        ),
        "captured modifiers and the path boundary reach the worker in order: {suffix:?}"
    );
    assert!(app.ui.app_error.is_none());
}

#[test]
fn modified_wheel_does_not_change_brush_settings_during_a_stroke() {
    let mut app = app_with_an_edit_session("sculpt-held-wheel-settings");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let layer_id = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .id();
    app.workspace.scenes[0].tools.sculpt.stroke = Some(StrokeState {
        layer_id,
        last_pointer: [180.0, 180.0],
        input_pointer: [180.0, 180.0],
        last_ray: None,
        hold_seconds: 0.0,
        path_break_pending: false,
        release_pending: false,
        retained_samples: VecDeque::new(),
    });
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let center = egui::pos2(200.0, 200.0);
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
                delta: egui::vec2(0.0, 50.0),
                phase: egui::TouchPhase::Move,
                modifiers: ctrl,
            },
        ],
        ..Default::default()
    };
    let mut used = true;

    ctx.run_ui(input, |ui| {
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        used = app
            .active_context()
            .expect("live test scene")
            .adjust_sculpt_brush_from_wheel(ui.ctx(), &response);
    })
    .drop_without_applying_deltas();

    assert!(
        !used,
        "the setting wheel is only active while Sculpt is idle"
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        SculptToolKind::AddRemove.default_strength()
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            workspace::id::SceneKey::INITIAL,
            SculptTip::Ball
        ),
        SculptTip::Ball.default_radius_mm()
    );
}

/// A cold mesh has no warmed BVH or prepared worker. The press ray must stay
/// queued during background preparation, with no stroke started until ready.
#[test]
fn sculpt_press_keeps_its_ray_while_the_surface_session_prepares() {
    let mut app = app_with_an_edit_session("sculpt-cursor-readiness");
    app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.workspace.scenes[0].render.camera = Some(Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        orientation: Some(Quat::IDENTITY),
        orthographic_height: 2.0,
        near: 0.1,
        far: 200.0,
        ..Default::default()
    });
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .clone()
        .expect("scene");
    assert!(
        !scene.meshes()[0].mesh.bvh_is_ready(),
        "the fixture must start cold, or there is nothing to prove"
    );
    assert!(
        app.workspace.scenes[0].tools.sculpt.worker.is_none(),
        "and no prepared worker may exist yet"
    );

    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let press_point = egui::pos2(900.0, 400.0);
    // Hover is computed from the previous frame, so establish the pointer over
    // the viewport once before the press frame.
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            events: vec![egui::Event::PointerMoved(press_point)],
            ..Default::default()
        },
        |ui| {
            ui.allocate_rect(screen, egui::Sense::click_and_drag());
        },
    )
    .drop_without_applying_deltas();

    let press = egui::RawInput {
        screen_rect: Some(screen),
        events: vec![egui::Event::PointerButton {
            pos: press_point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    ctx.run_ui(press, |ui| {
        let response = ui.allocate_rect(screen, egui::Sense::click_and_drag());
        assert!(
            response.contains_pointer(),
            "the pointer is inside the viewport"
        );
        assert!(
            app.active_context()
                .expect("live test scene")
                .viewport_press_owned(ui.ctx(), &response, press_point),
            "the preparation fixture must press the scene outside overlay controls"
        );
        let _ = app
            .active_context()
            .expect("live test scene")
            .handle_sculpt_drag(ui.ctx(), &response, false);
    })
    .drop_without_applying_deltas();

    assert!(
        app.workspace.scenes[0].tools.sculpt.stroke.is_none(),
        "a cold surface must not start a stroke it cannot sample"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.sculpt.pending_presses.len(),
        1,
        "the valid pointer ray stays queued while the surface session prepares"
    );
    assert_eq!(
        app.workspace.scenes[0].presentation.status_message,
        Some(
            app.ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-preparing"))
        ),
        "the operator is told the brush is still preparing rather than getting a dead press"
    );
}

/// The erase direction is the brush's own toggle XOR the shift key. Driving the
/// real stroke path must route that decision into the dab's brush mode, or a
/// shift held mid-stroke would keep adding where the operator meant to subtract.
#[test]
fn a_stroke_takes_its_direction_from_the_toggle_and_shift_together() {
    use crate::align::align_brush::AlignBrush;

    for inverse in [false, true] {
        for shift in [false, true] {
            let mut brush = AlignBrush::default();
            brush.set_inverse(inverse);
            assert_eq!(
                brush.erases(shift),
                inverse != shift,
                "inverse={inverse} shift={shift}: the direction is the toggle XOR shift"
            );
        }
    }
}
