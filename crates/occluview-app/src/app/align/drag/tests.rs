#![allow(clippy::expect_used, clippy::panic)]

use crate::app::OccluViewApp;

use super::*;
use crate::app::app_test_support::{named_scene, test_app};
use glam::Quat;
use occluview_core::Camera;

/// A layer and a camera looking down at it, active enough to drag.
fn rig(name: &str) -> (OccluViewApp, SceneMeshId, Camera) {
    let mut app = test_app(name);
    app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(named_scene("jaw", 0.0)));
    let id = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .id();
    let camera = Camera::default().frame_occlusal(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .bbox(),
        45.0_f32.to_radians(),
    );
    app.workspace.scenes[0].render.camera = Some(camera);
    (app, id, camera)
}

fn frame(rotating: bool) -> DragFrame {
    DragFrame {
        viewport: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
        motion: egui::vec2(40.0, 0.0),
        rotating,
    }
}

fn actual_drag_fixture(
    name: &str,
) -> (
    OccluViewApp,
    SceneMeshId,
    egui::Context,
    egui::Rect,
    egui::Pos2,
    egui::Modifiers,
    egui::Id,
    Vec3,
) {
    use crate::align::align_panel::AlignTab;
    use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
    use std::sync::Arc;

    let mesh = Mesh::new(
        Some("jaw".to_string()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(40.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 40.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("a triangle is a mesh");
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    let id = scene.meshes()[0].id();
    let mut app = test_app(name);
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    let camera = Camera::default().frame_occlusal(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .bbox(),
        45.0_f32.to_radians(),
    );
    app.workspace.scenes[0].render.camera = Some(camera);
    app.workspace.scenes[0].tools.align.tool.arm();
    app.workspace.scenes[0].tools.align.tab = AlignTab::Manually;

    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let target = Vec3::new(30.0, 5.0, 0.0);
    let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target)
        .expect("a surface point must project into the viewport");
    let ctx = egui::Context::default();
    app.ui.repaint_ctx = ctx.clone();
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let viewport_id = egui::Id::new("align-raw-gesture-viewport");
    (app, id, ctx, rect, press_at, modifiers, viewport_id, target)
}

/// Bounds centre of the triangle `actual_drag_fixture` loads.
const FIXTURE_CENTRE: Vec3 = Vec3::new(20.0, 20.0, 0.0);

fn pointer_button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

fn drive_actual_drag_frame(
    app: &mut OccluViewApp,
    ctx: &egui::Context,
    rect: egui::Rect,
    viewport_id: egui::Id,
    events: Vec<egui::Event>,
) -> bool {
    let raw = egui::RawInput {
        screen_rect: Some(rect),
        events,
        ..Default::default()
    };
    let mut consumed = false;
    ctx.run_ui(raw, |ui| {
        let response = ui.interact(rect, viewport_id, egui::Sense::click_and_drag());
        let frame_ctx = ui.ctx().clone();
        consumed = app
            .active_context()
            .expect("live test scene")
            .handle_align_drag(&response, &frame_ctx);
    })
    .drop_without_applying_deltas();
    consumed
}

fn expected_free_translation(camera: &Camera, viewport: egui::Rect, motion: egui::Vec2) -> Vec3 {
    let right = camera
        .view_direction()
        .cross(camera.view_up())
        .normalize_or_zero();
    let world_per_pixel =
        crate::align::align_drag::mm_per_pixel(camera.orthographic_height, viewport.height());
    crate::align::align_drag::screen_delta_to_world(
        motion,
        right,
        camera.view_up(),
        world_per_pixel,
    )
}

/// A Ctrl-drag turns a transformed scan in place: its centre stays where it
/// is and the grabbed point goes with the cursor.
///
/// This exercises the transformed layer pose, camera-relative step, and
/// world-space centre together. The translation constraints must not alter
/// the Ctrl turn.
#[test]
fn a_ctrl_drag_step_turns_the_scan_about_its_centre_for_every_constraint() {
    let mut steps = Vec::new();
    for constraint in [
        crate::align::align_drag::DragConstraint::Free,
        crate::align::align_drag::DragConstraint::ZOnly,
        crate::align::align_drag::DragConstraint::XyPlane,
    ] {
        let (mut app, id, _) = rig("ctrl-step-centre");
        app.workspace.scenes[0].tools.align.constraint = constraint;
        let pose = Affine3A::from_translation(Vec3::new(30.0, -12.0, 7.0))
            * Affine3A::from_quat(Quat::from_rotation_y(0.45));
        let mut scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .as_ref()
            .clone();
        scene
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == id)
            .expect("layer")
            .transform = pose;
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));
        let mut camera = Camera::default().frame_occlusal(
            app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .bbox(),
            45.0_f32.to_radians(),
        );
        camera.orbit_view_by(0.35, -0.2);
        app.workspace.scenes[0].render.camera = Some(camera);

        // A valid off-centre point in the mesh's local frame.
        let grabbed_local = Vec3::new(0.75, 0.1, 0.0);
        let drag = AlignDrag {
            layer: id,
            start: pose,
            grab_local: grabbed_local,
        };
        let grabbed_world = pose.transform_point3(grabbed_local);
        let first_step = app
            .active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(true))
            .expect("a Ctrl-drag over a live layer must produce a step");
        let scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene");
        let entry = scene.meshes().iter().find(|e| e.id() == id).expect("layer");
        let centre_world = entry
            .transform
            .transform_point3(entry.mesh.bbox_cached().center());
        assert!(
            (first_step.transform_point3(centre_world) - centre_world).length() < 1e-3,
            "{constraint:?}: the scan must turn in place"
        );
        assert!(
            (first_step.transform_point3(grabbed_world) - grabbed_world).length() > 1e-3,
            "{constraint:?}: the grabbed point must go with the cursor"
        );
        steps.push(first_step);
    }
    // The translation chips are labelled for movement; none of them may
    // change a Ctrl turn.
    for step in &steps[1..] {
        assert_eq!(
            *step, steps[0],
            "a translation chip changed the Ctrl-drag turn"
        );
    }
}

/// On screen, a point of the scan's ball moves exactly as far as the cursor
/// went: the step is built from the live camera, the layer's centre and its
/// radius, and the pixel scale, and a slip in any of them would show here.
#[test]
fn a_ctrl_drag_step_keeps_the_ball_under_the_cursor() {
    let (mut app, id, mut camera) = rig("ctrl-step-follows-cursor");
    camera.orbit_view_by(0.35, -0.2);
    app.workspace.scenes[0].render.camera = Some(camera);
    let viewport = frame(true).viewport;
    let bounds = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .mesh
        .bbox_cached();
    let radius = bounds.size().length() * 0.5;
    // A point of the ball over the scan, on the side facing the viewer.
    let view = camera.view_direction();
    let right = view.cross(camera.view_up()).normalize();
    let offset = egui::vec2(0.2, -0.1) * radius;
    let grabbed = bounds.center() + right * offset.x + camera.view_up() * offset.y
        - view * (radius * radius - offset.length_sq()).sqrt();
    let drag = AlignDrag {
        layer: id,
        start: Affine3A::IDENTITY,
        grab_local: grabbed,
    };
    let project = |point: Vec3| {
        crate::viewer::project_world_to_viewport(&camera, viewport, point)
            .expect("the point projects into the viewport")
            .0
    };
    // Short enough that the per-step cap on the turn does not cut in.
    let motion = egui::vec2(1.2, -0.9);
    let step = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(
            drag,
            &camera,
            DragFrame {
                viewport,
                motion,
                rotating: true,
            },
        )
        .expect("a Ctrl-drag over a live layer must produce a step");
    let landed = project(step.transform_point3(grabbed));
    assert!(
        (landed - (project(grabbed) + motion)).length() < 0.05,
        "the cursor went {motion:?} and the point of the ball went {:?}",
        landed - project(grabbed)
    );
}

/// Switching modifiers during one held gesture keeps turning about the scan's
/// centre wherever the plain movement has carried it.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the complete gesture and its state assertions in one regression scenario."
)]
fn changing_between_translation_and_tilt_turns_about_the_carried_centre() {
    let (mut app, id, mut camera) = rig("mixed-manual-gesture");
    camera.orbit_view_by(0.25, -0.18);
    app.workspace.scenes[0].render.camera = Some(camera);
    let pose = Affine3A::from_translation(Vec3::new(4.0, -2.0, 6.0))
        * Affine3A::from_quat(Quat::from_rotation_x(0.3));
    let drag = AlignDrag {
        layer: id,
        start: pose,
        grab_local: Vec3::new(0.75, 0.1, 0.0),
    };
    let centre_local = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .mesh
        .bbox_cached()
        .center();
    app.workspace.scenes[0].tools.align.drag = Some(drag);
    let mut scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .as_ref()
        .clone();
    scene
        .meshes_mut()
        .iter_mut()
        .find(|entry| entry.id() == id)
        .expect("layer")
        .transform = pose;
    app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));

    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let centre_before = pose.transform_point3(centre_local);
    let translation = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(
            drag,
            &camera,
            DragFrame {
                viewport,
                motion: egui::vec2(12.0, -6.0),
                rotating: false,
            },
        )
        .expect("plain movement step");
    app.active_context()
        .expect("live test scene")
        .nudge_align_layer(id, translation);
    let translated_scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene");
    let translated = translated_scene
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("layer")
        .transform;
    let translated_centre = translated.transform_point3(centre_local);
    assert!(
        (translated_centre - centre_before - Vec3::from(translation.translation)).length() < 1e-3,
        "plain movement should carry the centre with the layer"
    );

    let rotation = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(drag, &camera, frame(true))
        .expect("Ctrl movement step");
    assert!(
        (rotation.transform_point3(translated_centre) - translated_centre).length() < 1e-3,
        "adding Ctrl should turn about the centre in its current pose"
    );
    app.active_context()
        .expect("live test scene")
        .nudge_align_layer(id, rotation);

    let after_rotation = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene");
    let turned = after_rotation
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("layer")
        .transform;
    let turned_centre = turned.transform_point3(centre_local);
    assert!((turned_centre - translated_centre).length() < 1e-3);

    let next_translation = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(
            drag,
            &camera,
            DragFrame {
                viewport,
                motion: egui::vec2(-5.0, 7.0),
                rotating: false,
            },
        )
        .expect("plain movement after Ctrl");
    app.active_context()
        .expect("live test scene")
        .nudge_align_layer(id, next_translation);
    let moved_again = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene");
    let moved_pose = moved_again
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("layer")
        .transform;
    let moved_centre = moved_pose.transform_point3(centre_local);
    assert!(
        (moved_centre - turned_centre - Vec3::from(next_translation.translation)).length() < 1e-3,
        "plain movement after Ctrl should carry the same centre"
    );

    let final_rotation = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(drag, &camera, frame(true))
        .expect("second Ctrl movement");
    assert!(
        (final_rotation.transform_point3(moved_centre) - moved_centre).length() < 1e-3,
        "a later Ctrl step should turn about the carried centre"
    );
}

/// A grab far outside the scan still turns it in place rather than freezing
/// the gesture or swinging it about a point at infinity.
#[test]
fn an_absurd_grab_still_produces_a_usable_step() {
    let (mut app, id, camera) = rig("absurd-grab");
    let drag = AlignDrag {
        layer: id,
        start: Affine3A::IDENTITY,
        grab_local: Vec3::splat(1.0e9),
    };
    let step = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(drag, &camera, frame(true))
        .expect("an absurd grab must still leave the gesture alive");
    assert!(step.is_finite(), "{step:?}");
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene");
    let centre = scene.meshes()[0].mesh.bbox_cached().center();
    let pinned = step.transform_point3(centre);
    assert!(
        (pinned - centre).length() < 1e-3,
        "the turn must leave the layer centre in place, but it moved {pinned:?} \
         away from {centre:?}"
    );
}

/// A layer that vanished mid-gesture refuses the step instead of panicking.
#[test]
fn a_step_for_a_missing_layer_produces_nothing() {
    let (mut app, id, camera) = rig("missing-layer");
    // Drop the scene out from under the open gesture.
    app.workspace.scenes[0].document.scene = None;
    let drag = AlignDrag {
        layer: id,
        start: Affine3A::IDENTITY,
        grab_local: Vec3::ZERO,
    };
    assert!(
        app.active_context()
            .expect("live test scene")
            .align_drag_step(drag, &camera, frame(true))
            .is_none(),
        "a step with no scene must refuse rather than guess"
    );
}

/// A non-rotation frame is a translation and must not touch the rotation.
#[test]
fn a_plain_drag_step_translates_instead_of_turning() {
    let (mut app, id, camera) = rig("plain-drag");
    let drag = AlignDrag {
        layer: id,
        start: Affine3A::IDENTITY,
        grab_local: Vec3::new(5.0, 5.0, 5.0),
    };
    let step = app
        .active_context()
        .expect("live test scene")
        .align_drag_step(drag, &camera, frame(false))
        .expect("a plain drag must produce a step");
    let rotation = Quat::from_mat3a(&step.matrix3);
    assert!(
        rotation.angle_between(Quat::IDENTITY) < 1e-4,
        "a plain drag must not rotate: {rotation:?}"
    );
}

/// The actual press ray must select the surface point the Ctrl turn carries
/// with the cursor, while the scan turns in place.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "One actual press-drag-release scenario keeps its UI setup and geometry assertions together."
)]
fn a_real_ctrl_drag_gesture_carries_the_pressed_point_about_the_centre() {
    use crate::align::align_panel::AlignTab;
    use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
    use std::sync::Arc;

    /// A wide triangle, so a click lands well off the mesh centre.
    fn wide_scene() -> (Scene, SceneMeshId) {
        let mesh = Mesh::new(
            Some("jaw".to_string()),
            vec![
                Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(40.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 40.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("a triangle is a mesh");
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        let id = scene.meshes()[0].id();
        (scene, id)
    }

    let mut app = test_app("real-ctrl-drag-gesture");
    let (scene, id) = wide_scene();
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    let bbox = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .bbox();
    let camera = Camera::default().frame_occlusal(bbox, 45.0_f32.to_radians());
    app.workspace.scenes[0].render.camera = Some(camera);
    app.workspace.scenes[0].tools.align.tool.arm();
    app.workspace.scenes[0].tools.align.tab = AlignTab::Manually;

    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    // A point on the surface, far from the centre at (20, 20, 0).
    let target = Vec3::new(30.0, 5.0, 0.0);
    let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target)
        .expect("a surface point must project into the viewport");
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let viewport_id = egui::Id::new("gesture-viewport");

    // The interaction state must persist across frames, so one context
    // drives every frame of the gesture.
    let ctx = egui::Context::default();
    let mut frame = |events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(rect),
            events,
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| {
            let response = ui.interact(rect, viewport_id, egui::Sense::click_and_drag());
            let frame_ctx = ui.ctx().clone();
            app.active_context()
                .expect("live test scene")
                .handle_align_drag(&response, &frame_ctx);
        })
        .drop_without_applying_deltas();
    };

    // Frame 0: register the viewport widget so egui can hit-test the press.
    frame(vec![]);
    // Frame 1: the coalesced frame. A fast flick, or a delayed egui pass,
    // delivers the primary press and the pointer's move in one batch: the
    // press lands on the surface, then the pointer is already 60 px away by
    // the end of the same frame. The frame's current pointer position is
    // therefore NOT where the button went down, and neither `hover_pos` nor
    // `interact_pointer_pos` can be used to place the grab.
    frame(vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::PointerMoved(press_at),
        egui::Event::PointerButton {
            pos: press_at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        },
        egui::Event::PointerMoved(press_at + egui::vec2(60.0, 20.0)),
    ]);
    // Frame 2: a secondary press lands elsewhere. egui keeps a single
    // `press_origin` for every button, so this is what would displace the
    // grab if it read that shared slot.
    frame(vec![
        egui::Event::PointerButton {
            pos: press_at + egui::vec2(-120.0, -60.0),
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers,
        },
        egui::Event::PointerMoved(press_at + egui::vec2(90.0, 40.0)),
    ]);
    // Frame 3: no button is being pressed now, so egui promotes the held
    // primary to a drag and the grab is resolved.
    frame(vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::PointerMoved(press_at + egui::vec2(100.0, 45.0)),
    ]);

    let Some(drag) = app.workspace.scenes[0].tools.align.drag else {
        panic!("a Ctrl-drag over the surface must open a drag");
    };
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene");
    let entry = scene.meshes().iter().find(|e| e.id() == id).expect("layer");
    let centre = entry.mesh.bbox_cached().center();

    // The grab must be the surface point under the cursor, not the centre.
    assert!(
        (drag.grab_local - target).length() < 0.5,
        "the grab stored {:?} but the operator pressed on {target:?}",
        drag.grab_local
    );

    // The scan turns in place, and the pressed point is what the turn moves.
    let centre_world = entry.transform.transform_point3(centre);
    assert!(
        (centre_world - drag.start.transform_point3(centre)).length() < 1e-2,
        "the layer centre moved from {:?} to {centre_world:?}",
        drag.start.transform_point3(centre)
    );
    let pressed_world = entry.transform.transform_point3(drag.grab_local);
    let pressed_before = drag.start.transform_point3(drag.grab_local);
    assert!(
        (pressed_world - pressed_before).length() > 0.1,
        "the pressed surface point stayed at {pressed_before:?}"
    );
}

/// A fast gesture can be delivered entirely in one egui frame. The raw
/// event stream must still open, move, and close one Ctrl turn of the point
/// under the press, with one history entry.
#[test]
fn a_coalesced_ctrl_press_move_release_turns_and_records_one_drag() {
    let (mut app, id, ctx, rect, press_at, modifiers, viewport_id, target) =
        actual_drag_fixture("coalesced-ctrl-drag");
    let moved_at = press_at + egui::vec2(60.0, 20.0);
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);

    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, modifiers),
            egui::Event::PointerMoved(moved_at),
            pointer_button(moved_at, false, modifiers),
        ],
    ));

    assert!(
        app.workspace.scenes[0].tools.align.drag.is_none(),
        "the release closes the drag"
    );
    let entry = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("jaw");
    assert!(
        (entry.transform.transform_point3(target) - target).length() > 0.1,
        "the pressed surface point must go with the cursor"
    );
    let centre = entry.mesh.bbox_cached().center();
    assert!(
        (entry.transform.transform_point3(centre) - centre).length() < 1e-2,
        "Ctrl rotation must turn the scan in place"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
}

/// The actual pointer path has to ray-pick in a transformed instance, not
/// just use a grab a unit test placed into `AlignDrag` itself.
#[test]
fn a_ctrl_press_ray_picks_and_turns_a_transformed_scan_in_place() {
    let (mut app, id, ctx, rect, _, modifiers, viewport_id, _) =
        actual_drag_fixture("transformed-ctrl-ray-pick");
    let pose = Affine3A::from_translation(Vec3::new(11.0, -7.0, 5.0))
        * Affine3A::from_quat(Quat::from_rotation_y(0.34) * Quat::from_rotation_x(-0.21));
    let mut scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .as_ref()
        .clone();
    scene
        .meshes_mut()
        .iter_mut()
        .find(|entry| entry.id() == id)
        .expect("jaw")
        .transform = pose;
    app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene));
    let camera = Camera::default().frame_occlusal(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .bbox(),
        45.0_f32.to_radians(),
    );
    app.workspace.scenes[0].render.camera = Some(camera);

    let target_local = Vec3::new(12.0, 12.0, 0.0);
    let target_world = pose.transform_point3(target_local);
    let (press_at, _) = crate::viewer::project_world_to_viewport(&camera, rect, target_world)
        .expect("the transformed surface point projects into the viewport");
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, modifiers),
        ],
    ));

    let drag = app.workspace.scenes[0]
        .tools
        .align
        .drag
        .expect("the press ray opens a drag");
    assert_eq!(drag.layer, id);
    assert!(
        (drag.grab_local - target_local).length() < 1e-2,
        "the press ray must recover the known local point: {:?} vs {:?}",
        drag.grab_local,
        target_local
    );
    let moved_at = press_at + egui::vec2(48.0, 26.0);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(moved_at),
            pointer_button(moved_at, false, modifiers),
        ],
    ));

    let moved = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("jaw");
    assert_ne!(moved.transform, pose, "the Ctrl gesture must turn the scan");
    let centre = moved.mesh.bbox_cached().center();
    assert!(
        (moved.transform.transform_point3(centre) - pose.transform_point3(centre)).length() < 1e-2,
        "the scan must turn about its own centre"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
}

/// Once a surface press owns a drag, moving and releasing outside the
/// viewport still belongs to the align gesture rather than to the camera.
#[test]
fn a_surface_drag_can_release_outside_the_viewport() {
    let (mut app, id, ctx, rect, press_at, _modifiers, viewport_id, _) =
        actual_drag_fixture("outside-release-drag");
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, egui::Modifiers::NONE),
        ],
    ));
    assert!(app.workspace.scenes[0].tools.align.drag.is_some());

    let outside = egui::pos2(900.0, 700.0);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(outside),
            pointer_button(outside, false, egui::Modifiers::NONE),
        ],
    ));
    assert!(app.workspace.scenes[0].tools.align.drag.is_none());
    assert_ne!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .transform,
        Affine3A::IDENTITY,
        "motion through release outside the viewport must be applied"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .id(),
        id
    );
}

/// Pressing and releasing without motion starts no edit, so an already
/// landed fit remains valid until the operator actually changes the pose.
#[test]
fn a_surface_click_without_motion_keeps_the_landed_fit() {
    let (mut app, _, ctx, rect, press_at, _modifiers, viewport_id, _) =
        actual_drag_fixture("align-grab-without-move");
    app.workspace.scenes[0].tools.align.accepted =
        Some(crate::align::align_state::AcceptedAlignment::test_authority());
    app.workspace.scenes[0].tools.align.settings.show_deviation = true;
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, egui::Modifiers::NONE),
        ],
    ));
    assert!(app.workspace.scenes[0].tools.align.accepted.is_some());
    assert!(app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![pointer_button(press_at, false, egui::Modifiers::NONE,)],
    ));
    assert!(app.workspace.scenes[0].tools.align.accepted.is_some());
    assert!(app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
}

/// The final `InputState::modifiers` value applies to the whole egui pass,
/// not to each event in order. Replaying a move before and after Ctrl changes
/// must therefore start from the modifier state held by the previous frame.
#[test]
fn modifier_changes_keep_raw_move_order_across_frames() {
    // Plain translation, then Ctrl rotation in one batch that ends with
    // Ctrl down. Its first move must still translate the scan, centre and all.
    let (mut app, _, ctx, rect, press_at, ctrl, viewport_id, _) =
        actual_drag_fixture("translate-before-ctrl-same-batch");
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, egui::Modifiers::NONE),
        ],
    ));
    let drag = app.workspace.scenes[0]
        .tools
        .align
        .drag
        .expect("surface press opens drag");
    let camera = app.workspace.scenes[0].render.camera.expect("camera");
    let plain_motion = egui::vec2(16.0, 8.0);
    let turn_motion = egui::vec2(48.0, -18.0);
    let after_plain = press_at + plain_motion;
    let after_turn = after_plain + turn_motion;
    let expected_centre = drag.start.transform_point3(FIXTURE_CENTRE)
        + expected_free_translation(&camera, rect, plain_motion);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(after_plain),
            egui::Event::ModifiersChanged(ctrl),
            egui::Event::PointerMoved(after_turn),
            pointer_button(after_turn, false, ctrl),
        ],
    ));
    let moved = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .transform;
    assert!(
        (moved.transform_point3(FIXTURE_CENTRE) - expected_centre).length() < 1e-2,
        "the first movement must translate before the later Ctrl turn"
    );
    assert_ne!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);

    // The reverse ordering starts with Ctrl down, then releases it before
    // the second move in a batch whose final modifiers are NONE.
    let (mut app, _, ctx, rect, press_at, ctrl, viewport_id, _) =
        actual_drag_fixture("ctrl-before-translation-same-batch");
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::ModifiersChanged(ctrl),
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, ctrl),
        ],
    ));
    let drag = app.workspace.scenes[0]
        .tools
        .align
        .drag
        .expect("surface press opens drag");
    let camera = app.workspace.scenes[0].render.camera.expect("camera");
    let turn_motion = egui::vec2(46.0, 22.0);
    let plain_motion = egui::vec2(-14.0, 11.0);
    let after_turn = press_at + turn_motion;
    let after_plain = after_turn + plain_motion;
    let expected_centre = drag.start.transform_point3(FIXTURE_CENTRE)
        + expected_free_translation(&camera, rect, plain_motion);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::PointerMoved(after_turn),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::PointerMoved(after_plain),
            pointer_button(after_plain, false, egui::Modifiers::NONE),
        ],
    ));
    let moved = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()[0]
        .transform;
    assert!(
        (moved.transform_point3(FIXTURE_CENTRE) - expected_centre).length() < 1e-2,
        "the first Ctrl movement must turn in place before later translation"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 1);
}

/// Cancel discards the open gesture before restoring the session snapshot.
/// The restoration is one undoable cancellation entry; undoing it recovers
/// the pose the active drag had reached.
#[test]
fn cancel_discards_an_open_ctrl_drag_and_records_only_the_restore() {
    let (mut app, id, ctx, rect, press_at, modifiers, viewport_id, _) =
        actual_drag_fixture("cancel-open-ctrl-drag");
    app.active_context()
        .expect("live test scene")
        .arm_align_tool(&ctx);
    app.workspace.scenes[0].tools.align.tab = crate::align::align_panel::AlignTab::Manually;
    drive_actual_drag_frame(&mut app, &ctx, rect, viewport_id, vec![]);
    let moved_at = press_at + egui::vec2(50.0, -16.0);
    assert!(drive_actual_drag_frame(
        &mut app,
        &ctx,
        rect,
        viewport_id,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(press_at),
            pointer_button(press_at, true, modifiers),
            egui::Event::PointerMoved(moved_at),
        ],
    ));
    assert!(app.workspace.scenes[0].tools.align.drag.is_some());
    let moved_pose = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()
        .iter()
        .find(|entry| entry.id() == id)
        .expect("jaw")
        .transform;
    assert_ne!(moved_pose, Affine3A::IDENTITY);
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);

    app.active_context()
        .expect("live test scene")
        .cancel_align_session(&ctx);
    assert!(app.workspace.scenes[0].tools.align.drag.is_none());
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[0]
            .transform,
        Affine3A::IDENTITY
    );
    assert_eq!(
        app.workspace.scenes[0].document.edit_mode.undo_len(),
        1,
        "Cancel should have one restore entry, with no drag commit before it"
    );

    app.active_context()
        .expect("live test scene")
        .apply_history_navigation_now(false, &ctx);
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .expect("jaw")
            .transform,
        moved_pose,
        "one undo should cancel the Cancel"
    );
    assert_eq!(app.workspace.scenes[0].document.edit_mode.undo_len(), 0);
}
