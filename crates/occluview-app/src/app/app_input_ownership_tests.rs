//! Input ownership for the sculpt brush and the align tool: which keyboard and
//! pointer gestures reach a tool, and what happens to work already in flight
//! when another tool takes over.
//!
//! A `#[cfg(test)]` child module of `app_sculpt.rs` and `app_align.rs`, so it
//! drives the real hotkey and drag entry points rather than copies of them.
#![allow(
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::unwrap_used
)]

use super::app_test_support::{push_named_layer, test_app};
use super::*;
use crate::mesh_editor_overlay::EditorTab;
use crate::sculpt_tool::SculptToolKind;
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
use std::sync::Arc;

fn quad(x: f32) -> Mesh {
    Mesh::new(
        Some("quad".to_string()),
        vec![
            Vertex::at(glam::Vec3::new(x, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x + 1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x, 1.0, 0.0)),
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
    app.document.scene = Some(Arc::new(scene));
    let scene = app.document.scene.clone().expect("scene");
    let entry = &scene.meshes()[0];
    assert!(
        app.document
            .edit_mode
            .begin_face_selection(entry, scene.as_ref()),
        "the fixture must open an edit session"
    );
    app.tools.editor_tab = EditorTab::EditMesh;
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
        consumed = app.handle_sculpt_hotkeys(ui.ctx());
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
            app.tools.sculpt.armed,
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
    assert_eq!(app.tools.editor_tab, EditorTab::EditMesh);
    assert!(app.tools.sculpt.armed.is_none());

    assert!(
        press_key(&mut app, egui::Key::Num1, egui::Modifiers::NONE),
        "the digit must be consumed from the Edit Mesh tab"
    );
    assert_eq!(
        app.tools.sculpt.armed,
        Some(SculptToolKind::AddRemove),
        "the hotkey arms the brush"
    );
    assert_eq!(
        app.tools.editor_tab,
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
    app.document.scene = Some(Arc::new(scene));

    // Arm every tool that competes for the primary click first.
    app.tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    app.tools
        .measure
        .arm(crate::measure_tool::MeasureMode::Ruler);
    app.tools.cut_view.enable();
    assert!(app.tools.measure.is_active(), "measure is armed to start");
    assert!(app.tools.cut_view.is_active(), "cut is armed to start");

    let ctx = egui::Context::default();
    app.arm_align_tool(&ctx);

    assert!(app.tools.align.tool.is_armed(), "align takes the pointer");
    assert!(
        app.tools.sculpt.armed.is_none(),
        "the sculpt brush is stood down"
    );
    assert!(
        !app.tools.measure.is_active(),
        "the measure tool is stood down"
    );
    assert!(
        !app.tools.cut_view.is_active(),
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
    app.tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let scene = app.document.scene.clone().expect("scene");
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
    app.tools.sculpt.stroke = Some(crate::sculpt_tool::StrokeState {
        layer_id,
        last_dab_local: None,
        hold_seconds: 0.0,
    });
    app.document.unsaved_sculpt_stroke = true;

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
        owned = Some(app.handle_sculpt_drag(ui.ctx(), &response, false));
    })
    .drop_without_applying_deltas();

    assert_eq!(
        owned,
        Some(true),
        "the held gesture still belongs to the tool while the pointer is out"
    );
    assert!(
        app.tools.sculpt.stroke.is_some(),
        "leaving the viewport must pause the drag, not end it"
    );
    let worker_pending =
        app.tools.sculpt.worker.as_ref().is_some_and(|worker| {
            worker.has_pending_sparse_update() || worker.has_pending_rebuild()
        });
    assert!(
        !worker_pending,
        "no dab may be committed while the pointer is outside the viewport"
    );
}

/// The sculpt cursor is only drawn once the surface can answer a ray. A cold
/// mesh has no warmed BVH and no prepared worker, so a held brush over it must
/// paint the "preparing" message and lay no dab rather than pick against a tree
/// that does not exist yet.
#[test]
fn sculpt_cursor_waits_for_a_warm_pick_before_sampling() {
    let mut app = app_with_an_edit_session("sculpt-cursor-readiness");
    app.tools.sculpt.armed = Some(SculptToolKind::AddRemove);
    let scene = app.document.scene.clone().expect("scene");
    assert!(
        !scene.meshes()[0].mesh.bvh_is_ready(),
        "the fixture must start cold, or there is nothing to prove"
    );
    assert!(
        app.tools.sculpt.worker.is_none(),
        "and no prepared worker may exist yet"
    );

    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
    // Hover is computed from the previous frame, so establish the pointer over
    // the viewport once before the press frame.
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            events: vec![egui::Event::PointerMoved(screen.center())],
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
            pos: screen.center(),
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
        let _ = app.handle_sculpt_drag(ui.ctx(), &response, false);
    })
    .drop_without_applying_deltas();

    assert!(
        app.tools.sculpt.stroke.is_none(),
        "a cold surface must not start a stroke it cannot sample"
    );
    assert_eq!(
        app.ui.status_message,
        Some(app.ui.locale.tr("sculpt-preparing")),
        "the operator is told the brush is still preparing rather than getting a dead press"
    );
}

/// The erase direction is the brush's own toggle XOR the shift key. Driving the
/// real stroke path must route that decision into the dab's brush mode, or a
/// shift held mid-stroke would keep adding where the operator meant to subtract.
#[test]
fn a_stroke_takes_its_direction_from_the_toggle_and_shift_together() {
    use crate::align_brush::AlignBrush;

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
